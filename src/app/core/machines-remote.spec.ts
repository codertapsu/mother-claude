import { TestBed } from '@angular/core/testing';

import { MachineApiError, describeError } from './api.service';
import { AuthService } from './auth.service';
import { MachineScopes } from './machine-scope';
import { MachinesService } from './machines.service';
import { FakeBackend, me, useFakeServer } from './mock/fake-server.testing';
import { setActiveMock } from './mock/mock-api';
import {
  PEER_ID,
  ask,
  goLive,
  pairedMachines,
  remote,
  remoteList,
  remoteSession,
} from './mock/remote.testing';
import { RealtimeService } from './realtime.service';
import { RemoteSessionsService } from './remote-sessions.service';
import { machinesView, peer } from '../pages/machines/machines.testing';

const PROXY = `/machines/${PEER_ID}/proxy`;

describe("other computers' sessions (docs/PLAN-MACHINES.md §6)", () => {
  let backend: FakeBackend;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend);
  });

  afterEach(() => {
    TestBed.inject(RealtimeService).stop();
    setActiveMock(null);
  });

  it("keeps them apart from this computer's, each stamped with its computer", async () => {
    const s = await goLive(backend);
    const rt = TestBed.inject(RealtimeService);
    s.frame({ t: 'sessions', full: true, sessions: [remoteSession('here-1')], seq: 1 });
    remoteList(s, PEER_ID, [remoteSession('r1'), remoteSession('r2')]);
    expect(rt.sessions().map((x) => x.id)).toEqual(['here-1']);
    expect([...(rt.remote().get(PEER_ID)?.sessions.keys() ?? [])]).toEqual(['r1', 'r2']);
    expect(rt.remote().get(PEER_ID)?.sessions.get('r1')?.machine).toBe(PEER_ID);
    expect(rt.remote().get(PEER_ID)).toMatchObject({ loaded: true, online: true, since: null });

    remote(s, PEER_ID, {
      t: 'sessions',
      full: false,
      upsert: [remoteSession('r3')],
      remove: ['r1'],
      seq: 2,
    });
    expect([...(rt.remote().get(PEER_ID)?.sessions.keys() ?? [])]).toEqual(['r2', 'r3']);

    // The link drops: its sessions stay, as last heard; then it is back.
    remote(s, PEER_ID, { t: 'offline', since: 1234 });
    expect(rt.remote().get(PEER_ID)).toMatchObject({ online: false, since: 1234 });
    expect(rt.remote().get(PEER_ID)?.sessions.size).toBe(2);
    remote(s, PEER_ID, { t: 'online' });
    expect(rt.remote().get(PEER_ID)).toMatchObject({ online: true, since: null });

    // Revoked: gone.
    remote(s, PEER_ID, { t: 'forgotten' });
    expect(rt.remote().has(PEER_ID)).toBe(false);
    expect(rt.sessions().map((x) => x.id)).toEqual(['here-1']);
  });

  it('subscribes to one of their sessions with its computer, again after a reconnect', async () => {
    const s = await goLive(backend);
    const rt = TestBed.inject(RealtimeService);
    const release = rt.subscribeRemote(PEER_ID, 'r1');
    expect(s.sent).toContainEqual({ t: 'sub', session: 'r1', machine: PEER_ID });
    remote(s, PEER_ID, { t: 'transcript', session: 'r1', events: [], tail: '42' });

    rt.reconnectNow();
    const again = backend.last;
    expect(again).not.toBe(s);
    again.open();
    again.frame({ t: 'hello', v: 2, me: me(), serverTime: Date.now() });
    expect(again.sent).toContainEqual({ t: 'sub', session: 'r1', machine: PEER_ID, since: '42' });
    // Never as one of this computer's own sessions.
    expect(again.sent.filter((f) => f.t === 'sub' && !('machine' in f))).toEqual([]);

    release();
    expect(again.sent).toContainEqual({ t: 'unsub', session: 'r1', machine: PEER_ID });
  });

  it('calls through the proxy, and names the computer in what it refuses', async () => {
    pairedMachines();
    backend.routes.set(`${PROXY}/sessions/r1/message`, () => ({
      status: 403,
      body: {
        error: {
          code: 'grant_missing',
          message: 'This computer has not given yours the "control" permission.',
          grant: 'control',
        },
      },
    }));
    backend.routes.set(`${PROXY}/sessions/r1`, () => ({
      status: 502,
      body: {
        error: {
          code: 'machine_unreachable',
          message: "DESKTOP-PC can't be reached right now: timed out",
          machine: PEER_ID,
        },
      },
    }));
    backend.routes.set(`${PROXY}/inbox`, () => ({ status: 401, body: {} }));
    const api = TestBed.inject(MachineScopes).api(PEER_ID);

    const refused = await api.sendMessage('r1', { text: 'hi' }).catch((e: unknown) => e);
    expect(backend.calls).toContain(`POST ${PROXY}/sessions/r1/message`);
    expect(backend.calls).not.toContain('POST /sessions/r1/message');
    expect(refused).toBeInstanceOf(MachineApiError);
    expect(describeError(refused).text).toBe(
      'DESKTOP-PC lets this computer watch its sessions, not drive them.',
    );

    const away = await api.getSession('r1').catch((e: unknown) => e);
    expect(describeError(away)).toEqual({
      text: "DESKTOP-PC can't be reached right now.",
      retry: true,
    });

    // A refusal on the way to another computer never unpairs this device.
    await api.getInbox().catch(() => undefined);
    expect(TestBed.inject(AuthService).state()).toBe('paired');
  });

  it('lists them by name while Machines is on, their online requests too', async () => {
    const s = await goLive(backend);
    const list = TestBed.inject(RemoteSessionsService);
    pairedMachines();
    remoteList(s, PEER_ID, [remoteSession('r1', { pending: [ask('q1')] })]);
    expect(list.list().map((m) => m.name)).toEqual(['DESKTOP-PC']);
    expect(list.pending().map((p) => [p.id, p.sessionId, p.machine])).toEqual([
      ['q1', 'r1', PEER_ID],
    ]);

    remote(s, PEER_ID, { t: 'offline', since: 5 });
    expect(list.pending()).toEqual([]);
    expect(list.list()[0]).toMatchObject({ online: false, since: 5 });

    TestBed.inject(MachinesService).apply(machinesView({ enabled: false, peers: [peer()] }));
    expect(list.list()).toEqual([]);
  });
});
