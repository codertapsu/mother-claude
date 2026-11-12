// Fixtures for the specs of what waits for you (core/attention.ts, the
// Inbox): the sources the To fix items come from, as the server sends them.
import {
  PEER_ID,
  check,
  machinesView,
  peer,
  setupView,
} from '../../pages/machines/machines.testing';
import { KeptCopy } from '../copies';
import { FixSources } from '../attention';
import { MachineAsk, SyncPair } from '../machines';
import { JobView, PendingPair, PermissionInfo } from '../models';

export { PEER_ID, check, machinesView, peer, setupView };

const H = 3_600_000;

/** Nothing to fix anywhere. */
export function sources(over: Partial<FixSources> = {}): FixSources {
  return {
    desktop: true,
    now: Date.now(),
    permissions: [],
    machines: null,
    setup: null,
    accounts: [],
    sync: [],
    kept: [],
    jobs: [],
    update: null,
    dismissed: new Set(),
    ...over,
  };
}

/** A project kept in sync with DESKTOP-PC, in sync. */
export function syncPair(over: Partial<SyncPair> = {}): SyncPair {
  return {
    id: 'sy_0123456789abcdef',
    machine: PEER_ID,
    machineName: 'DESKTOP-PC',
    machineOs: 'windows',
    online: true,
    folder: '/Users/you/dev/tray-tool',
    name: 'tray-tool',
    there: 'D:\\dev\\tray-tool',
    here: { on: true },
    peer: { on: true, asked: false },
    status: { state: 'inSync', at: Date.now() - 60_000 },
    last: null,
    conflict: null,
    kept: [],
    createdAt: Date.now() - 86_400_000,
    updatedAt: Date.now() - 60_000,
    ...over,
  };
}

/** …changed on both computers. */
export function conflicted(): SyncPair {
  const at = Date.now() - 5 * 60_000;
  const changes = { files: 2, headMoved: false, branchSwitched: false, branch: 'main', head: 'a1' };
  return syncPair({
    status: { state: 'paused', reason: 'conflict', detail: 'Changed on both.', since: at },
    conflict: { at, first: true, here: changes, there: changes },
  });
}

/** A copy of this computer's session kept on DESKTOP-PC, paused. */
export function keptCopy(reason: string, over: Partial<KeptCopy> = {}): KeptCopy {
  return {
    id: `kept.${PEER_ID}.s1`,
    machine: PEER_ID,
    machineName: 'DESKTOP-PC',
    online: true,
    session: 's1',
    title: 'Release notes',
    from: 'session',
    status: {
      state: 'paused',
      reason: reason as NonNullable<KeptCopy['status']>['reason'],
      since: Date.now() - 60_000,
      bytes: 0,
      files: 0,
    },
    ...over,
  };
}

/** A data job (a backup of Work, failed an hour ago, by default). */
export function job(over: Partial<JobView> = {}): JobView {
  return {
    id: 'job_1',
    kind: 'backup',
    state: 'failed',
    startedAt: Date.now() - H - 60_000,
    finishedAt: Date.now() - H,
    filesDone: 0,
    filesTotal: 0,
    bytesDone: 0,
    bytesTotal: 0,
    sessions: [],
    account: 'acc_5f1c9e2b7a40',
    error: 'the backup folder was offline',
    ...over,
  };
}

/** What DESKTOP-PC asked this computer (to sign Work in, by default). */
export function machineAsk(over: Partial<MachineAsk> = {}): MachineAsk {
  return {
    id: 'ask_0000000000000001',
    from: PEER_ID,
    fromName: 'DESKTOP-PC',
    kind: 'signIn',
    account: 'Work',
    at: Date.now() - 2 * 60_000,
    state: 'asked',
    ...over,
  };
}

/** A device waiting to pair. */
export function pairRequest(over: Partial<PendingPair> = {}): PendingPair {
  return {
    id: 'pp_1',
    deviceName: "Marcus's iPhone",
    platform: 'iOS',
    ingress: 'lan',
    status: 'pending',
    createdAt: Date.now() - 30_000,
    expiresAt: Date.now() + 4 * 60_000,
    ...over,
  };
}

/** A macOS permission (Full Disk Access, missing by default). */
export function permission(over: Partial<PermissionInfo> = {}): PermissionInfo {
  return {
    id: 'full-disk-access',
    label: 'Full Disk Access',
    description: 'Lets Mother Claude read your Claude Code sessions.',
    granted: false,
    required: true,
    settingsUrl: 'x-apple.systempreferences:com.apple.preference.security',
    steps: [],
    ...over,
  };
}

/** Machines on with DESKTOP-PC, a required check failing here and one
 * there. */
export function failingSetup() {
  return {
    view: machinesView({
      enabled: true,
      peers: [peer()],
      health: { state: 'blocker', blockers: 2, warnings: 0, at: Date.now() },
    }),
    setup: setupView({
      self: {
        machine: '0'.repeat(32),
        name: 'MacBook',
        at: Date.now(),
        checks: [
          check({
            id: 'keepAwake',
            title: 'Stays awake while used',
            state: 'blocker',
            detail: 'It sleeps after 10 minutes.',
            fix: { kind: 'auto', label: 'Keep awake while used' },
          }),
        ],
        error: null,
      },
      peers: [
        {
          machine: PEER_ID,
          name: 'DESKTOP-PC',
          at: Date.now(),
          checks: [
            check({
              id: 'firewall',
              title: 'The firewall',
              state: 'blocker',
              detail: 'No rule lets your other computers reach it.',
              fix: { kind: 'admin', label: 'Allow through the firewall' },
            }),
          ],
          error: null,
        },
      ],
      blockers: 2,
    }),
  };
}
