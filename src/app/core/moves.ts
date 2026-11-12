// Moving a session to another computer (docs/PLAN-MACHINES.md §5.8, Phase
// 3a; spikes/machines/UX.md §7): the wire types of
// `/api/sessions/:id/move*`, `/api/moves*` and the `move` frame, as
// `src-tauri/src/machines/moves/` and `move_routes.rs` send them, and the
// helpers every move screen shares. Labels here are English; they are
// translated where they are shown.

import type { SessionSummary } from './models';

/** Where a move stands (`MoveState`): `preflight → freezing → packaging →
 * sending → landing → landed → committed → resumed`, or it ends `failed`,
 * `cancelled` or `undone`. */
export type MoveState =
  | 'preflight'
  | 'freezing'
  | 'packaging'
  | 'sending'
  | 'landing'
  | 'landed'
  | 'committed'
  | 'resumed'
  | 'failed'
  | 'cancelled'
  | 'undone';

/** What the move sheet offers instead of a blocker (`Check.offer`; a
 * failure's `offer` too). */
export type MoveOffer = 'worktree' | 'worktree-or-reset' | 'as-copy' | 'choose-folder';

/** A machine as a move names it. */
export interface MoveMachine {
  id: string;
  name: string;
  /** `macos` | `windows` | `linux` | `other`. */
  os: string;
}

/** A blocker or a warning, in the server's words (English). */
export interface MoveCheck {
  code: string;
  message: string;
  /** What fixes it, when there is something to do. */
  fix?: string;
  offer?: MoveOffer | string;
  /** More for the sheet: `{candidates: [folder…]}` for `choose-folder`. */
  detail?: unknown;
}

export interface MoveStep {
  state: MoveState;
  at: number;
}

/** One file of a move's package. */
export interface MoveFile {
  /** `session.tar.gz` (the conversation), `meta.json`, `move.bundle`,
   * `lfs.<oid>` (the code). */
  name: string;
  size: number;
  sha256: string;
  /** Bytes the target has. */
  sent: number;
}

/** The code of a move. */
export interface MoveCode {
  /** `bundle` | `remote` | `none`. */
  mode: string;
  /** `checkout` | `worktree`. */
  apply: string;
  remote?: string | null;
  /** `refs/heads/…`, none when detached. */
  branch?: string | null;
  base?: string | null;
  snapshot?: string | null;
  /** Paths `git status` showed on the source, untracked ones included. */
  changed: number;
  untracked: number;
  /** The snapshot's warnings (`lfs`, `submodules`, `case-collision`, …). */
  warnings: string[];
  /** The files git ignores that travel with it (the project's list), under
   * the repository's top. */
  ignored?: string[];
  /** *Replace the branch there*: the tip of the target's branch it
   * replaces (kept under the landing's undo ref for *Undo move*). */
  replaces?: string | null;
  /** Where it landed on the target (`{landing, path, action, undoRef…}`). */
  landed?: { landing?: string; path?: string; action?: string; undoRef?: string } | null;
}

/** What the target did with the conversation. */
export interface MoveLanding {
  job?: string | null;
  /** `new` | `fastForward` | `same` | `conflict`. */
  action: string;
  /** Its own turns, kept beside it (a conflict). */
  conflict?: string | null;
  trusted: boolean;
}

/** Why a move failed, and at which step. */
export interface MoveFailure {
  code: string;
  message: string;
  step: MoveState;
  offer?: MoveOffer | string | null;
}

/** A move, as `GET /api/moves/{id}` and the `move` frame carry it. */
export interface MoveRecord {
  /** `mv_` and 16 hex characters. */
  id: string;
  /** `out`: this record's machine sends it; `in`: it receives it. */
  role: 'out' | 'in';
  /** The session moved (its id on the source). */
  session: string;
  /** The id it goes on under there (a copy's is new). */
  targetSession?: string | null;
  title?: string | null;
  from: MoveMachine;
  to: MoveMachine;
  state: MoveState;
  /** It has ended (an undo may still follow `resumed`). */
  finished: boolean;
  asCopy: boolean;
  keepDriving: boolean;
  /** `idle` (after this turn) | `now` (interrupting it). */
  when: string;
  account: { name: string; isDefault: boolean };
  epoch: number;
  sourceCwd: string;
  targetCwd?: string | null;
  targetFolder?: string | null;
  code?: MoveCode | null;
  files: MoveFile[];
  bytesTotal: number;
  bytesSent: number;
  /** The conversation's own size. */
  conversationBytes: number;
  /** Mother Claude ran it when it was frozen. */
  wasRunning: boolean;
  /** Messages taken back from behind an interrupted turn (they go on
   * there). */
  heldCount: number;
  recipe: string[];
  trust: boolean;
  /** The conversation landed from the copy the target keeps current (§5.9):
   * it did not travel again. */
  fromCopy?: boolean;
  steps: MoveStep[];
  error?: MoveFailure | null;
  warnings: MoveCheck[];
  landing?: MoveLanding | null;
  /** It landed and the lease moved, but it did not start there. */
  resumeError?: string | null;
  /** Who started it (`desktop`, `device Phone`, `machine DESKTOP-PC`). */
  by: string;
  createdAt: number;
  updatedAt: number;
  finishedAt?: number | null;
  /** Until when *Undo move* is offered. */
  undoUntil?: number | null;
}

/** `POST /api/sessions/:id/move` and `…/move/preflight`. */
export interface SessionMoveBody {
  /** The computer to move it to (its machine id; this one's for a take
   * over or a bring back). */
  to: string;
  /** `bundle` (machine to machine) | `remote` (your git remote) | `none`
   * (only the conversation); the setup's choice when absent. */
  code?: CodeTransfer;
  /** `checkout` (the project folder there) | `worktree` (a separate one). */
  apply?: CodeLanding;
  /** The folder there, chosen in the sheet. */
  folder?: string;
  /** On by default (the server's default too). */
  keepDriving?: boolean;
  /** A copy goes on there under a new id; this one stays. */
  asCopy?: boolean;
  /** Ask Claude to run the project's recipe there. */
  recipe?: boolean;
  /** Carry the folder's trust there (the dangerous right). */
  trust?: boolean;
  /** Carry the files git ignores that the project lists (the dangerous
   * right on both computers). Left out, they travel when this caller holds
   * that right and stay otherwise; `false` leaves them. */
  carryIgnored?: boolean;
  /** The project folder's branch there went on there: replace it with this
   * computer's (its commits there kept for *Undo move*). */
  replaceBranch?: boolean;
  /** The session's computer is offline: it lives here from now on. */
  takeOver?: boolean;
  /** After this turn (`idle`, the default) or now, interrupting it. */
  when?: 'idle' | 'now';
  remote?: string;
}

export type CodeTransfer = 'bundle' | 'remote' | 'none';
export type CodeLanding = 'checkout' | 'worktree';

/** The session as the preflight read it. */
export interface PreflightSession {
  id: string;
  title?: string | null;
  owned: boolean;
  live: boolean;
  /** Another process runs it (a terminal, an editor): it moves as a copy. */
  foreign: boolean;
  account?: { id: string; name: string } | null;
  cwd: string;
  /** All of its files. */
  bytes: number;
  transcriptBytes: number;
  files: number;
  subagents: number;
}

/** The code as the preflight read it. */
export interface PreflightCode {
  /** The transfer in force: `bundle`, `remote`, `none`. */
  mode: string;
  apply?: string;
  /** The folder is in a git repository. */
  repo: boolean;
  top?: string | null;
  branch?: string | null;
  head?: string | null;
  changed?: number;
  untracked?: number;
  origin?: string | null;
  /** The target's branch against this base: `create`, `same`,
   * `fastForward`, `diverged`. */
  relation?: string | null;
  /** The files git ignores that the project lists, as found here (*Not in
   * git*); they travel when `carryIgnored`. */
  ignored?: string[];
  ignoredBytes?: number;
  carryIgnored?: boolean;
  /** The branch there (diverged) is replaced by this computer's. */
  replaceBranch?: boolean;
}

/** What the other computer answered. */
export interface PreflightTarget {
  account?: { id: string; name: string; signedIn?: boolean | null } | null;
  folder?: { path: string; how: string; trusted: boolean } | null;
  /** Its folders that may be the project, to choose from. */
  candidates: string[];
  /** Where the session will work there. */
  cwd?: string | null;
  code?: {
    repo: boolean;
    clean: boolean;
    parked: boolean;
    changed: number;
    untracked: number;
    origin?: string | null;
    branch?: string | null;
  } | null;
  session?: { here: boolean; live: boolean } | null;
  claude?: string | null;
  freeBytes?: number | null;
  /** The copy of the session it keeps current (§5.9): the move lands from
   * it, and the conversation does not travel again. */
  copy?: { held: boolean; copiedAt?: number | null; bytes: number; complete: boolean } | null;
}

/** `POST /api/sessions/:id/move/preflight`: nothing is written anywhere. */
export interface MovePreflight {
  ok: boolean;
  blockers: MoveCheck[];
  warnings: MoveCheck[];
  session: PreflightSession;
  to: { id: string; name: string; os: string; online: boolean };
  code: PreflightCode;
  target: PreflightTarget | null;
  /** The project's recipe for the target's system. */
  recipe: string[];
  asCopy: boolean;
}

/** `POST /api/sessions/:id/move {takeOver: true}`. */
export interface TakeOverReply {
  takenOver: true;
  session: string;
  home: string;
  from: string;
  epoch: number;
  /** What it means, in the server's words. */
  warning: string;
}

/** `{t:'move', move}`: a move changed (everyone). */
export interface MoveFrame {
  t: 'move';
  move: MoveRecord;
}

// ─────────────────────────────────────────────────────────────── Parsing

const STATES: ReadonlySet<string> = new Set<MoveState>([
  'preflight',
  'freezing',
  'packaging',
  'sending',
  'landing',
  'landed',
  'committed',
  'resumed',
  'failed',
  'cancelled',
  'undone',
]);
const FINAL: ReadonlySet<MoveState> = new Set<MoveState>([
  'resumed',
  'failed',
  'cancelled',
  'undone',
]);

/** A move's id: `mv_` and 16 lowercase hex characters. */
export function isMoveId(id: unknown): id is string {
  return typeof id === 'string' && /^mv_[0-9a-f]{16}$/.test(id);
}

function obj(v: unknown): Record<string, unknown> {
  return v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : {};
}

function str(v: unknown): string {
  return typeof v === 'string' ? v : '';
}

function num(v: unknown): number {
  return typeof v === 'number' && Number.isFinite(v) ? v : 0;
}

function optStr(v: unknown): string | null {
  return typeof v === 'string' ? v : null;
}

function optNum(v: unknown): number | null {
  return typeof v === 'number' && Number.isFinite(v) ? v : null;
}

function machineOf(v: unknown): MoveMachine {
  const m = obj(v);
  return { id: str(m['id']), name: str(m['name']), os: str(m['os']) };
}

/** A check as the server sent it (null when it has no code or words). */
export function normalizeCheck(v: unknown): MoveCheck | null {
  const c = obj(v);
  const code = str(c['code']);
  const message = str(c['message']);
  if (!code && !message) return null;
  return {
    code,
    message,
    ...(typeof c['fix'] === 'string' ? { fix: c['fix'] } : {}),
    ...(typeof c['offer'] === 'string' ? { offer: c['offer'] } : {}),
    ...(c['detail'] !== undefined && c['detail'] !== null ? { detail: c['detail'] } : {}),
  };
}

function checks(v: unknown): MoveCheck[] {
  return Array.isArray(v) ? v.map(normalizeCheck).filter((c): c is MoveCheck => c !== null) : [];
}

function strings(v: unknown): string[] {
  return Array.isArray(v) ? v.filter((s): s is string => typeof s === 'string') : [];
}

/**
 * A move record as the server sent it, every field tolerant (a newer
 * server's additions pass through untouched); null when it is not one.
 */
export function normalizeMove(v: unknown): MoveRecord | null {
  const r = obj(v);
  const id = r['id'];
  const state = str(r['state']);
  if (!isMoveId(id) || !STATES.has(state)) return null;
  const code = obj(r['code']);
  const error = obj(r['error']);
  const landing = obj(r['landing']);
  const account = obj(r['account']);
  return {
    ...(r as unknown as MoveRecord),
    id,
    role: r['role'] === 'in' ? 'in' : 'out',
    session: str(r['session']),
    targetSession: optStr(r['targetSession']),
    title: optStr(r['title']),
    from: machineOf(r['from']),
    to: machineOf(r['to']),
    state: state as MoveState,
    finished: typeof r['finished'] === 'boolean' ? r['finished'] : FINAL.has(state as MoveState),
    asCopy: r['asCopy'] === true,
    keepDriving: r['keepDriving'] === true,
    when: str(r['when']) || 'idle',
    account: { name: str(account['name']), isDefault: account['isDefault'] === true },
    epoch: num(r['epoch']),
    sourceCwd: str(r['sourceCwd']),
    targetCwd: optStr(r['targetCwd']),
    targetFolder: optStr(r['targetFolder']),
    code: r['code']
      ? {
          ...(code as unknown as MoveCode),
          mode: str(code['mode']),
          apply: str(code['apply']),
          branch: optStr(code['branch']),
          changed: num(code['changed']),
          untracked: num(code['untracked']),
          warnings: strings(code['warnings']),
          ignored: strings(code['ignored']),
          replaces: optStr(code['replaces']),
        }
      : null,
    files: Array.isArray(r['files'])
      ? r['files'].map((f) => {
          const x = obj(f);
          return {
            name: str(x['name']),
            size: num(x['size']),
            sha256: str(x['sha256']),
            sent: num(x['sent']),
          };
        })
      : [],
    bytesTotal: num(r['bytesTotal']),
    bytesSent: num(r['bytesSent']),
    conversationBytes: num(r['conversationBytes']),
    wasRunning: r['wasRunning'] === true,
    heldCount: num(r['heldCount']),
    recipe: strings(r['recipe']),
    trust: r['trust'] === true,
    fromCopy: r['fromCopy'] === true,
    steps: Array.isArray(r['steps'])
      ? r['steps']
          .map((s) => obj(s))
          .filter((s) => STATES.has(str(s['state'])))
          .map((s) => ({ state: str(s['state']) as MoveState, at: num(s['at']) }))
      : [],
    error: r['error']
      ? {
          code: str(error['code']),
          message: str(error['message']),
          step: (STATES.has(str(error['step'])) ? str(error['step']) : state) as MoveState,
          offer: optStr(error['offer']),
        }
      : null,
    warnings: checks(r['warnings']),
    landing: r['landing']
      ? {
          job: optStr(landing['job']),
          action: str(landing['action']),
          conflict: optStr(landing['conflict']),
          trusted: landing['trusted'] === true,
        }
      : null,
    resumeError: optStr(r['resumeError']),
    by: str(r['by']),
    createdAt: num(r['createdAt']),
    updatedAt: num(r['updatedAt']),
    finishedAt: optNum(r['finishedAt']),
    undoUntil: optNum(r['undoUntil']),
  };
}

/** `{moves: […]}`, the records that parse, newest first. */
export function normalizeMoves(v: unknown): MoveRecord[] {
  const list = obj(v)['moves'];
  return (Array.isArray(list) ? list : [])
    .map(normalizeMove)
    .filter((m): m is MoveRecord => m !== null)
    .sort((a, b) => b.createdAt - a.createdAt);
}

/** A preflight's answer, tolerant like a record. */
export function normalizePreflight(v: unknown): MovePreflight | null {
  const p = obj(v);
  if (!('blockers' in p) && !('ok' in p)) return null;
  const s = obj(p['session']);
  const to = obj(p['to']);
  const code = obj(p['code']);
  const t = p['target'] ? obj(p['target']) : null;
  const acc = s['account'] ? obj(s['account']) : null;
  const blockers = checks(p['blockers']);
  return {
    ok: typeof p['ok'] === 'boolean' ? p['ok'] : blockers.length === 0,
    blockers,
    warnings: checks(p['warnings']),
    session: {
      id: str(s['id']),
      title: optStr(s['title']),
      owned: s['owned'] === true,
      live: s['live'] === true,
      foreign: s['foreign'] === true,
      account: acc ? { id: str(acc['id']), name: str(acc['name']) } : null,
      cwd: str(s['cwd']),
      bytes: num(s['bytes']),
      transcriptBytes: num(s['transcriptBytes']),
      files: num(s['files']),
      subagents: num(s['subagents']),
    },
    to: {
      id: str(to['id']),
      name: str(to['name']),
      os: str(to['os']),
      online: to['online'] !== false,
    },
    code: {
      ...(code as unknown as PreflightCode),
      mode: str(code['mode']) || 'none',
      repo: code['repo'] === true,
      ignored: strings(code['ignored']),
      carryIgnored: code['carryIgnored'] === true,
      replaceBranch: code['replaceBranch'] === true,
    },
    target: t
      ? {
          ...(t as unknown as PreflightTarget),
          candidates: strings(t['candidates']),
        }
      : null,
    recipe: strings(p['recipe']),
    asCopy: p['asCopy'] === true,
  };
}

// ─────────────────────────────────────────────────────────────── State

/** It has ended (`resumed`, `failed`, `cancelled`, `undone`). */
export function moveFinal(m: Pick<MoveRecord, 'state'>): boolean {
  return FINAL.has(m.state);
}

/** Before *landed*: a failure leaves both computers as they were. */
export function beforeLanded(m: Pick<MoveRecord, 'state'>): boolean {
  return (
    m.state === 'preflight' ||
    m.state === 'freezing' ||
    m.state === 'packaging' ||
    m.state === 'sending' ||
    m.state === 'landing'
  );
}

/** Under way: not ended (a move that landed but did not start there yet
 * counts). */
export function moveRunning(m: Pick<MoveRecord, 'state'>): boolean {
  return !moveFinal(m);
}

/** *Cancel* works on the source until the target starts placing it. */
export function canCancel(m: MoveRecord): boolean {
  return (
    m.role === 'out' &&
    (m.state === 'preflight' ||
      m.state === 'freezing' ||
      m.state === 'packaging' ||
      m.state === 'sending')
  );
}

/** *Try again* starts it there again: it landed (and its lease moved) but
 * did not start. Nothing is sent twice. */
export function canRetry(m: MoveRecord): boolean {
  return (m.state === 'landed' || m.state === 'committed') && !!m.resumeError;
}

/** *Undo move* is offered until `undoUntil` (seven days); never for a copy. */
export function canUndo(m: MoveRecord, now = Date.now()): boolean {
  if (m.asCopy) return false;
  const placed =
    m.state === 'committed' || m.state === 'resumed' || (m.role === 'in' && m.state === 'landed');
  return placed && (!m.undoUntil || now <= m.undoUntil);
}

/** The session the move went on as, on the computer it went to. */
export function movedSession(m: MoveRecord): string {
  return m.targetSession || m.session;
}

/** The conversation's share of the package, and the code's. */
export function packageSizes(m: MoveRecord): { conversation: number; code: number } {
  let conversation = 0;
  let code = 0;
  for (const f of m.files) {
    if (f.name === 'session.tar.gz') conversation += f.size;
    else code += f.size;
  }
  return { conversation, code };
}

/** When the record reached `state` (ms), if it did. */
export function reachedAt(m: MoveRecord, state: MoveState): number | null {
  for (let i = m.steps.length - 1; i >= 0; i--) {
    if (m.steps[i].state === state) return m.steps[i].at;
  }
  return null;
}

/** How far it got, 0–1 (a bar's width). */
export function moveFraction(m: MoveRecord): number {
  switch (m.state) {
    case 'preflight':
      return 0.02;
    case 'freezing':
      return 0.05;
    case 'packaging':
      return 0.1;
    case 'sending':
      return 0.12 + 0.78 * (m.bytesTotal > 0 ? Math.min(1, m.bytesSent / m.bytesTotal) : 0);
    case 'landing':
      return 0.92;
    case 'landed':
    case 'committed':
      return 0.97;
    default:
      return 1;
  }
}

/** Seconds left of the sending, from its pace so far (null: unknown). */
export function secondsLeft(m: MoveRecord, now = Date.now()): number | null {
  if (m.state !== 'sending' || m.bytesTotal <= 0 || m.bytesSent <= 0) return null;
  const since = reachedAt(m, 'sending');
  if (!since || now <= since) return null;
  const rate = m.bytesSent / (now - since);
  if (!(rate > 0)) return null;
  return Math.max(1, Math.round((m.bytesTotal - m.bytesSent) / rate / 1000));
}

/** In a turn the move waits for (or interrupts): an owned session that
 * works now, or waits on someone in the middle of its turn; one that
 * resumes by itself between turns is not. */
export function sessionInTurn(
  s: Pick<SessionSummary, 'live' | 'owned' | 'status' | 'activity'> | null | undefined,
): boolean {
  return (
    !!s &&
    s.live &&
    s.owned &&
    s.status !== 'background' &&
    (s.status === 'working' || s.status === 'waiting' || BUSY_ACTIVITY.has(s.activity?.kind ?? ''))
  );
}

/** Activities of a turn under way (as `shared/util.ts` `activityIsBusy`). */
const BUSY_ACTIVITY: ReadonlySet<string> = new Set([
  'thinking',
  'tool',
  'delegating',
  'background',
  'compacting',
  'retrying',
  'starting',
]);

/** `refs/heads/feat/x` → `feat/x`. */
export function branchName(ref: string | null | undefined): string {
  return (ref ?? '').replace(/^refs\/heads\//, '');
}

/** The candidates a `choose-folder` check offers (its `detail`), else the
 * target's. */
export function folderCandidates(check: MoveCheck | null, pre: MovePreflight | null): string[] {
  const detail = obj(check?.detail);
  const from = Array.isArray(detail['candidates'])
    ? detail['candidates'].filter((c): c is string => typeof c === 'string')
    : [];
  return from.length ? from : (pre?.target?.candidates ?? []);
}
