import { TestBed } from '@angular/core/testing';

import { AttentionService } from './attention.service';
import { MachinesService } from './machines.service';
import { job, machineAsk, machinesView, pairRequest, peer } from './mock/attention.testing';
import { FakeBackend, asDesktop, flush, useFakeServer } from './mock/fake-server.testing';
import { setActiveMock } from './mock/mock-api';
import { PendingPreview } from './models';
import { NavBadges } from './nav-badges';
import { RealtimeService } from './realtime.service';

const request = (id: string, answerable = true): PendingPreview => ({
  id,
  kind: 'permission',
  title: `Allow ${id}?`,
  dangerous: false,
  answerable,
  createdAt: Date.now(),
});

describe('AttentionService: the Inbox badge and dot', () => {
  let backend: FakeBackend;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend);
    backend.routes.set('/devices', () => ({
      status: 200,
      body: { devices: [], pending: [pairRequest()] },
    }));
    backend.routes.set('/data/jobs', () => ({ status: 200, body: { jobs: [] } }));
    backend.routes.set('/machines', () => ({ status: 404, body: {} }));
  });

  afterEach(() => {
    setActiveMock(null);
    // What a spec put away stays on this device: not for the next one.
    localStorage.removeItem('mc_inbox_dismissed');
    document.title = 'Mother Claude';
  });

  const store = (pending: PendingPreview[]) =>
    (
      TestBed.inject(RealtimeService) as unknown as { replaceStore(l: unknown[]): void }
    ).replaceStore([{ id: 's1', live: true, pending }]);
  const asks = () =>
    TestBed.inject(MachinesService).apply(
      machinesView({
        enabled: true,
        peers: [peer()],
        asks: [
          machineAsk(),
          machineAsk({ id: 'ask_running', state: 'running' }),
          machineAsk({ id: 'ask_done', state: 'done', answeredAt: Date.now() }),
        ],
      }),
    );

  it("counts, on the desktop, the requests answerable here, the devices waiting to pair and other computers' asks", async () => {
    asDesktop();
    const attention = TestBed.inject(AttentionService);
    attention.start();
    await flush();
    store([request('p1'), request('p2', false)]);
    asks();
    // One request (the other waits where it runs), one device, one ask (the
    // one under way is no question any more).
    expect(attention.pendingCount()).toBe(3);
    expect(backend.calls).toContain('GET /devices');
    TestBed.tick();
    expect(document.title).toBe('(3) Mother Claude');
    expect(attention.toFixCount()).toBe(0);
  });

  it('counts on a phone only what it can answer: the requests', async () => {
    const attention = TestBed.inject(AttentionService);
    attention.start();
    await flush();
    store([request('p1')]);
    asks();
    expect(attention.pairRequests()).toEqual([]);
    expect(attention.asks().length).toBe(2);
    expect(attention.pendingCount()).toBe(1);
    // The desktop's pairing requests are never asked for.
    expect(backend.calls).not.toContain('GET /devices');
  });

  it('adds a dot for what to fix while no request waits, and names both for screen readers', async () => {
    backend.routes.set('/data/jobs', () => ({ status: 200, body: { jobs: [job()] } }));
    const attention = TestBed.inject(AttentionService);
    const badges = TestBed.inject(NavBadges);
    attention.start();
    await flush();
    expect(attention.toFixCount()).toBe(1);
    expect(badges.count('pending')).toBe(0);
    expect(badges.dot('pending')).toBe(true);
    expect(badges.label({ label: 'Inbox', badge: 'pending' })).toBe('Inbox, 1 to fix');

    store([request('p1')]);
    expect(badges.count('pending')).toBe(1);
    expect(badges.dot('pending')).toBe(false);
    expect(badges.label({ label: 'Inbox', badge: 'pending' })).toBe('Inbox, 1 waiting, 1 to fix');

    attention.dismiss('job:job_1');
    expect(attention.toFixCount()).toBe(0);
    expect(badges.label({ label: 'Inbox', badge: 'pending' })).toBe('Inbox, 1 waiting');
  });

  it('reads the failed jobs again after a reconnect', async () => {
    const attention = TestBed.inject(AttentionService);
    attention.start();
    await flush();
    const before = backend.calls.filter((c) => c === 'GET /data/jobs').length;
    // The server dropped frames: views refetch what they read.
    (TestBed.inject(RealtimeService) as unknown as { dispatch(raw: string): void }).dispatch(
      JSON.stringify({ t: 'resync' }),
    );
    await flush();
    expect(backend.calls.filter((c) => c === 'GET /data/jobs').length).toBe(before + 1);
  });
});
