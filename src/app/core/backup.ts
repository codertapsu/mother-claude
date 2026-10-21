import { ApiError, describeError } from './api.service';
import { isTransferRefusal, normalizeJob, plural, sentence, transferErrorText } from './data-jobs';
import {
  BackupAction,
  BackupConfigEntry,
  BackupIndex,
  BackupIndexSession,
  BackupMachine,
  BackupSet,
  BackupSkip,
  BackupSuggestion,
  BackupView,
  BackupWarning,
  JobView,
  PassphraseChange,
  PullResult,
  PullSkip,
  PushResult,
  RestoreResult,
} from './models';

/**
 * Pure helpers for Backup & sync (`/backup*`, docs/BACKUP.md): the view
 * and a set's index read tolerantly, what a restore or pull does with each
 * session in words, which sessions start selected, and the results.
 */

/** Longest schedule the server takes (a week). */
export const BACKUP_MAX_HOURS = 168;

/** Shortest passphrase of the user's own (the server's rule). */
export const PASSPHRASE_MIN = 12;

/** The schedules offered (0: by hand only). */
export const BACKUP_SCHEDULES: readonly { hours: number; label: string }[] = [
  { hours: 0, label: 'By hand only' },
  { hours: 1, label: 'Every hour' },
  { hours: 3, label: 'Every 3 hours' },
  { hours: 6, label: 'Every 6 hours' },
  { hours: 12, label: 'Every 12 hours' },
  { hours: 24, label: 'Every 24 hours' },
];

export function scheduleLabel(hours: number): string {
  const known = BACKUP_SCHEDULES.find((s) => s.hours === hours);
  if (known) return known.label;
  return hours === 1 ? 'Every hour' : `Every ${hours} hours`;
}

const ACTIONS: readonly BackupAction[] = [
  'new',
  'update',
  'conflict',
  'same',
  'localNewer',
  'live',
];

function str(v: unknown): string | undefined {
  return typeof v === 'string' && v.trim() ? v : undefined;
}

function num(v: unknown): number | undefined {
  return typeof v === 'number' && Number.isFinite(v) ? v : undefined;
}

function count(v: unknown): number {
  const n = num(v);
  return n !== undefined && n > 0 ? n : 0;
}

function obj(v: unknown): Record<string, unknown> | null {
  return v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : null;
}

function list<T>(v: unknown, read: (x: unknown) => T | null): T[] {
  return Array.isArray(v) ? v.map(read).filter((x): x is T => x !== null) : [];
}

/** Non-empty strings of a list (anything else is dropped). */
function texts(v: unknown): string[] {
  return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string' && !!x.trim()) : [];
}

function machine(raw: unknown): BackupMachine | null {
  const r = obj(raw);
  const host = str(r?.['host']);
  if (!r || !host) return null;
  const id = str(r['id']);
  const lastPush = num(r['lastPush']);
  return {
    ...(id ? { id } : {}),
    host,
    ...(lastPush !== undefined ? { lastPush } : {}),
  };
}

function backupSet(raw: unknown): BackupSet | null {
  const r = obj(raw);
  const account = str(r?.['account']);
  if (!r || !account) return null;
  const sharedWith = str(r['sharedWith']);
  const lastPush = num(r['lastPush']);
  const lastError = str(r['lastError']);
  const sessions = num(r['sessions']);
  return {
    account,
    name: str(r['name']) ?? account,
    dir: str(r['dir']) ?? '',
    exists: r['exists'] === true,
    ...(sharedWith && sharedWith !== account ? { sharedWith } : {}),
    keyMissing: r['keyMissing'] === true,
    ...(lastPush !== undefined ? { lastPush } : {}),
    ...(lastError ? { lastError } : {}),
    ...(sessions !== undefined ? { sessions } : {}),
    machines: list(r['machines'], machine),
  };
}

/** The sets of their own: a token account's is the Default's (`sharedWith`),
 * so it is backed up, restored and pulled there. */
export function ownSets(sets: readonly BackupSet[]): BackupSet[] {
  return sets.filter((s) => !s.sharedWith);
}

/** The other machines that pushed to a set. With this Mac's id (`machine`,
 * known once it has pushed or pulled) every other machine is told apart, also
 * one with this Mac's name; without it only by name, and `sameName` says
 * whether another Mac has this Mac's name too. */
export function otherMachines(
  machines: readonly BackupMachine[],
  host: string,
  machine?: string,
): { others: BackupMachine[]; sameName: boolean } {
  if (machine) {
    return {
      others: machines.filter((m) => (m.id ? m.id !== machine : m.host !== host)),
      sameName: false,
    };
  }
  return {
    others: machines.filter((m) => m.host !== host),
    sameName: machines.filter((m) => m.host === host).length > 1,
  };
}

/** `Default`, `Default and Work`, `Default, Work and Personal`. */
export function andList(names: readonly string[]): string {
  if (names.length <= 1) return names[0] ?? '';
  return `${names.slice(0, -1).join(', ')} and ${names[names.length - 1]}`;
}

/** What the page says about a set whose key is not in the folder here. */
export const KEY_MISSING_TEXT =
  "Its key (identity.age) isn't in the folder on this Mac — deleted, or not downloaded yet — so nothing can be backed up to it or read from it until it is back. On iCloud Drive, right-click the Mother Claude folder and choose Keep Downloaded; to start over instead, move the set's folder away.";

function suggestion(raw: unknown): BackupSuggestion | null {
  const r = obj(raw);
  const label = str(r?.['label']);
  const path = str(r?.['path']);
  return label && path ? { label, path } : null;
}

/** `GET /backup` made safe to render, or null when it isn't one. */
export function normalizeBackupView(raw: unknown): BackupView | null {
  const r = obj(raw);
  const s = obj(r?.['settings']);
  if (!r || !s) return null;
  const hours = num(s['everyHours']);
  const lastPull = num(r['lastPull']);
  const machine = str(r['machine']);
  return {
    settings: {
      folder: str(s['folder']) ?? null,
      everyHours: hours !== undefined && hours >= 0 ? Math.round(hours) : 0,
      pullOnLaunch: s['pullOnLaunch'] === true,
    },
    passphraseSet: r['passphraseSet'] === true,
    passphraseChangeUnfinished: r['passphraseChangeUnfinished'] === true,
    host: str(r['host']) ?? '',
    ...(machine ? { machine } : {}),
    sets: list(r['sets'], backupSet),
    suggestions: list(r['suggestions'], suggestion),
    ...(lastPull !== undefined ? { lastPull } : {}),
    jobs: list(r['jobs'], normalizeJob),
  };
}

function indexSession(raw: unknown): BackupIndexSession | null {
  const r = obj(raw);
  const id = str(r?.['id']);
  const action = ACTIONS.find((a) => a === r?.['action']);
  if (!r || !id || !action) return null;
  const cwd = str(r['cwd']);
  const title = str(r['title']);
  const localBytes = num(r['localBytes']);
  const pruneWarning = str(r['pruneWarning']);
  // `{at, machine}` (or true): moved out of every account into the set.
  const archived = r['archived'] === true || !!obj(r['archived']);
  return {
    id,
    action,
    ...(cwd ? { cwd } : {}),
    ...(title ? { title: title.trim() } : {}),
    remoteBytes: count(r['remoteBytes']),
    ...(localBytes !== undefined ? { localBytes } : {}),
    host: str(r['host']) ?? '',
    pushedAt: num(r['pushedAt']) ?? 0,
    newestMtime: num(r['newestMtime']) ?? 0,
    ...(pruneWarning ? { pruneWarning } : {}),
    ...(archived ? { archived } : {}),
  };
}

function configEntry(raw: unknown): BackupConfigEntry | null {
  const r = obj(raw);
  const name = str(r?.['name']);
  if (!r || !name) return null;
  return {
    name,
    createdAt: num(r['createdAt']) ?? 0,
    host: str(r['host']) ?? '',
    machine: str(r['machine']) ?? '',
    archiveSha256: str(r['archiveSha256']) ?? '',
    archiveBytes: count(r['archiveBytes']),
  };
}

/** `GET /backup/index` made safe to render, or null when it isn't one. A
 * set of a storage that no account is named after has no `account` ('').
 */
export function normalizeBackupIndex(raw: unknown): BackupIndex | null {
  const r = obj(raw);
  if (!r || !Array.isArray(r['sessions'])) return null;
  const account = str(r['account']) ?? '';
  const set = str(r['set']) ?? '';
  if (!account && !set) return null;
  return {
    account,
    name: str(r['name']) ?? (account || set),
    set,
    machines: list(r['machines'], machine),
    sessions: list(r['sessions'], indexSession),
    config: list(r['config'], configEntry),
  };
}

/** What a pull does with a session, in words. */
const PULL_TEXT: Record<BackupAction, string> = {
  new: 'New here',
  update: "Newer in the backup — replaces this Mac's copy, kept for undo",
  conflict: 'Changed on both — kept side by side',
  live: 'Running here — skipped',
  localNewer: 'Newer here',
  same: 'Same',
};

/** How a session of the set compares with the account it would go into. */
const RESTORE_TEXT: Record<BackupAction, string> = {
  new: 'New here',
  update: 'Newer in the backup — the account has it already',
  conflict: 'Changed on both — the account has it already',
  live: 'Running here',
  localNewer: 'Newer here',
  same: 'Same',
};

export function actionText(action: BackupAction, mode: 'pull' | 'restore'): string {
  return (mode === 'pull' ? PULL_TEXT : RESTORE_TEXT)[action];
}

/** A pull does something with these (the rest stay as they are). */
export function pullTakes(action: BackupAction): boolean {
  return action === 'new' || action === 'update' || action === 'conflict';
}

/** A session that runs here is never touched. */
export function restoreTakes(action: BackupAction): boolean {
  return action !== 'live';
}

/** Which sessions start selected: new ones for a restore, everything a pull
 * would bring for a pull. */
export function preselect(
  sessions: readonly BackupIndexSession[],
  mode: 'pull' | 'restore',
): Set<string> {
  return new Set(
    sessions
      .filter((s) => (mode === 'pull' ? pullTakes(s.action) : s.action === 'new'))
      .map((s) => s.id),
  );
}

/** The selected sessions Claude Code would prune again. */
export function prunedOf(
  sessions: readonly BackupIndexSession[],
  selected: ReadonlySet<string>,
): BackupIndexSession[] {
  return sessions.filter((s) => selected.has(s.id) && !!s.pruneWarning);
}

/** The warning restore and pull give about pruning (docs/ACCOUNTS.md §11.7). */
export const PRUNE_WARNING =
  'Claude Code will delete this again at its next cleanup unless cleanupPeriodDays is raised.';

function skip(raw: unknown): BackupSkip | null {
  const r = obj(raw);
  const id = typeof r?.['id'] === 'string' ? (r['id'] as string) : undefined;
  if (!r || id === undefined) return null;
  return { id, code: str(r['code']) ?? '', reason: str(r['reason']) ?? '' };
}

function warning(raw: unknown): BackupWarning | null {
  const r = obj(raw);
  const text = str(r?.['text']);
  if (!r || !text) return null;
  return { id: typeof r['id'] === 'string' ? r['id'] : '', text };
}

function ids(v: unknown): string[] {
  return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string' && !!x) : [];
}

/** A restore job's `result`. */
export function normalizeRestoreResult(raw: unknown): RestoreResult {
  const r = obj(raw) ?? {};
  return {
    restored: ids(r['restored']),
    skipped: list(r['skipped'], skip),
    warnings: list(r['warnings'], warning),
  };
}

/** A pull job's `result`. */
export function normalizePullResult(raw: unknown): PullResult {
  const r = obj(raw) ?? {};
  const conflicts: [string, string][] = Array.isArray(r['conflicts'])
    ? r['conflicts']
        .filter(
          (c): c is [string, string] =>
            Array.isArray(c) && typeof c[0] === 'string' && typeof c[1] === 'string',
        )
        .map(([id, path]) => [id, path])
    : [];
  return {
    ...normalizeRestoreResult(raw),
    updated: ids(r['updated']),
    conflicts,
    historyLines: count(r['historyLines']),
  };
}

/** A backup job's `result`, or null when it has none. */
export function normalizePushResult(raw: unknown): PushResult | null {
  const r = obj(raw);
  if (!r) return null;
  const failed: [string, string][] = Array.isArray(r['failed'])
    ? r['failed']
        .filter((f): f is [string, string] => Array.isArray(f) && typeof f[0] === 'string')
        .map(([id, why]) => [id, typeof why === 'string' ? why : ''])
    : [];
  return {
    archived: ids(r['archived']),
    unchanged: count(r['unchanged']),
    skippedLive: ids(r['skippedLive']),
    failed,
    conflicts: ids(r['conflicts']),
    newerInSet: ids(r['newerInSet']),
    config: str(r['config']) ?? null,
    bytes: count(r['bytes']),
    warnings: texts(r['warnings']),
    removed: count(r['removed']),
  };
}

/** `POST /backup/passphrase`'s answer. */
export function normalizePassphraseChange(raw: unknown): PassphraseChange {
  const r = obj(raw) ?? {};
  return {
    passphrase: str(r['passphrase']) ?? null,
    rekeyed: texts(r['rekeyed']),
    left: texts(r['left']),
  };
}

/** What a refused passphrase change left behind: whether some set has the
 * new passphrase already (then the change did not finish, and setting the
 * passphrase again finishes it) and, for a generated one, that passphrase —
 * shown, so it is never lost. */
export function passphraseFailure(e: unknown): { applied: boolean; passphrase: string } {
  if (!(e instanceof ApiError)) return { applied: false, passphrase: '' };
  return { applied: e.extra['applied'] === true, passphrase: e.detail('passphrase') };
}

/** The sets a pull of every set could not start (`{account, code,
 * message}`; an older server sent a sentence). */
export function normalizePullSkips(raw: unknown): PullSkip[] {
  if (!Array.isArray(raw)) return [];
  return raw
    .map((x): PullSkip | null => {
      if (typeof x === 'string') return x.trim() ? { account: '', code: '', message: x } : null;
      const r = obj(x);
      const message = str(r?.['message']);
      if (!r || !message) return null;
      return { account: str(r['account']) ?? '', code: str(r['code']) ?? '', message };
    })
    .filter((x): x is PullSkip => x !== null);
}

/** One line about a finished job ('' when there is nothing to say). */
export function jobSummary(job: JobView): string {
  if (job.state === 'failed') return backupJobErrorText(job.errorCode, job.error) || 'It failed.';
  if (job.state === 'undone') return 'Undone: everything is back as it was.';
  if (job.state !== 'done') return '';
  switch (job.kind) {
    case 'backup': {
      const r = normalizePushResult(job.result);
      if (!r) return 'Backed up.';
      const parts = [
        r.archived.length
          ? `${plural(r.archived.length, 'session')} backed up`
          : 'Nothing new to back up',
        r.unchanged ? `${r.unchanged} unchanged` : '',
        r.conflicts.length ? plural(r.conflicts.length, 'conflict') : '',
        r.newerInSet.length ? `${r.newerInSet.length} newer in the backup` : '',
        r.skippedLive.length ? `${r.skippedLive.length} running, skipped` : '',
        r.failed.length ? `${r.failed.length} failed` : '',
        r.warnings.length ? plural(r.warnings.length, 'warning') : '',
        r.config ? 'settings saved' : '',
      ];
      return parts.filter(Boolean).join(' · ');
    }
    case 'restore': {
      const r = normalizeRestoreResult(job.result);
      const parts = [
        `${plural(r.restored.length, 'session')} restored`,
        r.skipped.length ? `${r.skipped.length} skipped` : '',
      ];
      return parts.filter(Boolean).join(' · ');
    }
    case 'pull': {
      const r = normalizePullResult(job.result);
      const parts = [
        r.restored.length ? `${r.restored.length} new` : '',
        r.updated.length ? `${r.updated.length} updated` : '',
        r.conflicts.length ? plural(r.conflicts.length, 'conflict') : '',
        r.skipped.length ? `${r.skipped.length} skipped` : '',
        r.historyLines ? `${plural(r.historyLines, 'prompt-history line')} added` : '',
      ];
      return parts.filter(Boolean).join(' · ') || 'Already up to date.';
    }
    default: {
      const n = job.sessions.length;
      return `${job.kind === 'move' ? 'Moved' : 'Copied'} ${plural(n, 'session')}.`;
    }
  }
}

/** What each backup refusal means, when the server says nothing. */
const BACKUP_TEXT: Record<string, string> = {
  no_backup_folder: 'Choose a backup folder first.',
  no_backup_passphrase: 'Set a passphrase first.',
  wrong_passphrase:
    'The passphrase on this Mac does not open this backup. Set the passphrase it was made with (Passphrase › Change), then try again.',
  backup_unauthenticated:
    "This backup failed its check: its index was not written with this passphrase's key — someone changed it, or an older build of Mother Claude made it. Nothing was read from it.",
  set_key_missing:
    "The backup's key (identity.age) isn't in the folder on this Mac — deleted, or not downloaded yet. Nothing can be backed up or read until it is back: on iCloud Drive, choose Keep Downloaded for the Mother Claude folder.",
  invalid_backup: 'There is no backup to read there.',
  backup_io: 'The backup folder could not be read or written.',
  invalid_schedule: 'Back up at least once a week, or by hand only.',
  invalid_folder: 'That folder can’t be used.',
};

/** Said our way, whatever the server's words: the prerequisites, and the
 * refusals whose message names paths and file formats. */
const OUR_WORDS = new Set([
  'no_backup_folder',
  'no_backup_passphrase',
  'wrong_passphrase',
  'backup_unauthenticated',
  'set_key_missing',
]);

/** A refusal of a backup, restore, pull or settings change, in words. */
export function backupErrorText(e: unknown): string {
  const code = e instanceof ApiError ? (e.code ?? '') : '';
  if (OUR_WORDS.has(code)) return BACKUP_TEXT[code];
  if (e instanceof ApiError && code in BACKUP_TEXT) {
    return sentence(e.message) || BACKUP_TEXT[code];
  }
  if (isTransferRefusal(e)) return transferErrorText(e);
  return describeError(e).text;
}

/** The same refusal said about a job that ended with it (`errorCode`). */
export function backupJobErrorText(code: string | undefined, message: string | undefined): string {
  return code && OUR_WORDS.has(code) ? BACKUP_TEXT[code] : sentence(message);
}
