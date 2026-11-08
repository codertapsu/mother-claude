import {
  Activity,
  ActivityKind,
  ControlChannel,
  FileChange,
  PermissionMode,
  PlanSeverity,
  SessionStatus,
  SessionSummary,
  Surface,
  ThinkingState,
  Usage,
} from '../core/models';
import { locale, tr } from '../core/i18n/i18n';
import { tildePath } from '../core/paths';

/** Tones map to the theme's status colours (see styles.scss). */
export type Tone = 'green' | 'amber' | 'red' | 'blue' | 'accent' | 'muted' | 'faint';

// ───────────────────────────────────────────────────────────────── Time

export function relativeTime(ms?: number, now = Date.now()): string {
  if (!ms) return '—';
  const diff = now - ms;
  if (diff < 5_000) return tr('just now');
  const s = Math.floor(diff / 1000);
  if (s < 60) return tr('{n}s ago', { n: s });
  const m = Math.floor(s / 60);
  if (m < 60) return tr('{n}m ago', { n: m });
  const h = Math.floor(m / 60);
  if (h < 24) return tr('{n}h ago', { n: h });
  const d = Math.floor(h / 24);
  if (d < 30) return tr('{n}d ago', { n: d });
  return new Date(ms).toLocaleDateString(locale());
}

/** Compact duration: 12s · 3m 04s · 1h 02m · 2d 3h. */
export function formatDuration(ms?: number): string {
  if (ms === undefined || ms === null || !Number.isFinite(ms) || ms < 0) return '';
  const s = Math.floor(ms / 1000);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${String(s % 60).padStart(2, '0')}s`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ${String(m % 60).padStart(2, '0')}m`;
  return `${Math.floor(h / 24)}d ${h % 24}h`;
}

/** Time left, rounded up to the minute: 45m · 2h 10m · 3d 4h · <1m. */
export function formatCountdown(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return '0m';
  const mins = Math.ceil(ms / 60_000);
  if (ms < 60_000) return '<1m';
  if (mins < 60) return `${mins}m`;
  const h = Math.floor(mins / 60);
  if (h < 24) return mins % 60 ? `${h}h ${mins % 60}m` : `${h}h`;
  const d = Math.floor(h / 24);
  return h % 24 ? `${d}d ${h % 24}h` : `${d}d`;
}

/** "resets in 2h 10m" · "reset" (the time has passed) · '' (unknown). */
export function resetsIn(resetsAt: number | undefined, now = Date.now()): string {
  if (!resetsAt) return '';
  if (resetsAt <= now) return tr('reset');
  return tr('resets in {time}', { time: formatCountdown(resetsAt - now) });
}

/** When a scheduled session resumes, after its "Scheduled to resume":
 * "in 12m" · "due now" (the time has come; a fire can be a little late) ·
 * '' (unknown). */
export function resumesIn(resumesAt: number | undefined, now = Date.now()): string {
  if (!resumesAt) return '';
  if (resumesAt <= now) return tr('due now');
  return tr('in {time}', { time: formatCountdown(resumesAt - now) });
}

/** A reset time people can place: "14:30" today, "Tue 14:30" this week,
 * else "3 Oct, 14:30". */
export function resetClock(resetsAt: number | undefined, now = Date.now()): string {
  if (!resetsAt) return '';
  const d = new Date(resetsAt);
  const time = d.toLocaleTimeString(locale(), { hour: '2-digit', minute: '2-digit' });
  const today = new Date(now);
  if (d.toDateString() === today.toDateString()) return time;
  if (resetsAt - now < 6 * 24 * 3_600_000) {
    return `${d.toLocaleDateString(locale(), { weekday: 'short' })} ${time}`;
  }
  return `${d.toLocaleDateString(locale(), { day: 'numeric', month: 'short' })}, ${time}`;
}

// ─────────────────────────────────────────────────────────────── Numbers

export function formatTokens(n?: number): string {
  if (!n) return '0';
  if (n < 1000) return `${n}`;
  if (n < 1_000_000) return `${(n / 1000).toFixed(n < 10_000 ? 1 : 0)}k`;
  return `${(n / 1_000_000).toFixed(2)}M`;
}

export function formatUsd(n?: number): string {
  if (n === undefined || n === null || !Number.isFinite(n)) return '—';
  if (n > 0 && n < 0.01) return '<$0.01';
  if (n < 100) return `$${n.toFixed(2)}`;
  return `$${Math.round(n).toLocaleString()}`;
}

/** An amount in minor units (cents) as money: `formatMinor(1240, 'USD', 2)`
 * → "$12.40"; a plain "12.40" when the currency isn't known. */
export function formatMinor(minor: number, currency: string | undefined, decimals: number): string {
  const value = minor / 10 ** decimals;
  const digits = { minimumFractionDigits: decimals, maximumFractionDigits: decimals };
  try {
    return currency
      ? new Intl.NumberFormat(undefined, { style: 'currency', currency, ...digits }).format(value)
      : new Intl.NumberFormat(undefined, digits).format(value);
  } catch {
    return currency ? `${value.toFixed(decimals)} ${currency}` : value.toFixed(decimals);
  }
}

// ──────────────────────────────────────────────────────────────── Labels

const STATUS: Record<SessionStatus, { label: string; tone: Tone }> = {
  working: { label: 'Working', tone: 'green' },
  waiting: { label: 'Needs you', tone: 'amber' },
  background: { label: 'Background work', tone: 'accent' },
  idle: { label: 'Idle', tone: 'muted' },
  starting: { label: 'Starting', tone: 'blue' },
  ended: { label: 'Ended', tone: 'faint' },
  failed: { label: 'Failed', tone: 'red' },
  stopped: { label: 'Stopped', tone: 'faint' },
};

/** A status in words. A session that resumes by itself says what wakes it:
 * "Background work" (it still runs), or "Scheduled" (a wakeup or a job;
 * pass its activity). Never the bare "Background": the surface chip says
 * that of `--bg` jobs. */
export function statusLabel(
  status: SessionStatus | string,
  activity?: Pick<Activity, 'kind'>,
): string {
  if (status === 'background' && activity?.kind === 'scheduled') return tr('Scheduled');
  const label = STATUS[status as SessionStatus]?.label;
  return label ? tr(label) : capitalize(String(status || 'unknown'));
}

export function statusTone(status: SessionStatus | string): Tone {
  return STATUS[status as SessionStatus]?.tone ?? 'muted';
}

/** Names, and the words translated where they are read (`surfaceLabel`). */
const SURFACES: Record<Surface, string> = {
  vscode: 'VS Code',
  terminal: 'Terminal',
  tmux: 'tmux',
  desktop: 'Claude Desktop',
  jetbrains: 'JetBrains',
  // A `--bg` job ("Background" alone is the theme's background).
  background: 'Background||surface',
  'mother-claude': 'Mother Claude',
  sdk: 'SDK',
  unknown: 'Unknown',
};

export function surfaceLabel(surface: Surface | string | undefined): string {
  const label = SURFACES[surface as Surface];
  return label ? tr(label) : capitalize(String(surface || 'unknown'));
}

/** What stops a session's process, as a phrase ("closing VS Code"): the
 * app that runs it. '' for a tmux pane or a `--bg` job, which outlive a
 * closed window. */
export function stoppedBy(s: Pick<SessionSummary, 'surface' | 'owned'>): string {
  if (s.owned) return tr('quitting Mother Claude');
  switch (s.surface) {
    case 'vscode':
      return tr('closing VS Code');
    case 'jetbrains':
      return tr('closing JetBrains');
    case 'terminal':
      return tr('closing its terminal');
    case 'desktop':
      return tr('quitting Claude Desktop');
    case 'mother-claude':
      return tr('quitting Mother Claude');
    case 'sdk':
      return tr('stopping the program that runs it');
    case 'tmux':
    case 'background':
      return '';
    default:
      return tr('closing the app that runs it');
  }
}

/** A session that resumes by itself, in a sentence for its page: what it
 * waits on, when, and whether closing the app that runs it stops that.
 * '' for any other session. */
export function resumeNote(
  s: Pick<SessionSummary, 'live' | 'status' | 'activity' | 'surface' | 'owned'>,
  now = Date.now(),
): string {
  if (!s.live || s.status !== 'background') return '';
  let lead = tr('Claude picks this up when its background work finishes');
  if (s.activity?.kind === 'scheduled') {
    const at = s.activity.resumesAt;
    lead = !at
      ? tr('Claude resumes this by itself on its schedule')
      : at <= now
        ? tr('Claude resumes this by itself any moment')
        : tr('Claude resumes this by itself in {time}', { time: formatCountdown(at - now) });
  }
  const stop = stoppedBy(s);
  if (stop) return tr('{lead} — {stop} now stops it.', { lead, stop });
  return s.surface === 'tmux'
    ? tr("{lead}. It runs in tmux, so closing a window doesn't stop it.", { lead })
    : tr("{lead}. It runs as a background job, so closing a window doesn't stop it.", { lead });
}

const CHANNELS: Record<ControlChannel, { label: string; tone: Tone; hint: string }> = {
  owned: {
    label: 'Full control',
    tone: 'accent',
    hint: 'Started by Mother Claude: full, reliable control from here.',
  },
  hooks: {
    label: 'Via hooks',
    tone: 'blue',
    hint: 'Reached through the Mother Claude hooks plugin: approvals work, messages arrive at the next stopping point.',
  },
  tmux: {
    label: 'Via tmux',
    tone: 'blue',
    hint: 'Driven by typing into its tmux pane.',
  },
  pty: {
    label: 'Experimental',
    tone: 'amber',
    hint: 'Experimental terminal injection: may not work reliably.',
  },
  none: {
    label: 'Watch only',
    tone: 'muted',
    hint: 'You can watch this session from here, not drive it.',
  },
};

export function channelLabel(channel: ControlChannel | string | undefined): string {
  return tr(CHANNELS[channel as ControlChannel]?.label ?? CHANNELS.none.label);
}

export function channelTone(channel: ControlChannel | string | undefined): Tone {
  return CHANNELS[channel as ControlChannel]?.tone ?? 'muted';
}

export function channelHint(channel: ControlChannel | string | undefined): string {
  return tr(CHANNELS[channel as ControlChannel]?.hint ?? CHANNELS.none.hint);
}

export interface ModeOption {
  value: PermissionMode;
  label: string;
  hint: string;
  dangerous?: boolean;
}

/** English: translated where they are read (`tr(m.label)`, `modeLabel`). */
export const PERMISSION_MODES: ModeOption[] = [
  { value: 'default', label: 'Ask', hint: 'Ask before edits and commands' },
  { value: 'acceptEdits', label: 'Accept edits', hint: 'File edits go through; commands ask' },
  { value: 'plan', label: 'Plan', hint: 'Plan first, change nothing' },
  { value: 'auto', label: 'Auto', hint: 'A classifier decides what needs you' },
  {
    value: 'bypassPermissions',
    label: 'Bypass permissions',
    hint: 'Never ask (dangerous)',
    dangerous: true,
  },
];

export function modeLabel(mode: string | undefined): string {
  if (!mode) return tr('Ask');
  if (mode === 'dontAsk') return tr("Don't ask");
  const label = PERMISSION_MODES.find((m) => m.value === mode)?.label;
  return label ? tr(label) : mode;
}

/** English (the API page lists the keys): `effortLabel` translates. */
export const EFFORT_LABELS: Record<string, string> = {
  low: 'Low',
  medium: 'Medium',
  high: 'High',
  xhigh: 'Extra high',
  max: 'Max',
};

/** An effort level in words, in the language shown (`||effort`: "Max" is
 * also a plan's name). */
export function effortLabel(effort: string | undefined): string {
  if (!effort) return '';
  const label = EFFORT_LABELS[effort];
  return label ? tr(`${label}||effort`) : effort;
}

/** Chip-sized effort labels (the list cards). */
export const EFFORT_SHORT: Record<string, string> = {
  low: 'Low',
  medium: 'Med',
  high: 'High',
  xhigh: 'X-high',
  max: 'Max',
};

export function effortShortLabel(effort: string | undefined): string {
  if (!effort) return '';
  const label = EFFORT_SHORT[effort];
  return label ? tr(`${label}||effort`) : effortLabel(effort);
}

/** How an effort value is known, as a phrase after the value. */
export function effortSourceText(source: string | undefined): string {
  switch (source) {
    case 'applied':
      return tr('as Claude Code applies it');
    case 'observed':
      return tr('as sent with the latest request');
    case 'requested':
      return tr('requested, not confirmed yet');
    case 'launch':
      return tr('set at launch');
    default:
      return '';
  }
}

/** Where a session's own settings (ultracode, thinking…) are switched, as
 * a phrase: "in VS Code", "in the terminal"; '' when there is no one place. */
export function switchPlace(surface: Surface | string | undefined): string {
  switch (surface) {
    case 'vscode':
      return tr('in VS Code');
    case 'terminal':
    case 'tmux':
      return tr('in the terminal');
    case 'jetbrains':
      return tr('in JetBrains');
    case 'desktop':
      return tr('in Claude Desktop');
    default:
      return '';
  }
}

export function ultracodeLabel(state: string | undefined): string {
  switch (state) {
    case 'on':
      return tr('On');
    case 'off':
      return tr('Off');
    case 'unavailable':
      return tr('Unavailable');
    default:
      return '';
  }
}

export function thinkingModeLabel(mode: string | undefined): string {
  switch (mode) {
    case 'adaptive':
      return tr('Adaptive');
    case 'on':
      return tr('On');
    case 'off':
      return tr('Off');
    default:
      return '';
  }
}

/** "On · 32k budget" / "Adaptive" / "Off". */
export function thinkingText(t: ThinkingState | undefined): string {
  if (!t) return '';
  const mode = thinkingModeLabel(t.mode);
  return t.mode === 'on' && t.budget
    ? `${mode} · ${tr('{n} budget', { n: formatTokens(t.budget) })}`
    : mode;
}

/** How a thinking setting is known, as a phrase after the value. */
export function thinkingSourceText(source: string | undefined): string {
  switch (source) {
    case 'control':
      return tr('set by Mother Claude');
    case 'launch':
      return tr('at launch');
    case 'default':
      return tr("Claude Code's default||source");
    default:
      return '';
  }
}

/** Thinking's share of the output tokens, in whole percent (null when no
 * thinking was counted). Both totals include subagents. */
export function thinkingShare(usage: Usage | undefined): number | null {
  const thinking = usage?.thinking ?? 0;
  const output = usage?.output ?? 0;
  if (thinking <= 0 || output <= 0) return null;
  return Math.min(100, Math.max(1, Math.round((thinking / output) * 100)));
}

/** A plan-usage meter's colour: amber near the limit, red at it. */
export function severityTone(severity: PlanSeverity): Tone {
  return severity === 'critical' ? 'red' : severity === 'warning' ? 'amber' : 'accent';
}

const ACTIVITY_VERBS: Record<ActivityKind, string> = {
  thinking: 'Thinking',
  tool: 'Running',
  waiting: 'Waiting for you',
  delegating: 'Delegating',
  background: 'Background work',
  scheduled: 'Scheduled to resume',
  compacting: 'Compacting',
  retrying: 'Retrying',
  idle: 'Idle',
  starting: 'Starting',
  ended: 'Ended',
};

/** "Running Bash" — the server's label, else one derived from the kind. */
export function activityLabel(a: Activity | undefined): string {
  if (!a) return '';
  if (a.label?.trim()) return activityText(a.label.trim());
  if (a.kind === 'tool' && a.tool) return tr('Running {tool}', { tool: a.tool });
  const verb = ACTIVITY_VERBS[a.kind];
  return verb ? activityText(verb) : capitalize(a.kind ?? '');
}

/** An activity label in the language shown. The server's come from a
 * fixed list (`claude/activity.rs`: "Thinking", "Running Bash", "Waiting
 * for approval"…), keyed with `||activity` ("Thinking" is also the
 * setting's name); any other label shows as it came. */
function activityText(label: string): string {
  const known = tr(`${label}||activity`);
  if (known !== label) return known;
  const agents = /^Running (\d+) agents$/.exec(label);
  if (agents) return tr('Running {n} agents', { n: agents[1] });
  const running = /^Running (.+)$/.exec(label);
  if (running) return tr('Running {tool}', { tool: running[1] });
  const waiting = /^Waiting: (.+)$/.exec(label);
  if (waiting) return tr('Waiting: {what}', { what: waiting[1] });
  return label;
}

/** Kinds that mean "busy right now" (animated dot). */
export function activityIsBusy(a: Activity | undefined): boolean {
  return (
    !!a &&
    ['thinking', 'tool', 'delegating', 'background', 'compacting', 'retrying', 'starting'].includes(
      a.kind,
    )
  );
}

export function sessionName(
  s: Pick<SessionSummary, 'id' | 'name' | 'title' | 'projectName'>,
): string {
  return s.title?.trim() || s.name?.trim() || s.projectName || s.id.slice(0, 8);
}

/** `/Users/me/dev/app` → `~/dev/app`, `C:\Users\me\dev\app` → `~\dev\app`. */
export function shortPath(path: string | undefined): string {
  return tildePath(path);
}

export function capitalize(s: string): string {
  return s ? s[0].toUpperCase() + s.slice(1) : s;
}

// ──────────────────────────────────────────────────── Tools and git files

/** The argument of a tool call that tells a human what it actually does
 * (the Bash command, the file path, …). Mirrors the sidecar's SALIENT_ARGS. */
const SALIENT_ARGS: Record<string, string[]> = {
  Bash: ['description', 'command'],
  Edit: ['file_path'],
  MultiEdit: ['file_path'],
  Write: ['file_path'],
  Read: ['file_path'],
  Grep: ['pattern', 'path'],
  Glob: ['pattern'],
  WebFetch: ['url'],
  WebSearch: ['query'],
  Task: ['description'],
  Agent: ['description'],
  TodoWrite: [],
  NotebookEdit: ['notebook_path'],
};

/** One-line, human-readable summary of a tool call's input. */
export function toolSummary(name: string, input: unknown, max = 160): string {
  if (input == null || typeof input !== 'object') return '';
  const rec = input as Record<string, unknown>;
  const keys = SALIENT_ARGS[name];
  if (keys?.length === 0) return '';
  const parts = keys
    ?.map((k) => rec[k])
    .filter((v) => v != null && v !== '')
    .slice(0, name === 'Bash' ? 1 : undefined)
    .map((v) => (typeof v === 'string' ? v : JSON.stringify(v)));
  const text = (parts?.length ? parts.join(' · ') : JSON.stringify(rec)) ?? '';
  const oneLine = text.replace(/\s+/g, ' ').trim();
  return oneLine.length > max ? `${oneLine.slice(0, max)}…` : oneLine;
}

/** Changed files grouped by folder for the tree view, folders sorted, root
 * (`.`) first, files keeping their original (git) order within each group. */
export function groupByDir(files: FileChange[]): { dir: string; files: FileChange[] }[] {
  const groups = new Map<string, FileChange[]>();
  for (const f of files) {
    const i = f.path.lastIndexOf('/');
    const dir = i === -1 ? '.' : f.path.slice(0, i);
    const list = groups.get(dir) ?? [];
    list.push(f);
    groups.set(dir, list);
  }
  return [...groups.entries()]
    .sort(([a], [b]) => (a === '.' ? -1 : b === '.' ? 1 : a.localeCompare(b)))
    .map(([dir, list]) => ({ dir, files: list }));
}

/** Basename of a repo path (what the tree view shows next to its folder). */
export function baseName(path: string): string {
  const i = path.lastIndexOf('/');
  return i === -1 ? path : path.slice(i + 1);
}
