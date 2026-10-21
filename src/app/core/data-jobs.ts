import { ApiError, describeError } from './api.service';
import { DataJobKind, DataJobState, JobView } from './models';

/**
 * Pure helpers for the data jobs (`/data/jobs`: moves, copies, backups,
 * restores, pulls), read tolerantly like everything else from the wire.
 * Moving data (`POST /data/move`) has its own in move-data.ts.
 */

/** How often a running job is asked about (ms). */
export const JOB_POLL_MS = 700;

const KINDS: readonly DataJobKind[] = ['move', 'copy', 'backup', 'restore', 'pull'];
const STATES: readonly DataJobState[] = ['running', 'done', 'failed', 'undone'];

function num(v: unknown): number | undefined {
  return typeof v === 'number' && Number.isFinite(v) ? v : undefined;
}

function str(v: unknown): string | undefined {
  return typeof v === 'string' && v.trim() ? v : undefined;
}

function count(v: unknown): number {
  const n = num(v);
  return n !== undefined && n > 0 ? n : 0;
}

/** A job from the wire, or null when it isn't one. */
export function normalizeJob(raw: unknown): JobView | null {
  if (!raw || typeof raw !== 'object') return null;
  const r = raw as Record<string, unknown>;
  const id = str(r['id']);
  const kind = KINDS.find((k) => k === r['kind']);
  const state = STATES.find((s) => s === r['state']);
  if (!id || !kind || !state) return null;
  const finishedAt = num(r['finishedAt']);
  const undoUntil = num(r['undoUntil']);
  const error = str(r['error']);
  const errorCode = str(r['errorCode']);
  const account = str(r['account']);
  const from = str(r['from']);
  const to = str(r['to']);
  const fromStorage = str(r['fromStorage']);
  const toStorage = str(r['toStorage']);
  const set = str(r['set']);
  const toSet = str(r['toSet']);
  return {
    id,
    kind,
    state,
    startedAt: num(r['startedAt']) ?? 0,
    ...(finishedAt !== undefined ? { finishedAt } : {}),
    filesDone: count(r['filesDone']),
    filesTotal: count(r['filesTotal']),
    bytesDone: count(r['bytesDone']),
    bytesTotal: count(r['bytesTotal']),
    ...(error ? { error } : {}),
    ...(errorCode ? { errorCode } : {}),
    ...(undoUntil !== undefined ? { undoUntil } : {}),
    sessions: Array.isArray(r['sessions'])
      ? r['sessions'].filter((s): s is string => typeof s === 'string')
      : [],
    ...(account ? { account } : {}),
    ...(from ? { from } : {}),
    ...(to ? { to } : {}),
    ...(fromStorage ? { fromStorage } : {}),
    ...(toStorage ? { toStorage } : {}),
    ...(set ? { set } : {}),
    ...(toSet ? { toSet } : {}),
    ...(r['result'] !== undefined && r['result'] !== null ? { result: r['result'] } : {}),
  };
}

/** A job named in an answer: its view, or just its id (some routes answer
 * the id only). '' when neither. */
export function jobIdOf(raw: unknown): string {
  if (typeof raw === 'string') return raw.trim();
  return normalizeJob(raw)?.id ?? '';
}

/** Undo is offered: it finished (or failed without putting everything back:
 * `transfer_incomplete` keeps its undo, and Undo finishes the job) and its
 * undo has not run out. */
export function canUndo(j: JobView | null | undefined, now: number): boolean {
  return !!j && (j.state === 'done' || j.state === 'failed') && !!j.undoUntil && j.undoUntil > now;
}

/** How far a job is, 0–1 (bytes when known, else files); null when its
 * size is not known yet. */
export function jobFraction(j: JobView): number | null {
  if (j.state !== 'running') return 1;
  if (j.bytesTotal > 0) return Math.min(1, j.bytesDone / j.bytesTotal);
  if (j.filesTotal > 0) return Math.min(1, j.filesDone / j.filesTotal);
  return null;
}

/** `1536` → "1.5 KB", `0` → "0 B". */
export function formatBytes(n: number | undefined): string {
  if (!n || !Number.isFinite(n) || n <= 0) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let v = n;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  return i === 0 ? `${v} B` : `${v < 10 ? v.toFixed(1) : Math.round(v)} ${units[i]}`;
}

/** "1 session" / "3 sessions". */
export function plural(n: number, one: string, many = `${one}s`): string {
  return `${n} ${n === 1 ? one : many}`;
}

const KIND_LABELS: Record<DataJobKind, string> = {
  move: 'Move',
  copy: 'Copy',
  backup: 'Backup',
  restore: 'Restore',
  pull: 'Pull',
};

export function jobKindLabel(kind: DataJobKind): string {
  return KIND_LABELS[kind];
}

/** What a job did, with the places it names: "Move from Default to Work",
 * "Move from Work to iCloud Drive", "Copy from Google Drive · Shared to
 * Personal", "Backup of Work", "Restore of Work into Personal". `nameOf`
 * gives an account's name and `storageOf` a storage's (undefined when
 * unknown: then only what is known is said). A storage's set is named
 * unless it is the set of the account on the other side. */
export function jobTitle(
  j: JobView,
  nameOf: (id: string) => string | undefined,
  storageOf: (id: string) => string | undefined = () => undefined,
): string {
  const kind = jobKindLabel(j.kind);
  const name = (id: string | undefined) => (id ? nameOf(id) : undefined);
  const set = name(j.account);
  switch (j.kind) {
    case 'move':
    case 'copy': {
      const fromAccount = name(j.from);
      const toAccount = name(j.to);
      const side = (storage: string | undefined, other: string | undefined) => {
        const label = storage ? storageOf(storage) : undefined;
        if (!label) return undefined;
        return j.set && j.set !== other ? `${label} · ${j.set}` : label;
      };
      // Both sides storages: `set` is the source's, `toSet` the target's
      // (named when it differs from the source's).
      const from = j.fromStorage ? side(j.fromStorage, toAccount) : fromAccount;
      const toLabel = j.toStorage ? storageOf(j.toStorage) : undefined;
      const to = j.toStorage
        ? j.fromStorage
          ? toLabel && j.toSet && j.toSet !== j.set
            ? `${toLabel} · ${j.toSet}`
            : toLabel
          : side(j.toStorage, fromAccount)
        : toAccount;
      if (from && to) return `${kind} from ${from} to ${to}`;
      if (to) return `${kind} to ${to}`;
      if (from) return `${kind} from ${from}`;
      return kind;
    }
    case 'restore': {
      const into = name(j.to);
      if (set && into && j.to !== j.account) return `${kind} of ${set} into ${into}`;
      return set ? `${kind} of ${set}` : kind;
    }
    default:
      return set ? `${kind} of ${set}` : kind;
  }
}

/** "Running", "Done", "Failed", "Undone". */
export function jobStateLabel(state: DataJobState): string {
  switch (state) {
    case 'running':
      return 'Running';
    case 'done':
      return 'Done';
    case 'failed':
      return 'Failed';
    default:
      return 'Undone';
  }
}

/** A server message as a sentence: capitalised, with a full stop. */
export function sentence(text: string | undefined): string {
  const t = (text ?? '').trim();
  if (!t) return '';
  const s = t[0].toUpperCase() + t.slice(1);
  return /[.!?…]$/.test(s) ? s : `${s}.`;
}

/** What each refusal of a move or copy means, when the server says nothing. */
const TRANSFER_TEXT: Record<string, string> = {
  same_directory:
    "Both accounts use the same folder: a copy there would be the same conversation. Move it instead: that records it as the other account's.",
  not_found: 'There is nothing of it to move or copy.',
  session_live: 'It is running; stop it first.',
  already_exists: 'The other account already has this conversation.',
  directory_in_use:
    "A Claude Code process is using the other account's directory; quit its sessions first.",
  config_locked: 'Claude Code is writing its settings right now; try again in a few seconds.',
  invalid_transfer: 'That move or copy is not possible.',
  verify_failed: 'A copied file did not match its source.',
  transfer_io: 'The files could not be written.',
  no_data_dir: 'This build has no data directory to keep an undo in.',
  undo_conflict:
    'Something it created was changed since (for example, the copied conversation was continued), so it is not taken back.',
  session_changed:
    'The conversation changed while it was being moved; nothing was moved. Try again.',
  data_job_running: 'A backup, restore or transfer is running; try again when it has finished.',
  transfer_incomplete:
    'It failed, and putting things back failed too. Nothing is lost: undo it to finish.',
};

/** The error is one of a move's, copy's or undo's own refusals. */
export function isTransferRefusal(e: unknown): e is ApiError {
  return e instanceof ApiError && !!e.code && e.code in TRANSFER_TEXT;
}

/** A refusal of a move, copy or undo (or of a continue-on copy), in words:
 * the server's own sentence, else what its code means. */
export function transferErrorText(e: unknown): string {
  if (isTransferRefusal(e)) {
    return sentence(e.message) || TRANSFER_TEXT[e.code as string];
  }
  return describeError(e).text;
}
