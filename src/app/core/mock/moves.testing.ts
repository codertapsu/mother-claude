// Fixtures for the specs of moving sessions between computers
// (docs/PLAN-MACHINES.md §5.8): the server's records, preflights and checks,
// as `src-tauri/src/machines/moves/` sends them.
import { MoveCheck, MovePreflight, MoveRecord, MoveState } from '../moves';
import { PEER_ID, SELF_ID } from '../../pages/machines/machines.testing';

export { PEER_ID, SELF_ID };

export const MOVE_ID = 'mv_0123456789abcdef';

/** Every record made later is newer, as the server's writes are. */
let written = 0;
const later = () => Date.now() + ++written;

/** A move of `s1` from this computer (MacBook) to DESKTOP-PC, as it starts. */
export function moveRecord(over: Partial<MoveRecord> = {}): MoveRecord {
  const at = Date.now() - 5_000;
  return {
    id: MOVE_ID,
    role: 'out',
    session: 's1',
    targetSession: null,
    title: 'Fix the login',
    from: { id: SELF_ID, name: 'MacBook', os: 'macos' },
    to: { id: PEER_ID, name: 'DESKTOP-PC', os: 'windows' },
    state: 'preflight',
    finished: false,
    asCopy: false,
    keepDriving: true,
    when: 'idle',
    account: { name: 'Default', isDefault: true },
    epoch: 1,
    sourceCwd: '/Users/me/dev/api',
    targetCwd: null,
    targetFolder: null,
    code: {
      mode: 'bundle',
      apply: 'checkout',
      branch: 'refs/heads/feat/login',
      changed: 8,
      untracked: 2,
      warnings: [],
    },
    files: [],
    bytesTotal: 0,
    bytesSent: 0,
    conversationBytes: 18 * 1024 * 1024,
    wasRunning: false,
    heldCount: 0,
    recipe: [],
    trust: false,
    steps: [{ state: 'preflight', at }],
    error: null,
    warnings: [],
    landing: null,
    resumeError: null,
    by: 'desktop',
    createdAt: at,
    updatedAt: at,
    finishedAt: null,
    undoUntil: null,
    ...over,
  };
}

/** The same record further on: `state` reached (and every step before). */
export function movedTo(state: MoveState, over: Partial<MoveRecord> = {}): MoveRecord {
  const order: MoveState[] = [
    'preflight',
    'freezing',
    'packaging',
    'sending',
    'landing',
    'landed',
    'committed',
    'resumed',
  ];
  const base = moveRecord();
  const upTo = order.indexOf(state);
  const steps = (upTo < 0 ? [...order.slice(0, 4), state] : order.slice(0, upTo + 1)).map(
    (s, i) => ({ state: s, at: base.createdAt + i * 500 }),
  );
  const total = 20 * 1024 * 1024;
  const final =
    state === 'resumed' || state === 'failed' || state === 'cancelled' || state === 'undone';
  return {
    ...base,
    state,
    steps,
    files: [
      { name: 'session.tar.gz', size: 18 * 1024 * 1024, sha256: '0'.repeat(64), sent: 0 },
      { name: 'move.bundle', size: 2 * 1024 * 1024, sha256: '1'.repeat(64), sent: 0 },
    ],
    bytesTotal: total,
    bytesSent: upTo > 3 ? total : 0,
    wasRunning: true,
    finished: final,
    finishedAt: final ? base.createdAt + 4_000 : null,
    updatedAt: later(),
    ...over,
  };
}

export function moveCheck(code: string, message: string, over: Partial<MoveCheck> = {}): MoveCheck {
  return { code, message, ...over };
}

/** A preflight that found nothing in the way, to DESKTOP-PC. */
export function preflightOk(over: Partial<MovePreflight> = {}): MovePreflight {
  return {
    ok: true,
    blockers: [],
    warnings: [],
    session: {
      id: 's1',
      title: 'Fix the login',
      owned: true,
      live: true,
      foreign: false,
      account: { id: 'default', name: 'Default' },
      cwd: '/Users/me/dev/api',
      bytes: 18 * 1024 * 1024,
      transcriptBytes: 16 * 1024 * 1024,
      files: 12,
      subagents: 3,
    },
    to: { id: PEER_ID, name: 'DESKTOP-PC', os: 'windows', online: true },
    code: {
      mode: 'bundle',
      apply: 'checkout',
      repo: true,
      top: '/Users/me/dev/api',
      branch: 'refs/heads/feat/login',
      head: 'a'.repeat(40),
      changed: 8,
      untracked: 2,
      origin: 'git@github.com:me/api.git',
      relation: 'fastForward',
    },
    target: {
      account: { id: 'default', name: 'Default', signedIn: true },
      folder: { path: 'D:\\dev\\api', how: 'root', trusted: true },
      candidates: [],
      cwd: 'D:\\dev\\api',
      code: {
        repo: true,
        clean: true,
        parked: false,
        changed: 0,
        untracked: 0,
        origin: 'git@github.com:me/api.git',
        branch: 'refs/heads/main',
      },
      session: { here: false, live: false },
      claude: '2.1.322',
      freeBytes: 100 * 1024 * 1024 * 1024,
    },
    recipe: [],
    asCopy: false,
    ...over,
  };
}

/** A preflight with blockers (and so not ok). */
export function preflightBlocked(blockers: MoveCheck[], over: Partial<MovePreflight> = {}) {
  return preflightOk({ ok: false, blockers, ...over });
}
