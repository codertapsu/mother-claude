import { ApiError } from './api.service';
import { formatBytes, sentence } from './data-jobs';
import { computerNoun, fileManagerName, locale, tr } from './i18n/i18n';
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

/** "1,240 conversations": a count with its noun, grouped in thousands, in
 * the language shown (both nouns are keys of the dictionary). */
export function counted(n: number, one: string, many = `${one}s`): string {
  return `${n.toLocaleString(locale())} ${tr(n === 1 ? one : many)}`;
}

/** An account named on the wire: a bare id or `{account: id}`. */
function accountOf(v: unknown): string | undefined {
  return str(v) ?? str(obj(v)?.['account']);
}

// ─────────────────────────────────────────────── Where a conversation is

/** Where a conversation's files are on the desktop's computer (the session
 * detail's `files`, §14.1): absolute paths. */
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
    ? tr('Show in Finder')
    : os === 'windows'
      ? tr('Show in File Explorer')
      : tr('Show in folder');
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
  /** The conversation's title, when it has one. */
  title?: string;
  code: string;
  message: string;
}

/** A dry run: everything a delete would take, nothing taken yet. */
export interface DeletePlan {
  sessions: DeletePlanSession[];
  skipped: DeleteSkip[];
  /** How many conversations go, and are skipped, in all (the lists hold
   * at most 1000 each). */
  sessionsTotal: number;
  skippedTotal: number;
  /** The projects named that something of goes. */
  projectsTotal: number;
  /** The `projects/<enc>` folders left empty, which go too. */
  foldersTotal: number;
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
  const title = str(r['title'])?.trim();
  const code = str(r['code']) ?? '';
  const message = str(r['message']) ?? str(r['reason']) ?? '';
  if (!id && !cwd && !code && !message) return null;
  return {
    ...(id ? { id } : {}),
    ...(cwd ? { cwd } : {}),
    ...(title ? { title } : {}),
    code,
    message,
  };
}

/** A `…Total` of the plan, else what its list holds. */
function total(v: unknown, fallback: number): number {
  return num(v) !== undefined ? count(v) : fallback;
}

/** A dry run from the wire (`{plan}`'s value), or null when it isn't one. */
export function normalizeDeletePlan(raw: unknown): DeletePlan | null {
  const r = obj(raw);
  if (!r || (!Array.isArray(r['sessions']) && !Array.isArray(r['skipped']))) return null;
  const sessions = list(r['sessions'], planSession);
  const skipped = list(r['skipped'], skip);
  const truncated = r['truncated'];
  const jobs = num(r['jobs']);
  return {
    sessions,
    skipped,
    sessionsTotal: total(r['sessionsTotal'], sessions.length),
    skippedTotal: total(r['skippedTotal'], skipped.length),
    projectsTotal: total(r['projectsTotal'], texts(r['projects']).length),
    foldersTotal: total(r['foldersTotal'], texts(r['folders']).length),
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
    sessionsTotal: plans.reduce((n, p) => n + p.sessionsTotal, 0),
    skippedTotal: plans.reduce((n, p) => n + p.skippedTotal, 0),
    projectsTotal: plans.reduce((n, p) => n + p.projectsTotal, 0),
    foldersTotal: plans.reduce((n, p) => n + p.foldersTotal, 0),
    files: plans.reduce((n, p) => n + p.files, 0),
    bytes: plans.reduce((n, p) => n + p.bytes, 0),
    jobs: plans.reduce((n, p) => n + (p.sessionsTotal ? p.jobs : 0), 0),
    truncated: plans.some((p) => p.truncated),
  };
}

/** What a dry run would take, in a sentence or three: "Deletes 8
 * conversations of 2 projects (24 files, 759 KB). The project folder left
 * empty goes too." */
export function deletePlanLine(p: DeletePlan): string {
  if (!p.sessionsTotal) return tr('There is nothing to delete here.');
  const counts = {
    conversations: counted(p.sessionsTotal, 'conversation'),
    projects: counted(p.projectsTotal, 'project'),
    files: counted(p.files, 'file'),
    size: formatBytes(p.bytes),
  };
  const lines = [
    p.projectsTotal
      ? tr('Deletes {conversations} of {projects} ({files}, {size}).', counts)
      : tr('Deletes {conversations} ({files}, {size}).', counts),
  ];
  if (p.foldersTotal === 1) lines.push(tr('The project folder left empty goes too.'));
  else if (p.foldersTotal > 1) {
    lines.push(
      tr('The {folders} left empty go too.', {
        folders: counted(p.foldersTotal, 'project folder'),
      }),
    );
  }
  if (p.truncated) {
    lines.push(tr('The lists show the first {n}.', { n: (1000).toLocaleString(locale()) }));
  }
  return lines.join(' ');
}

/** What the confirm button says: "Delete 1,240 conversations". */
export function deleteButtonLabel(p: DeletePlan | null): string {
  return p?.sessionsTotal
    ? tr('Delete {conversations}', { conversations: counted(p.sessionsTotal, 'conversation') })
    : tr('Nothing to delete');
}

/** What each skip code means, when the server says nothing (English,
 * translated where read). */
const SKIP_TEXT: Record<string, string> = {
  not_found: 'It is not there any more.',
  invalid_transfer: 'That is not a conversation or a project folder.',
  session_changed: 'It was given to another account since: check again.',
  transfer_io: 'A file of it could not be moved, so it was put back whole.',
};

/** A running conversation: the server's message names it by its id.
 * English, like the next one: `deleteSkipReason` says them translated. */
export const SKIP_RUNNING = 'It is running: stop it first.';
/** A conversation open in the API's host runs too. */
export const SKIP_OPEN_IN_API = 'It is open in the API: close it there first.';

/** Why something is not deleted, as a sentence (`openInApi`: running
 * conversations the opener knows are open in the API). */
export function deleteSkipReason(s: DeleteSkip, openInApi?: ReadonlySet<string>): string {
  if (s.code === 'session_live') {
    return s.id && openInApi?.has(s.id) ? tr(SKIP_OPEN_IN_API) : tr(SKIP_RUNNING);
  }
  const ours = SKIP_TEXT[s.code];
  return sentence(s.message) || (ours ? tr(ours) : '') || tr('Skipped.');
}

/** What a device without the dangerous right is told (§14.3: deleting a
 * transcript is a dangerous action). English: shown with `tr()` (or `| t`)
 * where it is read. */
export const DELETE_DANGEROUS_REASON =
  'Deleting conversations needs dangerous actions for this device, granted on the computer in Settings › Phone & devices.';

/** A refusal of a delete or a reveal, in words. */
export function deleteErrorText(e: unknown): string {
  if (e instanceof ApiError && e.code === 'dangerous_blocked') return tr(DELETE_DANGEROUS_REASON);
  if (e instanceof ApiError && e.code === 'transcript_not_found') {
    return tr("Its transcript isn't on this {computer} any more.", { computer: computerNoun() });
  }
  if (e instanceof ApiError && e.code === 'session_not_found') {
    return tr('This conversation is not known here any more.');
  }
  if (e instanceof ApiError && e.code === 'project_not_found') {
    return tr('That account has no conversations of this folder any more.');
  }
  if (e instanceof ApiError && e.code === 'reveal_failed') {
    return tr('Could not open {fileManager}.', { fileManager: fileManagerName() });
  }
  if (e instanceof ApiError && e.code === 'invalid_transfer') {
    return tr('There was nothing to delete in the request.');
  }
  return moveErrorText(e);
}

// ────────────────────────────────────────────────────────────── Results

/** A delete job's `result`. */
export interface DeleteResult {
  deleted: string[];
  skipped: DeleteSkip[];
  /** Said when the files of an `undo: false` job could not all be removed. */
  warnings: string[];
}

export function normalizeDeleteResult(raw: unknown): DeleteResult | null {
  const r = obj(raw);
  if (!r || !('deleted' in r || 'skipped' in r)) return null;
  return {
    deleted: texts(r['deleted']),
    skipped: list(r['skipped'], skip),
    warnings: texts(r['warnings']),
  };
}

/** One line about a finished delete ("Deleted 3 conversations · 1
 * skipped"), or null for a job that is no delete. */
export function deleteResultSummary(job: JobView): string | null {
  if (job.kind !== 'delete') return null;
  const r = normalizeDeleteResult(job.result);
  const n = r?.deleted.length ?? job.sessions.length;
  const line = [
    n
      ? tr('Deleted {conversations}', { conversations: counted(n, 'conversation') })
      : tr('Nothing was deleted'),
    r?.skipped.length ? tr('{n} skipped', { n: r.skipped.length.toLocaleString(locale()) }) : '',
  ]
    .filter(Boolean)
    .join(' · ');
  const warnings = (r?.warnings ?? []).map(sentence).filter(Boolean);
  return warnings.length ? `${line}. ${warnings.join(' ')}` : line;
}
