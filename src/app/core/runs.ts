// Runs on another computer (docs/PLAN-MACHINES.md Phase 4): a session runs a
// command on another of your computers at its own code. The wire types of
// `/api/machines/runs*`, the approval's `input` (a pending item whose source
// is `machine`), and the helpers the screens share. The commands allowed
// without asking are permissions (`core/permission-rules.ts`). Labels are
// translated where they are made (`tr`).

import { tr } from './i18n/i18n';

/** Where a run is (`machines::work::RunState`). */
export type RunState =
  | 'sending'
  | 'queued'
  | 'preparing'
  | 'running'
  | 'done'
  | 'failed'
  | 'cancelled'
  | 'timedOut'
  | 'error'
  | 'lost';

/** `out`: this computer asked another; `in`: another asked this one. */
export type RunDirection = 'out' | 'in';

export interface RunArtifact {
  /** Relative to the project's top, with `/`. */
  path: string;
  size: number;
}

export interface FetchedArtifact {
  path: string;
  /** Where it is now, in the asking session's project. */
  to: string;
  size: number;
  at: number;
}

/** A run, as either computer keeps it. */
export interface MachineRun {
  id: string;
  direction: RunDirection;
  /** The other computer. */
  machine: string;
  machineName: string;
  /** The OS and shell it runs in (the computer that runs it). */
  os: string;
  shell: string;
  session: string | null;
  sessionTitle: string | null;
  /** The project's top on the asking computer. */
  project: string;
  projectName: string;
  work: string;
  /** The work folder, and the checkout it is a worktree of. */
  folder: string;
  checkout: string;
  /** Where it runs, relative to the project's top (`''`: the top). */
  cwd: string;
  command: string;
  program: string;
  commandSha256: string;
  dangerous: boolean;
  timeoutSecs: number;
  artifactsGlob: string | null;
  state: RunState;
  error: string | null;
  exitCode: number | null;
  createdAt: number;
  startedAt: number | null;
  endedAt: number | null;
  durationMs: number | null;
  outputSize: number;
  truncated: boolean;
  /** The output's end: 2 KiB in a list, 64 KiB in `GET …/runs/{id}`. */
  tail: string;
  artifacts: RunArtifact[];
  fetched: FetchedArtifact[];
  approvedBy: string | null;
  /** The permission it ran under, when one allowed it without asking. */
  grant: string | null;
  /** The asking side: the other computer did not answer the last time. */
  unreachable: boolean;
  updatedAt: number;
}

/** `GET /api/machines/runs`. */
export interface RunsView {
  runs: MachineRun[];
}

/** A run approval's `input` (`machines::work::source::approve`). */
export interface MachineRunInput {
  machine: string;
  machineName: string;
  os: string;
  shell: string;
  shellLabel: string;
  /** The project's folder there, and the work folder it runs in. */
  checkout: string;
  folder: string;
  cwd: string;
  command: string;
  timeoutSecs: number | null;
  artifacts: string | null;
  project: string;
  projectName: string;
}

const str = (v: unknown): string => (typeof v === 'string' ? v : '');

/** A run approval's input, read tolerantly (null: not one). */
export function machineRunInput(input: unknown): MachineRunInput | null {
  if (!input || typeof input !== 'object') return null;
  const o = input as Record<string, unknown>;
  const command = str(o['command']);
  if (!command) return null;
  const timeout = o['timeoutSecs'];
  return {
    machine: str(o['machine']),
    machineName: str(o['machineName']) || str(o['machine']).slice(0, 8),
    os: str(o['os']),
    shell: str(o['shell']),
    shellLabel: str(o['shellLabel']) || str(o['shell']),
    checkout: str(o['checkout']),
    folder: str(o['folder']),
    cwd: str(o['cwd']),
    command,
    timeoutSecs: typeof timeout === 'number' && Number.isFinite(timeout) ? timeout : null,
    artifacts: str(o['artifacts']) || null,
    project: str(o['project']),
    projectName: str(o['projectName']),
  };
}

/** Whether a run is over. */
export function runEnded(s: RunState): boolean {
  return !['sending', 'queued', 'preparing', 'running'].includes(s);
}

/** A run's state in words. */
export function runStateLabel(s: RunState): string {
  switch (s) {
    case 'sending':
      return tr('Sending the code');
    case 'queued':
      return tr('Waiting for its folder');
    case 'preparing':
      return tr('Setting up its folder');
    case 'running':
      return tr('Running');
    case 'done':
      return tr('Done');
    case 'failed':
      return tr('Failed');
    case 'cancelled':
      return tr('Cancelled');
    case 'timedOut':
      return tr('Ran out of time');
    case 'error':
      return tr('Could not run');
    case 'lost':
      return tr('Lost track');
  }
}

/** A run's chip tone. */
export function runTone(s: RunState): 'green' | 'red' | 'amber' | 'blue' | 'muted' {
  switch (s) {
    case 'done':
      return 'green';
    case 'failed':
    case 'error':
    case 'lost':
      return 'red';
    case 'cancelled':
    case 'timedOut':
      return 'amber';
    case 'sending':
    case 'queued':
    case 'preparing':
    case 'running':
      return 'blue';
  }
}

/** A shell's name as people know it. */
export function shellName(shell: string): string {
  switch (shell) {
    case 'pwsh':
      return 'PowerShell 7';
    case 'powershell':
      return 'Windows PowerShell';
    default:
      return shell;
  }
}

/** A time limit: `30 min`, `2 h`, `45 s`. */
export function timeLimitText(secs: number | null | undefined): string {
  if (!secs || secs <= 0) return '';
  if (secs % 3600 === 0) return tr('{n} h', { n: secs / 3600 });
  if (secs >= 60 && secs % 60 === 0) return tr('{n} min', { n: secs / 60 });
  return tr('{n} s', { n: secs });
}
