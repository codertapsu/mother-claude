// Tasks handed to another computer (docs/PLAN-MACHINES.md Phase 4, decision
// 6): a session hands a task to a Claude Code session on another of your
// computers, in its own worktree there at its own code; what it did comes
// back as the branch `mother-claude/<computer>/<task>`. The wire types of
// `/api/machines/tasks*`, the approval's `input` (a pending item whose
// source is `machine` and whose input's kind is `task`), and the helpers the
// screens share. Labels are translated where they are made (`tr`).

import { tr } from './i18n/i18n';
import type { RunDirection } from './runs';

/** Where a task is (`machines::work::tasks::TaskState`). */
export type TaskState =
  | 'sending'
  | 'starting'
  | 'running'
  | 'waiting'
  | 'finishing'
  | 'done'
  | 'stopped'
  | 'cancelled'
  | 'error'
  | 'lost';

/** A task, as either computer keeps it. */
export interface MachineTask {
  id: string;
  /** `out`: this computer handed it over; `in`: another handed it here. */
  direction: RunDirection;
  /** The other computer. */
  machine: string;
  machineName: string;
  /** The OS of the computer that runs it. */
  os: string;
  /** The asking session (on the asking computer). */
  session: string | null;
  sessionTitle: string | null;
  /** The project's top on the asking computer. */
  project: string;
  projectName: string;
  cwd: string;
  /** The checkout there, and the task's own worktree of it. */
  checkout: string;
  folder: string;
  /** The task: its first 500 characters in a list, whole in
   * `GET …/tasks/{id}`. */
  task: string;
  firstLine: string;
  permissionMode: string;
  model: string | null;
  effort: string | null;
  /** The account its session runs under there. */
  accountName: string | null;
  /** Its session, on the computer that runs it. */
  taskSession: string | null;
  state: TaskState;
  error: string | null;
  createdAt: number;
  startedAt: number | null;
  endedAt: number | null;
  turns: number;
  /** Its session's last message. */
  summary: string | null;
  /** Its session had background work running at its last end. */
  background: boolean;
  /** The handed-over code (the branch's base). */
  base: string | null;
  /** The asking side: the branch (`mother-claude/…`) once made. */
  branchName: string | null;
  tip: string | null;
  commits: number;
  files: number;
  uncommitted: boolean;
  /** Results made there, and taken here. */
  results: number;
  landed: number;
  /** Why the last result did not move the branch. */
  branchNote: string | null;
  approvedBy: string | null;
  /** Its session runs without approvals. */
  dangerous: boolean;
  /** The asking side: the other computer did not answer the last time. */
  unreachable: boolean;
  /** The asking side: the other computer removed it. */
  goneThere: boolean;
  /** Its session was stopped after it sat idle (a message resumes it). */
  idleStoppedAt?: number | null;
  updatedAt: number;
}

/** `GET /api/machines/tasks`. */
export interface TasksView {
  tasks: MachineTask[];
}

/** `DELETE /api/machines/tasks/{id}`. */
export interface TaskRemoved {
  removed: boolean;
  /** The asking side: what became of it there. */
  there?: 'removed' | 'gone' | 'unreachable';
}

/** A task approval's `input` (`machines::work::tasks::source::approve`). */
export interface MachineTaskInput {
  machine: string;
  machineName: string;
  os: string;
  /** The project's folder there (the task gets its own worktree of it). */
  checkout: string;
  cwd: string;
  task: string;
  firstLine: string;
  permissionMode: string;
  model: string | null;
  effort: string | null;
  /** The account its session would run under there. */
  account: string;
  project: string;
  projectName: string;
}

const str = (v: unknown): string => (typeof v === 'string' ? v : '');

/** A task approval's input, read tolerantly (null: not one). */
export function machineTaskInput(input: unknown): MachineTaskInput | null {
  if (!input || typeof input !== 'object') return null;
  const o = input as Record<string, unknown>;
  const task = str(o['task']);
  if (o['kind'] !== 'task' || !task) return null;
  return {
    machine: str(o['machine']),
    machineName: str(o['machineName']) || str(o['machine']).slice(0, 8),
    os: str(o['os']),
    checkout: str(o['checkout']),
    cwd: str(o['cwd']),
    task,
    firstLine: str(o['firstLine']),
    permissionMode: str(o['permissionMode']) || 'default',
    model: str(o['model']) || null,
    effort: str(o['effort']) || null,
    account: str(o['account']),
    project: str(o['project']),
    projectName: str(o['projectName']),
  };
}

/** Whether a task is over for now (a done one runs again when its session
 * is sent another message). */
export function taskEnded(s: TaskState): boolean {
  return ['done', 'stopped', 'cancelled', 'error', 'lost'].includes(s);
}

/** A task's state in words. */
export function taskStateLabel(s: TaskState): string {
  switch (s) {
    case 'sending':
      return tr('Sending the code');
    case 'starting':
      return tr('Starting its session');
    case 'running':
      return tr('Working');
    case 'waiting':
      return tr('Waiting for an answer');
    case 'finishing':
      return tr('Bringing its work back');
    case 'done':
      return tr('Done');
    case 'stopped':
      return tr('Stopped');
    case 'cancelled':
      return tr('Cancelled');
    case 'error':
      return tr('Could not start');
    case 'lost':
      return tr('Lost track');
  }
}

/** A task's chip tone. */
export function taskTone(s: TaskState): 'green' | 'red' | 'amber' | 'blue' | 'muted' {
  switch (s) {
    case 'done':
      return 'green';
    case 'error':
    case 'lost':
      return 'red';
    case 'waiting':
    case 'stopped':
    case 'cancelled':
      return 'amber';
    case 'sending':
    case 'starting':
    case 'running':
    case 'finishing':
      return 'blue';
  }
}

/** What a task's branch holds: "2 commits and uncommitted changes · 3
 * files". */
export function taskChanges(t: Pick<MachineTask, 'commits' | 'files' | 'uncommitted'>): string {
  const commits = t.commits === 1 ? tr('1 commit') : tr('{n} commits', { n: t.commits });
  const files = t.files === 1 ? tr('1 file') : tr('{n} files', { n: t.files });
  return t.uncommitted
    ? tr('{commits} and uncommitted changes · {files}', { commits, files })
    : `${commits} · ${files}`;
}
