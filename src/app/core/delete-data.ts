import { ApiError } from './api.service';
import { formatBytes, plural, sentence } from './data-jobs';
import { JobView } from './models';
import { moveErrorText } from './move-data';

/**
 * Finding conversations on disk and deleting them (docs/ACCOUNTS.md §14):
 * where a conversation's files are, every project folder on disk, the
 * delete request, its dry run and its jobs' results — read tolerantly like
 * everything else from the wire — and the words for them.
 */

function str(v: unknown): string | undefined {
  return typeof v === 'string' && v.trim() ? v : undefined;
}

function num(v: unknown): number | undefined {
  return typeof v === 'number' && Number.isFinite(v) ? v : undefined;
}

function count(v: unknown): number {
  const n = num(v);
  return n !== undefined && n > 0 ? Math.round(n) : 0;
}

function obj(v: unknown): Record<string, unknown> | null {
  return v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : null;
}

function list<T>(v: unknown, read: (x: unknown) => T | null): T[] {
  return Array.isArray(v) ? v.map(read).filter((x): x is T => x !== null) : [];
}

function texts(v: unknown): string[] {
  return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string' && !!x.trim()) : [];
}

/** An account named on the wire: a bare id or `{account: id}`. */
function accountOf(v: unknown): string | undefined {
  return str(v) ?? str(obj(v)?.['account']);
}

// ─────────────────────────────────────────────── Where a conversation is

/** Where a conversation's files are on the Mac (the session detail's
 * `files`, §14.1): absolute paths. */
export interface SessionFiles {
  /** The main transcript, the one the index uses. */
  transcript: string;
  /** Its `projects/<enc>/<id>/` folder (subagents, tool results). */
  folder?: string;
  /** Its `file-history/<id>/` folder. */
  fileHistory?: string;
  /** The account whose directory holds them. */
  account?: string;
}

export function normalizeSessionFiles(raw: unknown): SessionFiles | null {
  const r = obj(raw);
  const transcript = str(r?.['transcript']);
  if (!r || !transcript) return null;
  const folder = str(r['folder']);
  const fileHistory = str(r['fileHistory']);
  const account = accountOf(r['account']);
  return {
    transcript,
    ...(folder ? { folder } : {}),
    ...(fileHistory ? { fileHistory } : {}),
    ...(account ? { account } : {}),
  };
}

/** The folder a file is in (`/a/b/c.jsonl` → `/a/b`). */
export function parentFolder(path: string): string {
  const i = path.replace(/[\\/]+$/, '').search(/[\\/][^\\/]*$/);
  return i > 0 ? path.slice(0, i) : path;
}

/** What the desktop runs on, from its webview's user agent. */
export type DesktopOs = 'macos' | 'windows' | 'other';

export function desktopOs(userAgent: string): DesktopOs {
  if (/Windows/.test(userAgent)) return 'windows';
  if (/Macintosh|Mac OS X/.test(userAgent)) return 'macos';
  return 'other';
}

/** "Show in Finder", "Show in File Explorer", "Show in folder". */
export function revealLabel(os: DesktopOs): string {
  return os === 'macos'
    ? 'Show in Finder'
    : os === 'windows'
      ? 'Show in File Explorer'
      : 'Show in folder';
}

// ─────────────────────────────────────────────────── Projects on disk

/** A project folder that holds conversations, in one account's directory
 * (`GET /data/projects`, §14.2). */
export interface DataProject {
  /** The account (a token account's rows: the shared folder's
   * conversations attributed to it). */
  account: string;
  /** The folder the transcripts record. */
  cwd: string;
  name: string;
  /** Its `projects/<enc>/` folder on disk. */
  folder: string;
  sessions: number;
  /** Of them, running now. */
  live: number;
  bytes: number;
  lastActivityAt?: number;
}

function dataProject(raw: unknown): DataProject | null {
  const r = obj(raw);
  const cwd = str(r?.['cwd']);
  const account = accountOf(r?.['account']);
  if (!r || !cwd || !account) return null;
  const last = num(r['lastActivityAt']);
  const name =
    str(r['name'])?.trim() ||
    cwd
      .replace(/[\\/]+$/, '')
      .split(/[\\/]/)
      .pop() ||
    cwd;
  return {
    account,
    cwd,
    name,
    folder: str(r['folder']) ?? '',
    sessions: count(r['sessions']),
    live: count(r['live']),
    bytes: count(r['bytes']),
    ...(last !== undefined ? { lastActivityAt: last } : {}),
  };
}

/** `GET /data/projects` made safe to render (newest first, each row once). */
export function normalizeDataProjects(raw: unknown): DataProject[] {
  const r = obj(raw);
  const seen = new Set<string>();
  return list(r?.['projects'] ?? raw, dataProject)
    .filter((p) => !seen.has(projectKey(p)) && !!seen.add(projectKey(p)))
    .sort((a, b) => (b.lastActivityAt ?? 0) - (a.lastActivityAt ?? 0));
}

/** A project row's identity: its account and folder. */
export function projectKey(p: Pick<DataProject, 'account' | 'cwd'>): string {
  return `${p.account}\u0000${p.cwd}`;
}

/** The search box: `q` (lower case) in its name, folder or folder on disk. */
export function projectMatches(p: DataProject, q: string): boolean {
  return !q || [p.name, p.cwd, p.folder].some((v) => v.toLowerCase().includes(q));
}

// ───────────────────────────────────────────────────────────── Deleting

export interface DeleteItems {
  sessions?: string[];
  projects?: string[];
}

/** `POST /data/delete` (§14.3). */
export interface DeleteBody {
  /** One account's conversations (left out: every directory that has
   * them). */
  from?: string;
  items: DeleteItems;
  /** Keep a copy seven days so it can be undone (the default). */
  undo: boolean;
  dryRun?: boolean;
}

/** One request of a delete: one account's conversations and projects, or
 * (`from` left out) every directory's. */
export interface DeleteGroup {
  from?: string;
  sessions?: readonly string[];
  projects?: readonly string[];
}

export function deleteBody(g: DeleteGroup, undo: boolean): DeleteBody {
  const items: DeleteItems = {
    ...(g.sessions?.length ? { sessions: [...g.sessions] } : {}),
    ...(g.projects?.length ? { projects: [...g.projects] } : {}),
  };
  return { ...(g.from ? { from: g.from } : {}), items, undo };
}

/** Conversations as delete requests, one per account they are recorded as
 * (one not known here: from wherever it is). */
export function sessionGroups(items: readonly { id: string; account?: string }[]): DeleteGroup[] {
  const by = new Map<string, string[]>();
  for (const s of items) {
    const key = s.account ?? '';
    by.set(key, [...(by.get(key) ?? []), s.id]);
  }
  return [...by.entries()].map(([from, sessions]) => ({
    ...(from ? { from } : {}),
    sessions,
  }));
}

/** Project rows as delete requests, one per account. */
export function projectGroups(
  rows: readonly Pick<DataProject, 'account' | 'cwd'>[],
): DeleteGroup[] {
  const by = new Map<string, string[]>();
  for (const p of rows) by.set(p.account, [...(by.get(p.account) ?? []), p.cwd]);
  return [...by.entries()].map(([from, projects]) => ({ from, projects }));
}

export interface DeletePlanSession {
  id: string;
  title?: string;
  cwd?: string;
  /** The account whose directory it goes from. */
  account?: string;
  files: number;
  bytes: number;
}

/** Something that does not go, and why. */
export interface DeleteSkip {
  id?: string;
  cwd?: string;
  code: string;
  message: string;
}

/** A dry run: everything a delete would take, nothing taken yet. */
export interface DeletePlan {
  sessions: DeletePlanSession[];
  skipped: DeleteSkip[];
  files: number;
  bytes: number;
  /** One job per directory. */
  jobs: number;
  /** A list holds only its first 1000 entries (the run takes them all). */
  truncated: boolean;
}

function planSession(raw: unknown): DeletePlanSession | null {
  const r = obj(raw);
  const id = str(r?.['id']);
  if (!r || !id) return null;
  const title = str(r['title']);
  const cwd = str(r['cwd']);
  const account = accountOf(r['account']);
  const files = r['files'];
  return {
    id,
    ...(title ? { title: title.trim() } : {}),
    ...(cwd ? { cwd } : {}),
    ...(account ? { account } : {}),
    files: Array.isArray(files) ? files.length : count(files),
    bytes: count(r['bytes']),
  };
}

function skip(raw: unknown): DeleteSkip | null {
  if (typeof raw === 'string') return raw.trim() ? { code: '', message: raw.trim() } : null;
  const r = obj(raw);
  if (!r) return null;
  const id = str(r['id']);
  const cwd = str(r['cwd']);
  const code = str(r['code']) ?? '';
  const message = str(r['message']) ?? str(r['reason']) ?? '';
  if (!id && !cwd && !code && !message) return null;
  return { ...(id ? { id } : {}), ...(cwd ? { cwd } : {}), code, message };
}

/** A dry run from the wire (`{plan}`'s value), or null when it isn't one. */
export function normalizeDeletePlan(raw: unknown): DeletePlan | null {
  const r = obj(raw);
  if (!r || (!Array.isArray(r['sessions']) && !Array.isArray(r['skipped']))) return null;
  const sessions = list(r['sessions'], planSession);
  const truncated = r['truncated'];
  const jobs = num(r['jobs']);
  return {
    sessions,
    skipped: list(r['skipped'], skip),
    files:
      num(r['files']) !== undefined ? count(r['files']) : sessions.reduce((n, s) => n + s.files, 0),
    bytes:
      num(r['bytes']) !== undefined ? count(r['bytes']) : sessions.reduce((n, s) => n + s.bytes, 0),
    jobs: jobs !== undefined && jobs >= 1 ? Math.round(jobs) : 1,
    truncated:
      truncated === true ||
      (Array.isArray(truncated) && truncated.length > 0) ||
      (!!obj(truncated) && Object.values(obj(truncated) ?? {}).some((x) => x === true)),
  };
}

/** The dry runs of several requests (one per account) as one. */
export function mergeDeletePlans(plans: readonly DeletePlan[]): DeletePlan {
  return {
    sessions: plans.flatMap((p) => p.sessions),
    skipped: plans.flatMap((p) => p.skipped),
    files: plans.reduce((n, p) => n + p.files, 0),
    bytes: plans.reduce((n, p) => n + p.bytes, 0),
    jobs: plans.reduce((n, p) => n + (p.sessions.length ? p.jobs : 0), 0),
    truncated: plans.some((p) => p.truncated),
  };
}

/** One sentence about what a dry run would take. */
export function deletePlanLine(p: DeletePlan): string {
  if (!p.sessions.length) return 'There is nothing to delete here.';
  const size = ` (${plural(p.files, 'file')}, ${formatBytes(p.bytes)})`;
  const more = p.truncated ? ' The lists show the first 1000; it takes them all.' : '';
  return `Deletes ${plural(p.sessions.length, 'conversation')}${size}.${more}`;
}

/** What the confirm button says: "Delete 12 conversations". */
export function deleteButtonLabel(p: DeletePlan | null): string {
  const n = p?.sessions.length ?? 0;
  return n ? `Delete ${plural(n, 'conversation')}` : 'Nothing to delete';
}

/** What each skip code means, when the server says nothing. */
const SKIP_TEXT: Record<string, string> = {
  session_live:
    'It is running — in Claude Code, a session here or the API: stop it (or close it in the API) first.',
  not_found: 'It is not there any more.',
};

/** Why something is not deleted, as a sentence. */
export function deleteSkipReason(s: DeleteSkip): string {
  return sentence(s.message) || SKIP_TEXT[s.code] || 'Skipped.';
}

/** What a device without the dangerous right is told (§14.3: deleting a
 * transcript is a dangerous action). */
export const DELETE_DANGEROUS_REASON =
  'Deleting conversations needs dangerous actions for this device, granted on the Mac in Remote › Devices.';

/** A refusal of a delete or a reveal, in words. */
export function deleteErrorText(e: unknown): string {
  if (e instanceof ApiError && e.code === 'dangerous_blocked') return DELETE_DANGEROUS_REASON;
  if (e instanceof ApiError && e.code === 'transcript_not_found') {
    return "Its transcript isn't on this Mac any more.";
  }
  if (e instanceof ApiError && e.code === 'session_not_found') {
    return 'This conversation is not known here any more.';
  }
  return moveErrorText(e);
}

// ────────────────────────────────────────────────────────────── Results

/** A delete job's `result`. */
export interface DeleteResult {
  deleted: string[];
  skipped: DeleteSkip[];
}

export function normalizeDeleteResult(raw: unknown): DeleteResult | null {
  const r = obj(raw);
  if (!r || !('deleted' in r || 'skipped' in r)) return null;
  return { deleted: texts(r['deleted']), skipped: list(r['skipped'], skip) };
}

/** One line about a finished delete ("Deleted 3 conversations · 1
 * skipped"), or null for a job that is no delete. */
export function deleteResultSummary(job: JobView): string | null {
  if (job.kind !== 'delete') return null;
  const r = normalizeDeleteResult(job.result);
  const n = r?.deleted.length ?? job.sessions.length;
  const parts = [
    n ? `Deleted ${plural(n, 'conversation')}` : 'Nothing was deleted',
    r?.skipped.length ? `${r.skipped.length} skipped` : '',
  ].filter(Boolean);
  return parts.join(' · ');
}
