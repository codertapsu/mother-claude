import {
  Activity,
  ActivityKind,
  ControlChannel,
  FileChange,
  PermissionMode,
  SessionStatus,
  SessionSummary,
  Surface,
} from '../core/models';

/** Tones map to the theme's status colours (see styles.scss). */
export type Tone = 'green' | 'amber' | 'red' | 'blue' | 'accent' | 'muted' | 'faint';

// ───────────────────────────────────────────────────────────────── Time

export function relativeTime(ms?: number, now = Date.now()): string {
  if (!ms) return '—';
  const diff = now - ms;
  if (diff < 5_000) return 'just now';
  const s = Math.floor(diff / 1000);
  if (s < 60) return `${s}s ago`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ago`;
  const d = Math.floor(h / 24);
  if (d < 30) return `${d}d ago`;
  return new Date(ms).toLocaleDateString();
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

// ──────────────────────────────────────────────────────────────── Labels

const STATUS: Record<SessionStatus, { label: string; tone: Tone }> = {
  working: { label: 'Working', tone: 'green' },
  waiting: { label: 'Needs you', tone: 'amber' },
  idle: { label: 'Idle', tone: 'muted' },
  starting: { label: 'Starting', tone: 'blue' },
  ended: { label: 'Ended', tone: 'faint' },
  failed: { label: 'Failed', tone: 'red' },
  stopped: { label: 'Stopped', tone: 'faint' },
};

export function statusLabel(status: SessionStatus | string): string {
  return STATUS[status as SessionStatus]?.label ?? capitalize(String(status || 'unknown'));
}

export function statusTone(status: SessionStatus | string): Tone {
  return STATUS[status as SessionStatus]?.tone ?? 'muted';
}

const SURFACES: Record<Surface, string> = {
  vscode: 'VS Code',
  terminal: 'Terminal',
  tmux: 'tmux',
  desktop: 'Claude Desktop',
  jetbrains: 'JetBrains',
  background: 'Background',
  'mother-claude': 'Mother Claude',
  sdk: 'SDK',
  unknown: 'Unknown',
};

export function surfaceLabel(surface: Surface | string | undefined): string {
  return SURFACES[surface as Surface] ?? capitalize(String(surface || 'unknown'));
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
  return CHANNELS[channel as ControlChannel]?.label ?? 'Watch only';
}

export function channelTone(channel: ControlChannel | string | undefined): Tone {
  return CHANNELS[channel as ControlChannel]?.tone ?? 'muted';
}

export function channelHint(channel: ControlChannel | string | undefined): string {
  return CHANNELS[channel as ControlChannel]?.hint ?? CHANNELS.none.hint;
}

export interface ModeOption {
  value: PermissionMode;
  label: string;
  hint: string;
  dangerous?: boolean;
}

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
  if (!mode) return 'Ask';
  if (mode === 'dontAsk') return "Don't ask";
  return PERMISSION_MODES.find((m) => m.value === mode)?.label ?? mode;
}

export const EFFORT_LABELS: Record<string, string> = {
  low: 'Low',
  medium: 'Medium',
  high: 'High',
  xhigh: 'Extra high',
  max: 'Max',
};

export function effortLabel(effort: string | undefined): string {
  if (!effort) return '';
  return EFFORT_LABELS[effort] ?? effort;
}

const ACTIVITY_VERBS: Record<ActivityKind, string> = {
  thinking: 'Thinking',
  tool: 'Running',
  waiting: 'Waiting for you',
  delegating: 'Delegating',
  background: 'Background work',
  compacting: 'Compacting',
  retrying: 'Retrying',
  idle: 'Idle',
  starting: 'Starting',
  ended: 'Ended',
};

/** "Running Bash" — the server's label, else one derived from the kind. */
export function activityLabel(a: Activity | undefined): string {
  if (!a) return '';
  if (a.label?.trim()) return a.label.trim();
  const verb = ACTIVITY_VERBS[a.kind] ?? capitalize(a.kind ?? '');
  return a.kind === 'tool' && a.tool ? `${verb} ${a.tool}` : verb;
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

/** `/Users/me/dev/app` → `~/dev/app`. */
export function shortPath(path: string | undefined): string {
  if (!path) return '';
  return path.replace(/^\/(Users|home)\/[^/]+/, '~');
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
