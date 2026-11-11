// Fixtures for the specs of other computers' sessions (docs/PLAN-MACHINES.md
// §6): their sessions and requests, and the `remote` frames that carry them.
import { TestBed } from '@angular/core/testing';

import { MachinesService } from '../machines.service';
import { PendingItem, PendingPreview, SessionSummary } from '../models';
import { RealtimeService } from '../realtime.service';
import { FakeBackend, FakeSocket, me } from './fake-server.testing';
import { ownedCaps } from './fixtures';
import { machinesView, peer, PEER_ID } from '../../pages/machines/machines.testing';

export { PEER_ID };
/** A second paired computer. */
export const PEER2_ID = 'e'.repeat(32);

/** One session, as another computer's frames carry it (no `machine`). */
export function remoteSession(id: string, over: Partial<SessionSummary> = {}): SessionSummary {
  return {
    id,
    name: id,
    title: `Session ${id}`,
    projectName: 'mc',
    cwd: 'D:\\dev\\mc',
    surface: 'mother-claude',
    owned: true,
    live: true,
    status: 'working',
    activity: { kind: 'thinking', label: 'Thinking' },
    usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
    messageCount: 1,
    agents: { running: 0, total: 0, workflowsRunning: 0, backgroundRunning: 0 },
    queued: 0,
    pending: [],
    control: ownedCaps(),
    lastActivityAt: Date.now(),
    ...over,
  } as SessionSummary;
}

/** A permission request's list preview. */
export function ask(id: string, over: Partial<PendingPreview> = {}): PendingPreview {
  return {
    id,
    kind: 'permission',
    title: 'Allow Bash?',
    detail: 'cargo build --release',
    toolName: 'Bash',
    dangerous: false,
    answerable: true,
    createdAt: Date.now() - 60_000,
    ...over,
  };
}

/** The same request in full. */
export function askFull(id: string, sessionId: string): PendingItem {
  return {
    ...ask(id),
    sessionId,
    source: 'owned',
    input: { command: 'cargo build --release' },
    suggestions: [],
  } as PendingItem;
}

/** Live updates up, as the desktop shell starts them. */
export async function goLive(backend: FakeBackend): Promise<FakeSocket> {
  await TestBed.inject(RealtimeService).start();
  const s = backend.last;
  s.open();
  s.frame({ t: 'hello', v: 2, me: me(), serverTime: Date.now() });
  return s;
}

/** One of another computer's frames, as this computer republishes it. */
export function remote(s: FakeSocket, machine: string, frame: unknown): void {
  s.frame({ t: 'remote', machine, frame });
}

/** Another computer's whole session list. */
export function remoteList(s: FakeSocket, machine: string, sessions: SessionSummary[]): void {
  remote(s, machine, { t: 'sessions', full: true, sessions, seq: 1 });
}

/** Machines on, with DESKTOP-PC (and BUILD-BOX when asked) paired. */
export function pairedMachines(withSecond = false): void {
  TestBed.inject(MachinesService).apply(
    machinesView({
      enabled: true,
      peers: [
        peer(),
        ...(withSecond
          ? [peer({ id: PEER2_ID, name: 'BUILD-BOX', online: false, lastSeen: Date.now() })]
          : []),
      ],
    }),
  );
}
