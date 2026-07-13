import { FileChange, SessionState, Surface } from '../core/models';

export function relativeTime(ms?: number): string {
  if (!ms) return '—';
  const diff = Date.now() - ms;
  if (diff < 0) return 'just now';
  const s = Math.floor(diff / 1000);
  if (s < 60) return `${s}s ago`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.floor(h / 24)}d ago`;
}

export function formatTokens(n?: number): string {
  if (!n) return '0';
  if (n < 1000) return `${n}`;
  if (n < 1_000_000) return `${(n / 1000).toFixed(1)}k`;
  return `${(n / 1_000_000).toFixed(2)}M`;
}

export function stateLabel(state: SessionState): string {
  return state.replace('-', ' ');
}

export function surfaceLabel(surface: Surface): string {
  switch (surface) {
    case 'cli':
      return 'CLI';
    case 'vs-code':
      return 'VS Code';
    case 'jet-brains':
      return 'JetBrains';
    case 'desktop':
      return 'Desktop';
    default:
      return 'Unknown';
  }
}

/** The argument of a tool call that tells a human what it actually does
 * (the Bash command, the file path, …). Mirrors the sidecar's SALIENT_ARGS. */
const SALIENT_ARGS: Record<string, string[]> = {
  Bash: ['command'],
  Edit: ['file_path'],
  Write: ['file_path'],
  Read: ['file_path'],
  Grep: ['pattern', 'path'],
  Glob: ['pattern'],
  WebFetch: ['url'],
  WebSearch: ['query'],
  Task: ['description'],
  TodoWrite: [],
  NotebookEdit: ['notebook_path'],
};

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

/** One-line, human-readable summary of a tool call's input. */
export function toolSummary(name: string, input: unknown, max = 160): string {
  if (input == null || typeof input !== 'object') return '';
  const rec = input as Record<string, unknown>;
  const keys = SALIENT_ARGS[name];
  if (keys?.length === 0) return '';
  const parts = keys
    ?.map((k) => rec[k])
    .filter((v) => v != null)
    .map((v) => (typeof v === 'string' ? v : JSON.stringify(v)));
  const text = (parts?.length ? parts.join(' · ') : JSON.stringify(rec)) ?? '';
  const oneLine = text.replace(/\s+/g, ' ').trim();
  return oneLine.length > max ? `${oneLine.slice(0, max)}…` : oneLine;
}
