import { TestBed } from '@angular/core/testing';

import { AuthService } from './auth.service';
import { ConfigService } from './config.service';
import { FakeBackend, FakeSocket, flush, me, useFakeServer } from './mock/fake-server.testing';
import { setActiveMock } from './mock/mock-api';
import { SessionSummary } from './models';
import { RealtimeService } from './realtime.service';

const session = (id: string, status: string): SessionSummary =>
  ({ id, status, live: true, pending: [] }) as unknown as SessionSummary;

describe('RealtimeService', () => {
  let backend: FakeBackend;
  let rt: RealtimeService;
  let auth: AuthService;
  let config: ConfigService;

  beforeEach(async () => {
    backend = new FakeBackend();
    useFakeServer(backend);
    config = TestBed.inject(ConfigService);
    auth = TestBed.inject(AuthService);
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

  it('does not poll through a tunnel pointed at the main port', async () => {
    backend.routes.set('/me', () => ({
      status: 200,
      body: me({ ingress: 'loopback', effectiveIngress: 'tunnel' }),
    }));
    const internals = rt as unknown as { maybeStartPolling(): Promise<void> };
    await internals.maybeStartPolling();
    expect(rt.connection()).not.toBe('limited');
  });
});
