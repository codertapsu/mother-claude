// Teams of Claude Code sessions (docs/TEAMS.md): the wire types of
// `/api/team-templates*`, `/api/teams*` and the `teams` WebSocket frame, and
// the pure helpers the Teams pages share. The label tables stay English and
// are translated where they are read (docs/I18N.md).

import { tr } from './i18n/i18n';

/** Where a task is on the board. */
export type TaskStatus =
  'todo' | 'claimed' | 'in_progress' | 'review' | 'done' | 'failed' | 'cancelled';

export const TASK_STATUSES: readonly TaskStatus[] = [
  'todo',
  'claimed',
  'in_progress',
  'review',
  'done',
  'failed',
  'cancelled',
];

export const STATUS_LABELS: Readonly<Record<TaskStatus, string>> = {
  todo: 'To do',
  claimed: 'Claimed',
  in_progress: 'In progress',
  review: 'Review',
  done: 'Done',
  failed: 'Failed',
  cancelled: 'Cancelled',
};

/** The task classes the model router knows. */
export const TASK_CLASSES = [
  'plan',
  'architecture',
  'review',
  'implement',
  'test',
  'fix',
  'search',
  'summarize',
  'classify',
  'ops',
  'other',
] as const;

export const PRIORITY_LABELS = ['P0 · urgent', 'P1 · high', 'P2 · normal', 'P3 · low'] as const;

/** A role's state on the Teams page. */
export type RoleState = 'notStarted' | 'waiting' | 'starting' | 'running' | 'idle' | 'stopped';

export const ROLE_STATE_LABELS: Readonly<Record<RoleState, string>> = {
  notStarted: 'Not started',
  waiting: 'Waiting',
  starting: 'Starting',
  running: 'Running',
  idle: 'Idle',
  stopped: 'Stopped',
};

/** The badge class of a role state. */
export const ROLE_STATE_TONE: Readonly<Record<RoleState, string>> = {
  notStarted: '',
  waiting: 'warn',
  starting: 'info',
  running: 'working',
  idle: 'ok',
  stopped: '',
};

export interface RoleDef {
  id: string;
  title: string;
  lead?: boolean;
  description?: string;
  charter?: string;
  model?: string;
  effort?: string;
  permissionMode?: string;
  worktree?: boolean;
  paths?: string[];
  readOnly?: boolean;
  budgetUsd?: number;
  autostart?: boolean;
  addDirs?: string[];
  tools?: string[];
  classes?: string[];
  [extra: string]: unknown;
}

export interface Integration {
  branch?: string;
  mergeBy?: string;
  gate?: string;
  review?: boolean;
  [extra: string]: unknown;
}

export interface TeamTemplate {
  id: string;
  name: string;
  description?: string;
  roles: RoleDef[];
  integration?: Integration;
  idleStopMinutes?: number;
  [extra: string]: unknown;
}

export interface TemplateIssue {
  path: string;
  message: string;
}

export interface TemplateReport {
  errors: TemplateIssue[];
  warnings: TemplateIssue[];
}

/** `GET /api/team-templates` rows. */
export interface TemplateEntry extends TeamTemplate {
  builtin: boolean;
  report: TemplateReport;
}

export interface ModelChoice {
  model: string;
  effort?: string;
}

export interface TaskNote {
  at: number;
  by: string;
  text: string;
}

export interface TaskHistoryEntry {
  at: number;
  by: string;
  action: string;
  detail?: string;
}

export interface TaskResult {
  summary: string;
  branch?: string;
  changedFiles?: string[];
  gatePassed?: boolean;
}

export interface TeamTask {
  id: number;
  title: string;
  goal: string;
  acceptance?: string[];
  role?: string;
  class?: string;
  priority: number;
  status: TaskStatus;
  files?: string[];
  constraints?: string[];
  claimedBy?: string;
  progress?: TaskNote[];
  blocked?: string;
  result?: TaskResult;
  history?: TaskHistoryEntry[];
  model?: ModelChoice;
  modelReason?: string;
  modelOverride?: ModelChoice;
  escalations: number;
  createdAt: number;
  updatedAt: number;
  createdBy: string;
  startedAt?: number;
  endedAt?: number;
}

export interface RoleView {
  id: string;
  title: string;
  lead: boolean;
  description?: string;
  state: RoleState;
  sessionId?: string;
  sessionName: string;
  foreign: boolean;
  model?: string;
  effort?: string;
  modelReason?: string;
  permissionMode: string;
  worktree: boolean;
  worktreePath?: string;
  paths: string[];
  readOnly: boolean;
  budgetUsd?: number;
  budgetApplied: boolean;
  autostart: boolean;
  classes: string[];
  waitReasons?: string[];
  waitingSince?: number;
  currentTask?: number;
  queuedTasks: number;
  lastError?: string;
  guardDenials: number;
  startedAt?: number;
  stoppedAt?: number;
}

export interface TeamView {
  id: string;
  project: string;
  label: string;
  templateId: string;
  templateName: string;
  account: string;
  stopped: boolean;
  createdAt: number;
  integration: Integration;
  idleStopMinutes: number;
  reviews: boolean;
  roles: RoleView[];
  counts: Partial<Record<TaskStatus, number>>;
  brief?: string;
}

/** Per-role changes a team starts with. */
export interface RoleOverride {
  model?: string;
  effort?: string;
  permissionMode?: string;
  budgetUsd?: number;
  autostart?: boolean;
}

export interface StartTeamBody {
  project: string;
  template: string;
  account?: string;
  overrides?: Record<string, RoleOverride>;
  brief?: string;
}

export interface TaskDraft {
  title: string;
  goal: string;
  acceptance?: string[];
  role?: string;
  class?: string;
  priority?: number;
  files?: string[];
  constraints?: string[];
}

export interface TaskPatch {
  status?: TaskStatus;
  note?: string;
  blocked?: string;
  priority?: number;
  role?: string;
  title?: string;
  goal?: string;
  escalate?: boolean;
}

/** `POST /api/teams/{id}/roles/{role}/start`. */
export interface RoleStartResult {
  state: 'started' | 'waiting';
  sessionId?: string;
  reasons?: string[];
}

/** An agent-team event an interactive session's hooks reported. */
export interface ObservedEvent {
  at: number;
  sessionId: string;
  event: string;
  teammate?: string;
  team?: string;
  taskId?: string;
  subject?: string;
  fromModel?: string;
  toModel?: string;
  source?: string;
}

/** `{t:"teams"}`: a team's view, a task of a board, a removed team, or "the
 * templates changed". */
export interface TeamsFrame {
  t: 'teams';
  team?: TeamView;
  teamId?: string;
  task?: TeamTask;
  removed?: string;
  templates?: boolean;
}

/** Permission modes that run tools without asking: starting a role in one
 * needs the dangerous right. */
export const DANGEROUS_MODES = ['bypassPermissions', 'dontAsk'];

// ───────────────────────────────────────────────────────────── helpers

/** Tasks by status, each column by priority, then oldest first. */
export function columns(tasks: readonly TeamTask[]): Record<TaskStatus, TeamTask[]> {
  const out = Object.fromEntries(TASK_STATUSES.map((s) => [s, [] as TeamTask[]])) as Record<
    TaskStatus,
    TeamTask[]
  >;
  for (const t of tasks) {
    (out[t.status] ?? out.todo).push(t);
  }
  for (const s of TASK_STATUSES) {
    out[s].sort((a, b) => a.priority - b.priority || a.id - b.id);
  }
  return out;
}

/** A task list with `task` added or replaced. */
export function upsertTask(tasks: readonly TeamTask[], task: TeamTask): TeamTask[] {
  const i = tasks.findIndex((t) => t.id === task.id);
  if (i < 0) return [task, ...tasks];
  const next = tasks.slice();
  next[i] = task;
  return next;
}

/** A team list with `team` added or replaced, newest first. */
export function upsertTeam(teams: readonly TeamView[], team: TeamView): TeamView[] {
  const next = teams.filter((t) => t.id !== team.id);
  next.push(team);
  return next.sort((a, b) => b.createdAt - a.createdAt);
}

/** The roles of a template after a start's overrides. */
export function applyOverrides(
  roles: readonly RoleDef[],
  overrides: Readonly<Record<string, RoleOverride>>,
): RoleDef[] {
  return roles.map((r) => {
    const o = overrides[r.id];
    if (!o) return r;
    return {
      ...r,
      ...(o.model ? { model: o.model } : {}),
      ...(o.effort ? { effort: o.effort } : {}),
      ...(o.permissionMode ? { permissionMode: o.permissionMode } : {}),
      ...(o.budgetUsd !== undefined ? { budgetUsd: o.budgetUsd } : {}),
      ...(o.autostart !== undefined ? { autostart: o.autostart } : {}),
    };
  });
}

/** Whether starting these roles needs the dangerous right. */
export function needsDangerous(roles: readonly RoleDef[]): boolean {
  return roles.some((r) => DANGEROUS_MODES.includes(r.permissionMode ?? 'default'));
}

/** Parse a template typed as JSON: the template, or why it is not one. */
export function parseTemplate(text: string): { template?: TeamTemplate; error?: string } {
  let v: unknown;
  try {
    v = JSON.parse(text);
  } catch (e) {
    return {
      error: tr('Not JSON: {error}', { error: e instanceof Error ? e.message : String(e) }),
    };
  }
  if (!v || typeof v !== 'object' || Array.isArray(v)) {
    return { error: tr('A template is a JSON object.') };
  }
  const t = v as Partial<TeamTemplate>;
  if (typeof t.id !== 'string' || !Array.isArray(t.roles)) {
    return { error: tr('A template has an "id" and a list of "roles".') };
  }
  return { template: v as TeamTemplate };
}

/** A template as the editor shows it (the list's extra fields left out). */
export function templateJson(t: TeamTemplate | TemplateEntry): string {
  const copy: Record<string, unknown> = { ...t };
  delete copy['builtin'];
  delete copy['report'];
  return JSON.stringify(copy, null, 2);
}

/** One line about a role: its state and, while it waits, why. */
export function roleStatusLine(r: RoleView): string {
  const known = ROLE_STATE_LABELS[r.state];
  const label = known ? tr(known) : r.state;
  if (r.state === 'waiting' && r.waitReasons?.length) {
    return `${label}: ${r.waitReasons.join('; ')}`;
  }
  if (r.lastError && (r.state === 'stopped' || r.state === 'notStarted')) {
    return `${label} · ${r.lastError}`;
  }
  return label;
}

/** Lines of a textarea as a list (empty lines dropped). */
export function lines(text: string): string[] {
  return text
    .split('\n')
    .map((l) => l.trim())
    .filter((l) => l.length > 0);
}

/** Open tasks (not done, failed or cancelled). */
export function openCount(team: TeamView): number {
  const c = team.counts;
  return (c.todo ?? 0) + (c.claimed ?? 0) + (c.in_progress ?? 0) + (c.review ?? 0);
}
