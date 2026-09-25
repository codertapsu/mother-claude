import { TestBed } from '@angular/core/testing';

import { AuthService } from './auth.service';
import { ConfigService } from './config.service';
import { MockBackend, WebSocketLike, setActiveMock } from './mock/mock-api';
import { MeInfo, SessionSummary } from './models';
import { RealtimeService } from './realtime.service';

class FakeSocket implements WebSocketLike {
  readyState = 0;
  onopen: ((ev: Event) => void) | null = null;
  onmessage: ((ev: MessageEvent) => void) | null = null;
  onclose: ((ev: CloseEvent) => void) | null = null;
  onerror: ((ev: Event) => void) | null = null;
  readonly sent: { t: string }[] = [];

  send(data: string): void {
    this.sent.push(JSON.parse(data));
  }
  close(): void {
    this.readyState = 3;
  }
  open(): void {
    this.readyState = 1;
    this.onopen?.(new Event('open'));
  }
  frame(obj: unknown): void {
    this.onmessage?.(new MessageEvent('message', { data: JSON.stringify(obj) }));
  }
  closeWith(code: number): void {
    this.readyState = 3;
    this.onclose?.(new CloseEvent('close', { code }));
  }
}

type Route = (
  method: string,
) => { status: number; body: unknown } | Promise<{ status: number; body: unknown }>;

class FakeBackend implements MockBackend {
  readonly role = 'device' as const;
  readonly token = 'tok';
  readonly sockets: FakeSocket[] = [];
  readonly calls: string[] = [];
  readonly routes = new Map<string, Route>();

  async fetch(url: string, init?: RequestInit): Promise<Response> {
    const path = new URL(url).pathname.replace(/^\/api/, '');
    const method = init?.method ?? 'GET';
    this.calls.push(`${method} ${path}`);
    const route = this.routes.get(path);
    const { status, body } = route ? await route(method) : { status: 404, body: {} };
    return new Response(JSON.stringify(body), {
      status,
      headers: { 'content-type': 'application/json' },
    });
  }

  openSocket(): WebSocketLike {
    const s = new FakeSocket();
    this.sockets.push(s);
    return s;
  }

  get last(): FakeSocket {
    return this.sockets[this.sockets.length - 1];
  }
}

const me = (over: Partial<MeInfo> = {}): MeInfo => ({
  principal: 'device',
  ingress: 'lan',
  local: false,
  allowDangerous: false,
  ...over,
});

const session = (id: string, status: string): SessionSummary =>
  ({ id, status, live: true, pending: [] }) as unknown as SessionSummary;

const flush = async (times = 5) => {
  for (let i = 0; i < times; i++) await new Promise((r) => setTimeout(r, 0));
};

describe('RealtimeService', () => {
  let backend: FakeBackend;
  let rt: RealtimeService;
  let auth: AuthService;
  let config: ConfigService;

  beforeEach(async () => {
    backend = new FakeBackend();
    setActiveMock(backend);
    TestBed.configureTestingModule({});
    config = TestBed.inject(ConfigService);
    config.config.set({
      baseUrl: 'http://mc.test',
      wsUrl: 'ws://mc.test/ws',
      token: 'tok',
      desktop: false,
      mock: true,
    });
    auth = TestBed.inject(AuthService);
    auth.state.set('paired');
    rt = TestBed.inject(RealtimeService);
    await rt.start();
  });

  afterEach(() => {
    rt.stop();
    setActiveMock(null);
  });

  /** Open the socket and complete the auth handshake. */
  const goLive = (): FakeSocket => {
    const s = backend.last;
    s.open();
    s.frame({ t: 'hello', v: 2, me: me(), serverTime: Date.now() });
    return s;
  };

  it('comes back to Live when a frame arrives after an offline blip', () => {
    const s = goLive();
    expect(rt.connection()).toBe('open');
    window.dispatchEvent(new Event('offline'));
    expect(rt.connection()).toBe('offline');
    window.dispatchEvent(new Event('online'));
    // The probe went out on the surviving socket…
    expect(s.sent.some((f) => f.t === 'ping')).toBeTrue();
    // …and its answer proves the link.
    s.frame({ t: 'pong' });
    expect(rt.connection()).toBe('open');
    expect(rt.canAct()).toBeTrue();
  });

  it('keeps the device token when REST still accepts it after a 4401 close', async () => {
    backend.routes.set('/me', () => ({ status: 200, body: me() }));
    const s = goLive();
    s.closeWith(4401);
    await flush();
    expect(backend.calls).toContain('GET /me');
    expect(config.token()).toBe('tok');
    expect(auth.state()).toBe('paired');
    expect(rt.connection()).toBe('reconnecting');
  });

  it('unpairs after a 4401 close when REST rejects the token too', async () => {
    backend.routes.set('/me', () => ({ status: 401, body: { error: { code: 'unauthorized' } } }));
    const s = goLive();
    s.closeWith(4401);
    await flush();
    expect(auth.state()).toBe('unpaired');
    expect(auth.reason()).toBe('revoked');
    expect(config.token()).toBe('');
  });

  it('ignores REST copies of a session while the socket is live', () => {
    const s = goLive();
    s.frame({ t: 'sessions', full: true, seq: 1, sessions: [session('s1', 'idle')] });
    rt.mergeSession(session('s1', 'working'));
    expect(rt.session('s1')()?.status).toBe('idle');
  });

  it('merges REST copies of a session while not live', () => {
    const s = goLive();
    s.frame({ t: 'sessions', full: true, seq: 1, sessions: [session('s1', 'idle')] });
    window.dispatchEvent(new Event('offline'));
    rt.mergeSession(session('s1', 'working'));
    expect(rt.session('s1')()?.status).toBe('working');
  });

  it('does not reload the store over REST on resync (a full frame follows)', async () => {
    const s = goLive();
    s.frame({ t: 'resync' });
    await flush();
    expect(backend.calls.filter((c) => c.startsWith('GET /sessions'))).toEqual([]);
  });

  it('replays diffs that arrived while a gap reload was in flight', async () => {
    let answer!: (v: { status: number; body: unknown }) => void;
    backend.routes.set(
      '/sessions',
      () => new Promise<{ status: number; body: unknown }>((r) => (answer = r)),
    );
    const s = goLive();
    s.frame({ t: 'sessions', full: true, seq: 1, sessions: [session('s1', 'idle')] });
    // seq 3 after 1: a gap ⇒ reload over REST.
    s.frame({ t: 'sessions', full: false, seq: 3, upsert: [session('s2', 'idle')] });
    await flush();
    expect(backend.calls).toContain('GET /sessions');
    // A newer diff lands before the (older) REST answer.
    s.frame({ t: 'sessions', full: false, seq: 4, upsert: [session('s1', 'working')] });
    answer({ status: 200, body: { sessions: [session('s1', 'idle'), session('s2', 'idle')] } });
    await flush();
    expect(rt.session('s1')()?.status).toBe('working');
    expect(rt.session('s2')()?.status).toBe('idle');
  });

  it('drops a gap reload when a full frame arrived meanwhile', async () => {
    let answer!: (v: { status: number; body: unknown }) => void;
    backend.routes.set(
      '/sessions',
      () => new Promise<{ status: number; body: unknown }>((r) => (answer = r)),
    );
    const s = goLive();
    s.frame({ t: 'sessions', full: true, seq: 1, sessions: [session('s1', 'idle')] });
    s.frame({ t: 'sessions', full: false, seq: 3, upsert: [] });
    await flush();
    s.frame({ t: 'sessions', full: true, seq: 5, sessions: [session('s1', 'working')] });
    answer({ status: 200, body: { sessions: [session('s1', 'idle')] } });
    await flush();
    expect(rt.session('s1')()?.status).toBe('working');
  });
});
