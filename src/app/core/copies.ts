import { tr } from './i18n/i18n';

/**
 * Copies kept current on other computers (docs/PLAN-MACHINES.md §5.9,
 * Phase 3b): a session marked *Keep a copy on DESKTOP-PC* — itself, or every
 * session of its project — is copied there after each turn, so a move is
 * instant and the other computer can go on with it while this one sleeps.
 */

/** Where one copy this computer keeps elsewhere stands. */
export type CopyState = 'pending' | 'waiting' | 'copying' | 'copied' | 'paused';

/** Why a copy waits. */
export type CopyReason =
  | 'offline'
  | 'machinesOff'
  | 'noGrant'
  | 'unsupported'
  | 'livesElsewhere'
  | 'tooLarge'
  | 'changed'
  | 'error';

export interface CopyStatus {
  state: CopyState;
  reason?: CopyReason | null;
  /** What the server says, in words. */
  detail?: string | null;
  /** Since when it is in this state (ms). */
  since: number;
  /** When the copy there was last made current (ms). */
  copiedAt?: number | null;
  /** The time of the turn it holds (ms). */
  turnAt?: number | null;
  bytes: number;
  files: number;
}

/** One paired computer, for a session's *Keep a copy on …*. */
export interface SessionCopyMachine {
  id: string;
  name: string;
  os: string;
  online: boolean;
  /** The session keeps a copy there. */
  on: boolean;
  /** Its own choice, or its project's (null when off). */
  from: 'session' | 'project' | null;
  /** Its project keeps copies there. */
  inProject: boolean;
  /** That computer's Mother Claude keeps copies. */
  supported: boolean;
  status: CopyStatus | null;
}

/** `GET /api/sessions/{id}/copies`. */
export interface SessionCopiesView {
  session: string;
  /** The session's own choice (else its project's). */
  explicit: boolean;
  /** The computers its project keeps copies on. */
  project: string[];
  /** The computer it lives on, when not this one. */
  livesOn: string | null;
  enabled: boolean;
  machines: SessionCopyMachine[];
}

/** A copy held here, of another computer's session. */
export interface HeldCopy {
  /** `held.<computer>.<session>`. */
  id: string;
  machine: string;
  machineName: string;
  online: boolean;
  session: string;
  title: string | null;
  cwd: string;
  account: string;
  bytes: number;
  files: number;
  copiedAt: number | null;
  turnAt: number | null;
  createdAt: number;
  /** The last round went through whole. */
  complete: boolean;
  /** The session lives on this computer now. */
  livesHere: boolean;
  /** Its home is offline: it can go on here from this copy. */
  canContinue: boolean;
}

/** A copy this computer keeps on another computer. */
export interface KeptCopy {
  /** `kept.<computer>.<session>`. */
  id: string;
  machine: string;
  machineName: string;
  online: boolean;
  session: string;
  title: string | null;
  from: 'session' | 'project';
  status: CopyStatus | null;
}

/** `GET /api/machines/copies`. */
export interface CopiesView {
  enabled: boolean;
  held: HeldCopy[];
  kept: KeptCopy[];
  /** What this computer keeps at most of each other computer's copies. */
  capBytes: number;
  /** Computer → the bytes its copies hold here. */
  used: Record<string, number>;
}

/** The answer of a take over (`POST /api/sessions/{id}/move`, `takeOver`). */
export interface TakeOverFromCopy {
  fromCopy?: boolean;
  copiedAt?: number | null;
  resumed?: boolean;
  resumeError?: string | null;
  warnings?: string[];
  cwd?: string | null;
}

/** `14:32` today, else `12 Nov, 14:32`. */
export function copyClock(ms: number | null | undefined, now = Date.now()): string {
  if (!ms) return '';
  const d = new Date(ms);
  const time = d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  const today = new Date(now);
  if (d.toDateString() === today.toDateString()) return time;
  return `${d.toLocaleDateString([], { day: 'numeric', month: 'short' })}, ${time}`;
}

/** A size as people read it. */
export function copySize(bytes: number): string {
  if (bytes < 1000) return tr('{n} bytes', { n: bytes });
  const units = ['KB', 'MB', 'GB', 'TB'];
  let v = bytes / 1000;
  let u = 0;
  while (v >= 1000 && u < units.length - 1) {
    v /= 1000;
    u += 1;
  }
  return `${v.toFixed(v < 10 ? 1 : 0)} ${units[u]}`;
}

/** How a copy stands, in a line: *Copied at 14:32*, *Paused: why*. */
export function copyStateText(s: CopyStatus | null | undefined): string {
  if (!s) return tr('Not copied yet');
  switch (s.state) {
    case 'copied':
      return s.copiedAt ? tr('Copied at {time}', { time: copyClock(s.copiedAt) }) : tr('Copied');
    case 'copying':
      return tr('Copying…');
    case 'waiting':
      return s.copiedAt
        ? tr('Copied at {time} · copies again once its turn ends', {
            time: copyClock(s.copiedAt),
          })
        : tr('Copies once its turn ends');
    case 'paused':
      return tr('Paused: {why}', { why: copyReasonText(s) });
    default:
      return tr('Not copied yet');
  }
}

/** Why a copy is paused, in words. */
export function copyReasonText(s: CopyStatus): string {
  switch (s.reason) {
    case 'offline':
      return tr('the other computer can’t be reached');
    case 'machinesOff':
      return tr('Computers is off on one of the computers');
    case 'noGrant':
      return tr('the other computer has not given this one the Move permission');
    case 'unsupported':
      return tr('Mother Claude on the other computer can’t keep copies yet');
    case 'livesElsewhere':
      return tr('the session lives on another computer now');
    case 'tooLarge':
      return tr('the other computer keeps less of this one’s copies');
    case 'changed':
      return tr('it changed while it was read; it goes again once quiet');
    default:
      return s.detail || tr('something went wrong');
  }
}

/** The state's tone, for a dot: ok, waiting, paused. */
export function copyTone(s: CopyStatus | null | undefined): 'ok' | 'wait' | 'off' {
  if (!s) return 'wait';
  if (s.state === 'copied') return 'ok';
  if (s.state === 'paused') return 'off';
  return 'wait';
}

/** The session's title, else the start of its id. */
export function copyTitle(c: { title: string | null; session: string }): string {
  return c.title?.trim() || c.session.slice(0, 8);
}
