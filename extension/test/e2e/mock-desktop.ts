// A stand-in for the desktop side of docs/BROWSER-PROTOCOL.md: pairing,
// hello/welcome, state, leases, commands, approvals and site modes, on an
// ephemeral loopback port. It refuses an Origin other than the pinned
// extension id, like the real server.

import { createServer, type Server } from 'node:http';
import type { AddressInfo } from 'node:net';
import { WebSocketServer, type WebSocket } from 'ws';

export type Frame = Record<string, unknown> & { type: string; id?: string };

export interface DesktopState {
  siteModes: Record<string, string>;
  settings: Record<string, unknown>;
  leases: Record<string, unknown>[];
  pending: Record<string, unknown>[];
}

export class MockDesktop {
  readonly pairCode = 'ABCD-EFGH';
  readonly token = 'mcx_e2e_token_0123456789';
  readonly browserId = 'brw_e2e';
  readonly sensitive = new Set<string>();
  readonly received: Frame[] = [];
  readonly origins: string[] = [];
  state: DesktopState = {
    siteModes: {},
    settings: { screenshots: true, defaultMode: 'off' },
    leases: [],
    pending: [],
  };
  /** What `sessions` answers (the side panel's list). */
  sessions: Record<string, unknown>[] = [];
  /** The `session.tell` frames received. */
  readonly told: Frame[] = [];
  port = 0;
  connections = 0;
  closes = 0;
  private http: Server | null = null;
  private wss: WebSocketServer | null = null;
  private socket: WebSocket | null = null;
  private authenticated = false;
  private seq = 0;
  private waiters: { pred: (f: Frame) => boolean; resolve: (f: Frame) => void }[] = [];
  private readonly expectedOrigin: string;

  constructor(extensionId: string) {
    this.expectedOrigin = `chrome-extension://${extensionId}`;
  }

  async start(): Promise<void> {
    this.http = createServer((_req, res) => {
      res.writeHead(426);
      res.end();
    });
    this.wss = new WebSocketServer({ noServer: true, maxPayload: 8 * 1024 * 1024 });
    this.http.on('upgrade', (req, socket, head) => {
      const origin = req.headers.origin ?? '';
      this.origins.push(origin);
      const peer = req.socket.remoteAddress ?? '';
      if (
        new URL(req.url ?? '/', 'http://x').pathname !== '/ws/browser' ||
        origin !== this.expectedOrigin ||
        !/^(::ffff:)?127\.0\.0\.1$|^::1$/.test(peer)
      ) {
        socket.write('HTTP/1.1 403 Forbidden\r\n\r\n');
        socket.destroy();
        return;
      }
      this.wss?.handleUpgrade(req, socket, head, (ws) => this.onConnection(ws));
    });
    await new Promise<void>((resolve) => this.http?.listen(0, '127.0.0.1', resolve));
    this.port = (this.http.address() as AddressInfo).port;
  }

  async stop(): Promise<void> {
    this.socket?.close();
    this.wss?.close();
    await new Promise<void>((r) => (this.http ? this.http.close(() => r()) : r()));
  }

  get isConnected(): boolean {
    return !!this.socket && this.authenticated;
  }

  private onConnection(ws: WebSocket): void {
    this.connections++;
    this.socket?.close();
    this.socket = ws;
    this.authenticated = false;
    ws.on('message', (raw) => this.onMessage(ws, String(raw)));
    ws.on('close', () => {
      this.closes++;
      if (this.socket === ws) {
        this.socket = null;
        this.authenticated = false;
      }
    });
  }

  private send(frame: Frame, ws: WebSocket | null = this.socket): void {
    ws?.send(JSON.stringify(frame));
  }

  /** Send anything, even a frame the extension does not know. */
  raw(frame: Frame): void {
    this.send(frame);
  }

  private onMessage(ws: WebSocket, text: string): void {
    const frame = JSON.parse(text) as Frame;
    const stamped: Frame = { ...frame, at: Date.now() };
    this.received.push(stamped);
    for (const w of [...this.waiters]) {
      if (w.pred(stamped)) {
        this.waiters.splice(this.waiters.indexOf(w), 1);
        w.resolve(stamped);
      }
    }
    switch (frame.type) {
      case 'pair':
        if (frame['code'] === this.pairCode) {
          this.send(
            { type: 'paired', id: frame.id ?? '', token: this.token, browserId: this.browserId },
            ws,
          );
        } else {
          this.send(
            { type: 'error', id: frame.id ?? '', code: 'bad_code', message: 'wrong code' },
            ws,
          );
        }
        return;
      case 'hello':
        if (frame['token'] !== this.token) {
          this.send(
            { type: 'error', id: frame.id ?? '', code: 'unauthenticated', message: 'bad token' },
            ws,
          );
          ws.close();
          return;
        }
        this.authenticated = true;
        this.send(
          {
            type: 'welcome',
            id: frame.id ?? '',
            browserId: this.browserId,
            protocol: 1,
            appVersion: '0.8.0-e2e',
            siteModes: this.state.siteModes,
            settings: this.state.settings,
          },
          ws,
        );
        this.pushState();
        return;
      case 'ping':
        this.send({ type: 'pong', id: frame.id ?? '' }, ws);
        return;
      case 'setSiteMode': {
        const origin = String(frame['origin']);
        if (this.sensitive.has(origin) && frame['mode'] === 'act') {
          this.send(
            { type: 'error', id: frame.id ?? '', code: 'needs_desktop', message: 'sensitive site' },
            ws,
          );
          return;
        }
        this.state.siteModes[origin] = String(frame['mode']);
        this.send({ type: 'ok', id: frame.id ?? '' }, ws);
        this.pushState();
        return;
      }
      case 'sessions':
        this.send({ type: 'ok', id: frame.id ?? '', data: { sessions: this.sessions } }, ws);
        return;
      case 'session.connect': {
        const session = this.sessions.find((x) => x['id'] === frame['session']);
        if (!session) {
          this.send(
            { type: 'error', id: frame.id ?? '', code: 'not_found', message: 'no such session' },
            ws,
          );
          return;
        }
        // As the real desktop: the lease opens before the answer.
        const lease = `l_connect_${String(session['id']).slice(0, 8)}`;
        void this.request({
          type: 'lease.open',
          lease,
          session: { id: session['id'], title: session['name'] },
          color: 'purple',
        }).then(() => {
          session['lease'] = lease;
          this.send(
            {
              type: 'ok',
              id: frame.id ?? '',
              data: { lease, title: session['name'], color: 'purple', hasTools: true },
            },
            ws,
          );
        });
        return;
      }
      case 'session.tell': {
        const session = this.sessions.find((x) => x['id'] === frame['session']);
        if (!session || session['canTell'] !== true) {
          this.send(
            {
              type: 'error',
              id: frame.id ?? '',
              code: 'no_channel',
              message: 'Mother Claude cannot reach this session.',
            },
            ws,
          );
          return;
        }
        this.told.push(frame);
        this.send({ type: 'ok', id: frame.id ?? '', data: { delivery: 'relayed' } }, ws);
        return;
      }
      case 'answer': {
        const i = this.state.pending.findIndex((p) => p['id'] === frame['pending']);
        if (i < 0) {
          this.send(
            { type: 'error', id: frame.id ?? '', code: 'already_answered', message: 'gone' },
            ws,
          );
          return;
        }
        this.state.pending.splice(i, 1);
        this.send({ type: 'ok', id: frame.id ?? '' }, ws);
        this.pushState();
        return;
      }
      default:
        return;
    }
  }

  pushState(): void {
    if (this.authenticated) this.send({ type: 'state', ...this.state });
  }

  setState(patch: Partial<DesktopState>): void {
    this.state = { ...this.state, ...patch };
    this.pushState();
  }

  /** The next frame (already received after `since`, or arriving) matching `pred`. */
  waitFor(pred: (f: Frame) => boolean, timeoutMs = 10_000, since = 0): Promise<Frame> {
    const seen = this.received.find((f) => (f['at'] as number) >= since && pred(f));
    if (seen) return Promise.resolve(seen);
    return new Promise<Frame>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.waiters.splice(this.waiters.indexOf(waiter), 1);
        reject(new Error(`no matching frame within ${timeoutMs} ms`));
      }, timeoutMs);
      const waiter = {
        pred,
        resolve: (f: Frame) => {
          clearTimeout(timer);
          resolve(f);
        },
      };
      this.waiters.push(waiter);
    });
  }

  event(
    name: string,
    extra: (f: Frame) => boolean = () => true,
    timeoutMs = 10_000,
    since = 0,
  ): Promise<Frame> {
    return this.waitFor(
      (f) => f.type === 'event' && f['event'] === name && extra(f),
      timeoutMs,
      since,
    );
  }

  /** Send a request and wait for the reply with the same id. */
  async request(frame: Frame, timeoutMs = 60_000): Promise<Frame> {
    const id = frame.id ?? `d${++this.seq}`;
    const reply = this.waitFor((f) => f.id === id && f.type !== 'event', timeoutMs);
    this.send({ ...frame, id });
    return reply;
  }

  async command(
    lease: string,
    action: string,
    params: Record<string, unknown> = {},
    opts: { tabId?: number; timeoutMs?: number } = {},
  ): Promise<{
    ok: boolean;
    data: Record<string, unknown>;
    error?: { code: string; message: string; data?: unknown };
    ms: number;
  }> {
    const started = performance.now();
    const res = await this.request(
      {
        type: 'command',
        id: `c${++this.seq}`,
        lease,
        action,
        params,
        timeoutMs: opts.timeoutMs ?? 30_000,
        ...(opts.tabId !== undefined ? { tabId: opts.tabId } : {}),
      },
      (opts.timeoutMs ?? 30_000) + 5_000,
    );
    const ms = performance.now() - started;
    return {
      ok: res['ok'] === true,
      data: (res['data'] ?? {}) as Record<string, unknown>,
      ...(res['error']
        ? { error: res['error'] as { code: string; message: string; data?: unknown } }
        : {}),
      ms,
    };
  }
}
