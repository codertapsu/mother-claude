import { DestroyRef, Injectable, NgZone, Signal, computed, inject, signal } from '@angular/core';
import { Observable, Subject } from 'rxjs';

import { ApiService } from './api.service';
import { AuthService } from './auth.service';
import { ConfigService } from './config.service';
import { WebSocketLike, activeMock } from './mock/mock-api';
import {
  AgentsFrame,
  ClientFrame,
  FleetFrame,
  HelloFrame,
  NoticeFrame,
  PairRequestFrame,
  PendingItem,
  ServerFrame,
  SessionSummary,
  SessionsFrame,
  StreamFrame,
  TranscriptFrame,
} from './models';
import { TranscriptStore, TranscriptStoreOptions } from './transcript-store';

/**
 * - `connecting`   first connect, or the socket is open but not authed yet
 * - `open`         authed (`hello` received)
 * - `reconnecting` dropped; retrying with backoff (≤ 8 s)
 * - `offline`      the browser reports no network; retry when it returns
 * - `unpaired`     no token, or the server rejected it (close 4401)
 * - `idle`         not started / stopped
 */
export type ConnectionState =
  | 'idle'
  | 'connecting'
  | 'open'
  | 'reconnecting'
  | 'offline'
  | 'unpaired';

/** Human label for a connection state. */
export function connectionLabel(state: ConnectionState): string {
  switch (state) {
    case 'open':
      return 'Live';
    case 'connecting':
      return 'Connecting…';
    case 'reconnecting':
      return 'Reconnecting…';
    case 'offline':
      return 'Offline';
    case 'unpaired':
      return 'Not paired';
    default:
      return 'Not connected';
  }
}

/** Why consumers should refetch REST-loaded data. */
export type RefetchReason = 'reconnect' | 'resync' | 'resume' | 'online';

const MAX_BACKOFF_MS = 8000;
/** No hello within this long after the socket opened ⇒ reconnect. */
const AUTH_TIMEOUT_MS = 10_000;
/** The server pings every 25 s; silence this long means a dead socket. */
const SILENCE_LIMIT_MS = 65_000;
/** After this long in the background, treat coming back as a resume. */
const RESUME_AFTER_MS = 30_000;
/** Liveness probe answer time when the page becomes visible again. */
const PROBE_TIMEOUT_MS = 5_000;

/**
 * RealtimeService — the WebSocket v2 client (design contract §4.6).
 *
 * Public API (other pages, e.g. session detail, build on this):
 *
 * State (signals):
 * - `connection`       {@link ConnectionState}; the shell shows it as a dot.
 * - `sessions`         the live session store (scope `active`), maintained
 *                      from `sessions{full}` + `sessions{upsert,remove}` frames.
 * - `sessionsLoaded`   true once the first full snapshot arrived.
 * - `pendingItems`     every PendingItem across the store, oldest first.
 * - `session(id)`      a signal of one session from the store (undefined when
 *                      it's not in the active scope).
 *
 * Streams (RxJS; one emission per frame — never latest-value signals, so a
 * burst of frames can't be coalesced away):
 * - `hello$`, `sessions$` (raw), `transcript$`, `stream$`, `agents$`,
 *   `fleet$`, `notice$`, `devices$`, `pairRequest$`.
 * - `refetch$` emits a {@link RefetchReason} whenever REST-loaded data may be
 *   stale: after a reconnect, a server `resync`, returning to the page after
 *   ≥ 30 s in the background, or the network coming back. Pages that hold
 *   REST data refetch on it.
 *
 * Subscriptions (ref-counted; each call returns its own release function):
 * - `subscribe(sessionId)` ⇒ `sub{session, since}`; transcript/agents/stream
 *   frames for it start flowing. Automatically re-sent after every reconnect
 *   with `since` = the last transcript tail seen (the server replays the gap).
 * - `subscribeFleet()` ⇒ `subFleet{}` (fleet frames).
 * - `noteTail(sessionId, tail)` seeds `since` from a REST transcript fetch.
 * - `openTranscript(sessionId, opts)` ⇒ a {@link TranscriptStore}: REST tail +
 *   live appends + "load earlier" + refetch on reconnect, deduped by event id.
 *
 * Lifecycle: `start()` (idempotent; waits for a token), `restart()` after the
 * token changes (pairing), `stop()`. Ping frames get a `pong`; `visible`
 * frames report page visibility (the server skips push for foreground
 * devices). Close code 4401 ⇒ AuthService.handleUnauthorized() ⇒ /pair.
 */
@Injectable({ providedIn: 'root' })
export class RealtimeService {
  private config = inject(ConfigService);
  private auth = inject(AuthService);
  private api = inject(ApiService);
  private zone = inject(NgZone);

  private readonly connectionS = signal<ConnectionState>('idle');
  readonly connection = this.connectionS.asReadonly();

  private readonly store = signal<ReadonlyMap<string, SessionSummary>>(new Map());
  readonly sessions: Signal<SessionSummary[]> = computed(() => [...this.store().values()]);
  private readonly sessionsLoadedS = signal(false);
  readonly sessionsLoaded = this.sessionsLoadedS.asReadonly();

  readonly pendingItems: Signal<PendingItem[]> = computed(() =>
    this.sessions()
      .flatMap((s) => s.pending ?? [])
      .sort((a, b) => a.createdAt - b.createdAt),
  );

  private readonly helloS = new Subject<HelloFrame>();
  private readonly sessionsS = new Subject<SessionsFrame>();
  private readonly transcriptS = new Subject<TranscriptFrame>();
  private readonly streamS = new Subject<StreamFrame>();
  private readonly agentsS = new Subject<AgentsFrame>();
  private readonly fleetS = new Subject<FleetFrame>();
  private readonly noticeS = new Subject<NoticeFrame>();
  private readonly devicesS = new Subject<void>();
  private readonly pairRequestS = new Subject<PairRequestFrame>();
  private readonly refetchS = new Subject<RefetchReason>();

  readonly hello$: Observable<HelloFrame> = this.helloS.asObservable();
  readonly sessions$: Observable<SessionsFrame> = this.sessionsS.asObservable();
  readonly transcript$: Observable<TranscriptFrame> = this.transcriptS.asObservable();
  readonly stream$: Observable<StreamFrame> = this.streamS.asObservable();
  readonly agents$: Observable<AgentsFrame> = this.agentsS.asObservable();
  readonly fleet$: Observable<FleetFrame> = this.fleetS.asObservable();
  readonly notice$: Observable<NoticeFrame> = this.noticeS.asObservable();
  readonly devices$: Observable<void> = this.devicesS.asObservable();
  readonly pairRequest$: Observable<PairRequestFrame> = this.pairRequestS.asObservable();
  readonly refetch$: Observable<RefetchReason> = this.refetchS.asObservable();

  private socket: WebSocketLike | null = null;
  private started = false;
  private authed = false;
  private everConnected = false;
  private attempt = 0;
  private lastSeq = -1;
  private lastFrameAt = 0;
  private hiddenAt = 0;
  private reconnectTimer?: ReturnType<typeof setTimeout>;
  private authTimer?: ReturnType<typeof setTimeout>;
  private probeTimer?: ReturnType<typeof setTimeout>;
  private watchdog?: ReturnType<typeof setInterval>;
  private resyncing = false;
  private listenersBound = false;
  /** session id → { ref count, last transcript tail } */
  private readonly subs = new Map<string, { count: number; tail?: string }>();
  private fleetSubs = 0;

  /** Open the socket (once a token exists). Safe to call repeatedly. */
  async start(): Promise<void> {
    if (this.started) return;
    this.started = true;
    await this.config.ensure();
    this.bindGlobalListeners();
    this.connect();
  }

  /** Reconnect from scratch, e.g. right after pairing stored a new token. */
  restart(): void {
    this.started = true;
    this.bindGlobalListeners();
    this.attempt = 0;
    this.teardown();
    this.connect();
  }

  /** Close the socket and stop reconnecting. */
  stop(): void {
    this.started = false;
    this.teardown();
    this.connectionS.set('idle');
  }

  /** A signal of one session from the live store. */
  session(id: string): Signal<SessionSummary | undefined> {
    return computed(() => this.store().get(id));
  }

  /** Ref-counted `sub`; call the returned function to release it. */
  subscribe(sessionId: string): () => void {
    const entry = this.subs.get(sessionId);
    if (entry) {
      entry.count += 1;
    } else {
      this.subs.set(sessionId, { count: 1 });
      this.sendSub(sessionId);
    }
    let released = false;
    return () => {
      if (released) return;
      released = true;
      const e = this.subs.get(sessionId);
      if (!e) return;
      e.count -= 1;
      if (e.count <= 0) {
        this.subs.delete(sessionId);
        this.send({ t: 'unsub', session: sessionId });
      }
    };
  }

  /** Ref-counted `subFleet`; call the returned function to release it. */
  subscribeFleet(): () => void {
    this.fleetSubs += 1;
    if (this.fleetSubs === 1) this.send({ t: 'subFleet' });
    let released = false;
    return () => {
      if (released) return;
      released = true;
      this.fleetSubs -= 1;
      if (this.fleetSubs === 0) this.send({ t: 'unsubFleet' });
    };
  }

  /** Record the newest transcript cursor we hold (used as `since` on resubscribe). */
  noteTail(sessionId: string, tail: string | undefined): void {
    const e = this.subs.get(sessionId);
    if (e && tail) e.tail = tail;
  }

  /** A live, deduped transcript for one session (or one of its agents).
   * Pass `destroyRef` to release it automatically, else call `destroy()`. */
  openTranscript(
    sessionId: string,
    opts: TranscriptStoreOptions & { destroyRef?: DestroyRef } = {},
  ): TranscriptStore {
    const store = new TranscriptStore(this.api, this, sessionId, opts);
    opts.destroyRef?.onDestroy(() => store.destroy());
    return store;
  }

  // ────────────────────────────────────────────────────────── Connection

  private connect(): void {
    clearTimeout(this.reconnectTimer);
    if (!this.started) return;
    const cfg = this.config.config();
    if (!cfg) return;
    if (this.auth.state() === 'unpaired' || (!cfg.token && !cfg.desktop)) {
      this.connectionS.set('unpaired');
      return;
    }
    if (navigator.onLine === false) {
      this.connectionS.set('offline');
      return;
    }
    this.connectionS.set(this.everConnected ? 'reconnecting' : 'connecting');
    this.authed = false;

    let socket: WebSocketLike;
    try {
      const mock = activeMock();
      socket = mock ? mock.openSocket(cfg.wsUrl) : new WebSocket(cfg.wsUrl);
    } catch {
      this.scheduleReconnect();
      return;
    }
    this.socket = socket;

    socket.onopen = () => {
      if (this.socket !== socket) return;
      this.lastFrameAt = Date.now();
      this.sendRaw(socket, { t: 'auth', token: this.config.token() });
      clearTimeout(this.authTimer);
      this.authTimer = setTimeout(() => {
        if (this.socket === socket && !this.authed) this.forceReconnect();
      }, AUTH_TIMEOUT_MS);
    };
    socket.onmessage = (ev: MessageEvent) => {
      if (this.socket !== socket) return;
      this.lastFrameAt = Date.now();
      this.dispatch(ev.data);
    };
    socket.onclose = (ev: CloseEvent) => {
      if (this.socket !== socket) return;
      this.socket = null;
      this.authed = false;
      clearTimeout(this.authTimer);
      if (ev.code === 4401 || ev.code === 4403) {
        this.connectionS.set('unpaired');
        this.auth.handleUnauthorized();
        if (!this.config.desktop()) return;
      }
      this.scheduleReconnect();
    };
    socket.onerror = () => {
      /* onclose follows and handles the retry */
    };
  }

  private scheduleReconnect(): void {
    if (!this.started) return;
    if (navigator.onLine === false) {
      this.connectionS.set('offline');
      return;
    }
    this.connectionS.set(this.everConnected ? 'reconnecting' : 'connecting');
    const base = Math.min(MAX_BACKOFF_MS, 500 * 2 ** this.attempt);
    const delay = Math.min(MAX_BACKOFF_MS, Math.round(base * (0.75 + Math.random() * 0.5)));
    this.attempt += 1;
    clearTimeout(this.reconnectTimer);
    this.reconnectTimer = setTimeout(() => this.connect(), delay);
  }

  /** Drop the current socket and connect again right away. */
  private forceReconnect(): void {
    this.attempt = 0;
    this.teardown();
    this.connect();
  }

  private teardown(): void {
    clearTimeout(this.reconnectTimer);
    clearTimeout(this.authTimer);
    clearTimeout(this.probeTimer);
    const s = this.socket;
    this.socket = null;
    this.authed = false;
    if (s) {
      s.onopen = s.onmessage = s.onclose = s.onerror = null;
      try {
        s.close(1000, 'client reconnect');
      } catch {
        /* already closed */
      }
    }
  }

  // ───────────────────────────────────────────────────────────── Frames

  private dispatch(raw: unknown): void {
    if (typeof raw !== 'string') return;
    let frame: ServerFrame;
    try {
      frame = JSON.parse(raw) as ServerFrame;
    } catch {
      return;
    }
    if (!frame || typeof frame !== 'object' || typeof frame.t !== 'string') return;
    switch (frame.t) {
      case 'hello':
        this.onHello(frame);
        break;
      case 'sessions':
        this.onSessions(frame);
        break;
      case 'transcript': {
        const e = this.subs.get(frame.session);
        if (e && frame.tail) e.tail = frame.tail;
        this.transcriptS.next(frame);
        break;
      }
      case 'stream':
        this.streamS.next(frame);
        break;
      case 'agents':
        this.agentsS.next(frame);
        break;
      case 'fleet':
        this.fleetS.next(frame);
        break;
      case 'notice':
        this.noticeS.next(frame);
        break;
      case 'devices':
        this.devicesS.next();
        break;
      case 'pairRequest':
        this.pairRequestS.next(frame);
        break;
      case 'resync':
        void this.resync();
        break;
      case 'ping':
        this.send({ t: 'pong' });
        break;
      case 'pong':
        clearTimeout(this.probeTimer);
        break;
      default:
        // Unknown frame kinds from a newer server are ignored.
        break;
    }
  }

  private onHello(frame: HelloFrame): void {
    clearTimeout(this.authTimer);
    this.authed = true;
    this.attempt = 0;
    this.connectionS.set('open');
    if (frame.me) this.auth.me.set(frame.me);
    this.send({ t: 'visible', v: document.visibilityState === 'visible' });
    for (const id of this.subs.keys()) this.sendSub(id);
    if (this.fleetSubs > 0) this.send({ t: 'subFleet' });
    const reconnect = this.everConnected;
    this.everConnected = true;
    this.helloS.next(frame);
    if (reconnect) this.refetchS.next('reconnect');
  }

  private onSessions(frame: SessionsFrame): void {
    if (frame.full) {
      const next = new Map<string, SessionSummary>();
      for (const s of frame.sessions ?? []) if (s?.id) next.set(s.id, s);
      this.store.set(next);
      this.lastSeq = typeof frame.seq === 'number' ? frame.seq : -1;
      this.sessionsLoadedS.set(true);
    } else {
      const seq = typeof frame.seq === 'number' ? frame.seq : -1;
      if (seq >= 0 && this.lastSeq >= 0 && seq <= this.lastSeq) return; // stale
      const gap = seq >= 0 && this.lastSeq >= 0 && seq !== this.lastSeq + 1;
      this.lastSeq = seq;
      this.store.update((prev) => {
        const next = new Map(prev);
        for (const id of frame.remove ?? []) next.delete(id);
        for (const s of frame.upsert ?? []) if (s?.id) next.set(s.id, s);
        return next;
      });
      if (gap) void this.reloadSessions();
    }
    this.sessionsS.next(frame);
  }

  /** The server dropped frames for us: rebuild the store and resubscribe. */
  private async resync(): Promise<void> {
    await this.reloadSessions();
    for (const id of this.subs.keys()) this.sendSub(id);
    if (this.fleetSubs > 0) this.send({ t: 'subFleet' });
    this.refetchS.next('resync');
  }

  /** Replace the store from REST (after a sequence gap or a resync). */
  private async reloadSessions(): Promise<void> {
    if (this.resyncing) return;
    this.resyncing = true;
    try {
      const page = await this.api.listSessions({ scope: 'active' });
      const next = new Map<string, SessionSummary>();
      for (const s of page.sessions ?? []) if (s?.id) next.set(s.id, s);
      this.store.set(next);
      this.lastSeq = -1; // accept whatever diff comes next
      this.sessionsLoadedS.set(true);
    } catch {
      // The next reconnect's full snapshot will repair the store.
    } finally {
      this.resyncing = false;
    }
  }

  private sendSub(sessionId: string): void {
    const since = this.subs.get(sessionId)?.tail;
    this.send(since ? { t: 'sub', session: sessionId, since } : { t: 'sub', session: sessionId });
  }

  private send(frame: ClientFrame): void {
    const s = this.socket;
    if (!s || !this.authed) return; // re-sent after the next hello
    this.sendRaw(s, frame);
  }

  private sendRaw(socket: WebSocketLike, frame: ClientFrame): void {
    if (socket.readyState !== 1) return;
    try {
      socket.send(JSON.stringify(frame));
    } catch {
      /* the close handler reconnects */
    }
  }

  // ────────────────────────────────────── Visibility, network, watchdog

  private bindGlobalListeners(): void {
    if (this.listenersBound) return;
    this.listenersBound = true;
    document.addEventListener('visibilitychange', () => this.onVisibility());
    window.addEventListener('online', () => {
      this.attempt = 0;
      if (!this.socket) this.connect();
      else this.probe();
      this.refetchS.next('online');
    });
    window.addEventListener('offline', () => {
      if (this.started && this.connectionS() !== 'unpaired') this.connectionS.set('offline');
    });
    this.zone.runOutsideAngular(() => {
      this.watchdog = setInterval(() => {
        if (!this.socket || !this.authed) return;
        if (Date.now() - this.lastFrameAt > SILENCE_LIMIT_MS) {
          this.zone.run(() => this.forceReconnect());
        }
      }, 15_000);
    });
  }

  private onVisibility(): void {
    const visible = document.visibilityState === 'visible';
    this.send({ t: 'visible', v: visible });
    if (!visible) {
      this.hiddenAt = Date.now();
      return;
    }
    if (!this.started || this.connectionS() === 'unpaired') return;
    const away = this.hiddenAt ? Date.now() - this.hiddenAt : 0;
    this.hiddenAt = 0;
    if (!this.socket) {
      // Waiting in backoff: don't make a returning user wait.
      this.attempt = 0;
      this.connect();
      return;
    }
    // Phones freeze sockets in the background without closing them: check
    // the socket still answers, else reconnect (which refetches).
    this.probe();
    if (away >= RESUME_AFTER_MS && this.authed) this.refetchS.next('resume');
  }

  /** Ping the server; no frame back within 5 s ⇒ the socket is dead. */
  private probe(): void {
    if (!this.socket || !this.authed) return;
    const sentAt = Date.now();
    this.send({ t: 'ping' });
    clearTimeout(this.probeTimer);
    this.probeTimer = setTimeout(() => {
      if (this.lastFrameAt < sentAt) this.forceReconnect();
    }, PROBE_TIMEOUT_MS);
  }
}
