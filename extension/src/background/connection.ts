// The socket to the desktop: ws://127.0.0.1:<port>/ws/browser. Pairing,
// authentication (`hello`), a ping every 20 s (WebSocket traffic keeps an
// MV3 service worker alive since Chrome 116), and reconnecting with backoff
// for as long as the browser is paired.

import { backoffDelay } from '../shared/backoff.ts';
import { messageOf } from '../shared/errors.ts';
import {
  decodeIncoming,
  encodeFrame,
  type ErrorFrame,
  type IncomingFrame,
  type PairedFrame,
  type WelcomeFrame,
} from '../shared/protocol.ts';
import { EXTENSION_VERSION, browserInfo, capabilities, grantedList } from './env.ts';
import type { Store } from './state.ts';

export const PING_INTERVAL_MS = 20_000;
/** No frame at all for this long: the socket is dead even if it says open. */
const SILENCE_LIMIT_MS = 45_000;
const REQUEST_TIMEOUT_MS = 15_000;
/** After `replaced`, no automatic reconnect for this long. */
const REPLACED_HOLD_MS = 5 * 60_000;

export type ConnectionState =
  'unpaired' | 'connecting' | 'connected' | 'disconnected' | 'revoked' | 'auth_failed';

export interface ConnectionEvents {
  /** A frame from the desktop that is not a reply (state, lease.*, command). */
  frame(frame: IncomingFrame): void;
  /** Authenticated (after `welcome`). */
  welcome(frame: WelcomeFrame): void;
  /** State or error changed: refresh the UI. */
  changed(): void;
  /** The socket went away: in-flight work has no one to answer to. */
  closed(): void;
}

interface Waiter {
  resolve(frame: IncomingFrame): void;
  reject(error: Error): void;
  timer: ReturnType<typeof setTimeout>;
}

export class DesktopError extends Error {
  readonly code: string;
  constructor(code: string, message: string) {
    super(message);
    this.name = 'DesktopError';
    this.code = code;
  }
}

export class Connection {
  state: ConnectionState = 'unpaired';
  lastError = '';
  appVersion = '';
  /** When the next reconnect attempt is due (epoch ms), 0 when none. */
  retryAt = 0;
  /** Round trip of the last ping, ms. */
  pingMs = 0;

  private ws: WebSocket | null = null;
  private seq = 0;
  private attempt = 0;
  private readonly waiters = new Map<string, Waiter>();
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private pingTimer: ReturnType<typeof setInterval> | null = null;
  private lastInbound = 0;
  private authenticated = false;
  private replacedAt = 0;
  private opening: Promise<WebSocket> | null = null;
  private readonly store: Store;
  private readonly events: ConnectionEvents;

  constructor(store: Store, events: ConnectionEvents) {
    this.store = store;
    this.events = events;
  }

  get isAuthenticated(): boolean {
    return this.authenticated && this.ws?.readyState === WebSocket.OPEN;
  }

  /** Connect and authenticate if paired; a no-op when already connected or connecting. */
  ensure(): void {
    if (!this.store.auth) {
      this.setState(this.state === 'revoked' ? 'revoked' : 'unpaired');
      return;
    }
    if (this.ws || this.opening || this.reconnectTimer) return;
    // Replaced by another connection of this browser: stay out of its way a while.
    if (this.replacedAt && Date.now() - this.replacedAt < REPLACED_HOLD_MS) return;
    void this.connect();
  }

  /** Drop the socket and connect again now (settings changed, or the user asked). */
  restart(): void {
    this.attempt = 0;
    this.replacedAt = 0;
    this.closeSocket('restart');
    this.ensure();
  }

  /** Pair with the code the desktop shows, then authenticate. */
  async pair(code: string, name: string): Promise<void> {
    this.closeSocket('pairing');
    this.lastError = '';
    this.setState('connecting');
    let ws: WebSocket;
    try {
      ws = await this.open();
    } catch (e) {
      this.lastError = `Mother Claude is not reachable at ${this.store.config.host}:${this.store.config.port}`;
      this.setState(this.store.auth ? 'disconnected' : 'unpaired');
      this.closeSocket('pairing failed');
      throw new DesktopError('unreachable', this.lastError + ` (${messageOf(e)})`);
    }
    const reply = await this.request(
      {
        type: 'pair',
        code: code.trim().toUpperCase(),
        name,
        browser: await browserInfo(),
        extensionVersion: EXTENSION_VERSION,
      },
      ws,
    ).catch((e: unknown) => {
      this.closeSocket('pairing failed');
      this.setState(this.store.auth ? 'disconnected' : 'unpaired');
      throw e;
    });
    if (reply.type !== 'paired') throw new DesktopError('bad_reply', `unexpected ${reply.type}`);
    const paired = reply as PairedFrame;
    await this.store.saveAuth({
      token: paired.token,
      browserId: paired.browserId,
      pairedAt: Date.now(),
    });
    await this.authenticate(ws);
  }

  /** Forget the pairing (the user asked, or the desktop revoked it). */
  async forget(revoked = false): Promise<void> {
    await this.store.saveAuth(null);
    this.closeSocket(revoked ? 'revoked' : 'forgotten');
    this.clearReconnect();
    this.setState(revoked ? 'revoked' : 'unpaired');
  }

  /** Send a frame that expects no reply. False when it could not be sent. */
  send(frame: object): boolean {
    return this.trySend(frame) === 'sent';
  }

  trySend(frame: object): 'sent' | 'offline' | 'too_large' {
    const ws = this.ws;
    if (!ws || ws.readyState !== WebSocket.OPEN) return 'offline';
    const encoded = encodeFrame(frame);
    if (!encoded.ok) return 'too_large';
    try {
      ws.send(encoded.text);
      return 'sent';
    } catch {
      return 'offline';
    }
  }

  /** A request with an id; resolves with the reply, rejects on `error`. */
  request(frame: Record<string, unknown>, socket?: WebSocket): Promise<IncomingFrame> {
    const ws = socket ?? this.ws;
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      return Promise.reject(new DesktopError('disconnected', 'not connected to Mother Claude'));
    }
    const id = `x${++this.seq}`;
    const encoded = encodeFrame({ ...frame, id });
    if (!encoded.ok) return Promise.reject(new DesktopError('frame_too_large', 'frame too large'));
    return new Promise<IncomingFrame>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.waiters.delete(id);
        reject(new DesktopError('timeout', 'Mother Claude did not answer'));
      }, REQUEST_TIMEOUT_MS);
      this.waiters.set(id, { resolve, reject, timer });
      try {
        ws.send(encoded.text);
      } catch (e) {
        clearTimeout(timer);
        this.waiters.delete(id);
        reject(new DesktopError('disconnected', messageOf(e)));
      }
    });
  }

  // ------------------------------------------------------------------------

  private setState(s: ConnectionState): void {
    if (this.state === s) {
      this.events.changed();
      return;
    }
    this.state = s;
    this.events.changed();
  }

  private open(): Promise<WebSocket> {
    if (this.opening) return this.opening;
    const url = this.store.wsUrl;
    this.opening = new Promise<WebSocket>((resolve, reject) => {
      let ws: WebSocket;
      try {
        ws = new WebSocket(url);
      } catch (e) {
        reject(e instanceof Error ? e : new Error(String(e)));
        return;
      }
      const fail = (): void => {
        ws.removeEventListener('open', ok);
        reject(new Error(`cannot connect to ${url}`));
      };
      const ok = (): void => {
        ws.removeEventListener('error', fail);
        ws.removeEventListener('close', fail);
        this.attach(ws);
        resolve(ws);
      };
      ws.addEventListener('open', ok, { once: true });
      ws.addEventListener('error', fail, { once: true });
      ws.addEventListener('close', fail, { once: true });
    }).finally(() => {
      this.opening = null;
    });
    return this.opening;
  }

  private attach(ws: WebSocket): void {
    this.ws = ws;
    this.lastInbound = Date.now();
    ws.addEventListener('message', (ev: MessageEvent) => {
      if (this.ws !== ws) return;
      this.lastInbound = Date.now();
      if (typeof ev.data === 'string') this.onMessage(ev.data, ws);
    });
    ws.addEventListener('close', () => {
      if (this.ws !== ws) return;
      this.onClosed();
    });
    this.startPing(ws);
  }

  private async connect(): Promise<void> {
    this.clearReconnect();
    this.setState('connecting');
    let ws: WebSocket;
    try {
      ws = await this.open();
    } catch (e) {
      this.lastError = `Mother Claude is not reachable at ${this.store.config.host}:${this.store.config.port} (${messageOf(e)})`;
      this.scheduleReconnect();
      return;
    }
    try {
      await this.authenticate(ws);
    } catch (e) {
      if (e instanceof DesktopError && (e.code === 'revoked' || e.code === 'unauthenticated'))
        return;
      this.lastError = messageOf(e);
      this.closeSocket('hello failed');
      this.scheduleReconnect();
    }
  }

  private async authenticate(ws: WebSocket): Promise<void> {
    const auth = this.store.auth;
    if (!auth) throw new DesktopError('unpaired', 'not paired');
    const reply = await this.request(
      {
        type: 'hello',
        token: auth.token,
        extensionVersion: EXTENSION_VERSION,
        browser: await browserInfo(),
        capabilities: await capabilities(),
        granted: await grantedList(),
        // The browser was closed since the desktop last welcomed us: the
        // leases from before end (the desktop keeps them otherwise).
        ...(this.store.session.freshBrowser ? { freshBrowser: true } : {}),
      },
      ws,
    );
    if (reply.type !== 'welcome') throw new DesktopError('bad_reply', `unexpected ${reply.type}`);
    const welcome = reply as WelcomeFrame;
    // Before any await: frames right behind `welcome` must not be dropped.
    this.authenticated = true;
    if (this.store.session.freshBrowser) {
      this.store.session.freshBrowser = false;
      this.store.persist();
    }
    this.attempt = 0;
    this.lastError = '';
    this.appVersion = welcome.appVersion ?? '';
    this.setState('connected');
    this.events.welcome(welcome);
    if (welcome.browserId && welcome.browserId !== auth.browserId) {
      await this.store.saveAuth({ ...auth, browserId: welcome.browserId });
    }
  }

  private onMessage(text: string, ws: WebSocket): void {
    const decoded = decodeIncoming(text);
    if (!decoded.ok) {
      if (decoded.reply) this.sendOn(ws, decoded.reply);
      return;
    }
    const frame = decoded.frame;
    const id = 'id' in frame ? frame.id : undefined;
    if (id && this.waiters.has(id) && isReply(frame)) {
      const waiter = this.waiters.get(id);
      this.waiters.delete(id);
      if (waiter) {
        clearTimeout(waiter.timer);
        if (frame.type === 'error') {
          waiter.reject(new DesktopError(frame.code, frame.message ?? frame.code));
          this.onErrorFrame(frame);
        } else waiter.resolve(frame);
      }
      return;
    }
    if (frame.type === 'error') {
      this.onErrorFrame(frame);
      return;
    }
    if (frame.type === 'pong' || frame.type === 'ok') return;
    if (!this.authenticated && frame.type !== 'state') return;
    this.events.frame(frame);
  }

  private onErrorFrame(frame: ErrorFrame): void {
    if (frame.code === 'revoked') {
      this.lastError = 'This browser was removed in Mother Claude. Pair it again to continue.';
      void this.forget(true);
    } else if (frame.code === 'unauthenticated' && !this.authenticated) {
      this.lastError =
        frame.message ?? 'Mother Claude did not accept this browser’s pairing. Pair it again.';
      this.closeSocket('unauthenticated');
      this.setState('auth_failed');
      // Keep trying slowly: the desktop may simply have restarted.
      this.scheduleReconnect(Math.max(this.attempt, 5));
    } else if (frame.code === 'replaced') {
      // A newer connection of this browser took over (the desktop keeps one).
      // Reconnecting from here would only push it out again.
      this.lastError =
        'Another connection of this browser to Mother Claude took over. Press Reconnect to take it back.';
      this.replacedAt = Date.now();
      this.closeSocket('replaced');
      this.clearReconnect();
      this.setState('disconnected');
    }
  }

  private sendOn(ws: WebSocket, frame: object): void {
    const encoded = encodeFrame(frame);
    if (encoded.ok && ws.readyState === WebSocket.OPEN) ws.send(encoded.text);
  }

  private startPing(ws: WebSocket): void {
    this.stopPing();
    this.pingTimer = setInterval(() => {
      if (this.ws !== ws || ws.readyState !== WebSocket.OPEN) return;
      if (Date.now() - this.lastInbound > SILENCE_LIMIT_MS) {
        this.lastError = 'Mother Claude stopped answering';
        ws.close(4000, 'silent');
        this.onClosed();
        return;
      }
      const started = Date.now();
      this.request({ type: 'ping' }, ws).then(
        () => {
          this.pingMs = Date.now() - started;
        },
        () => undefined,
      );
    }, PING_INTERVAL_MS);
  }

  private stopPing(): void {
    if (this.pingTimer) clearInterval(this.pingTimer);
    this.pingTimer = null;
  }

  private onClosed(): void {
    const wasConnected = this.authenticated;
    this.ws = null;
    this.authenticated = false;
    this.stopPing();
    for (const [id, w] of this.waiters) {
      clearTimeout(w.timer);
      w.reject(new DesktopError('disconnected', 'the connection to Mother Claude closed'));
      this.waiters.delete(id);
    }
    this.events.closed();
    if (this.state === 'revoked' || this.state === 'unpaired') return;
    if (!this.store.auth) {
      this.setState('unpaired');
      return;
    }
    if (wasConnected) this.lastError = 'The connection to Mother Claude closed';
    this.scheduleReconnect();
  }

  private closeSocket(reason: string): void {
    const ws = this.ws;
    this.ws = null;
    this.authenticated = false;
    this.stopPing();
    if (ws) {
      try {
        ws.close(1000, reason);
      } catch {
        // already closing
      }
      for (const [id, w] of this.waiters) {
        clearTimeout(w.timer);
        w.reject(new DesktopError('disconnected', reason));
        this.waiters.delete(id);
      }
      this.events.closed();
    }
  }

  private scheduleReconnect(attempt = this.attempt): void {
    if (!this.store.auth) return;
    this.clearReconnect();
    const delay = backoffDelay(attempt);
    this.attempt = attempt + 1;
    this.retryAt = Date.now() + delay;
    if (this.state !== 'auth_failed') this.setState('disconnected');
    else this.events.changed();
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      this.retryAt = 0;
      void this.connect();
    }, delay);
  }

  private clearReconnect(): void {
    if (this.reconnectTimer) clearTimeout(this.reconnectTimer);
    this.reconnectTimer = null;
    this.retryAt = 0;
  }

  /** The alarm's safety net: a reconnect timer lost to a worker restart. */
  wake(): void {
    if (this.reconnectTimer && this.retryAt && Date.now() > this.retryAt + 5_000) {
      this.clearReconnect();
    }
    this.ensure();
  }
}

function isReply(frame: IncomingFrame): boolean {
  return (
    frame.type === 'ok' ||
    frame.type === 'error' ||
    frame.type === 'paired' ||
    frame.type === 'welcome' ||
    frame.type === 'pong'
  );
}
