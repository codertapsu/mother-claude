// Dev-only: the data side of the fixture backend for `?mock=1` — moves and
// copies between accounts (`/data/move`), data jobs (`/data/jobs*`) and
// backups (`/backup*`), with the server's refusals. Never bundled in
// production builds.
//
// `?mockbackup=1` starts with a folder, a passphrase and backups of the
// Default from this Mac and another one; a token account's sessions are in
// the Default's set, a set of no account here ("Personal") is left alone by
// a passphrase change, and a backup of the Default reports a conflict and a
// session newer in the set. Its variants: `nokey` (Work's set is in the
// folder without its key), `unfinished` (a passphrase change did not
// finish), `tampered` (the Default's set fails its check) and `wrong` (the
// passphrase opens no set).
// `?mockpass=fail` makes the next passphrase change fail after it reached a
// set (`error.applied`; a generated passphrase comes back in the error).
// `?mockundo=conflict` makes every undo refuse with `undo_conflict`.
// `?mocktransfer=locked` refuses a move or copy that writes the target's
// shared files with `config_locked`; `?mocktransfer=nodata` refuses every
// one with `no_data_dir` (a build without a data directory).

import { DEFAULT_ACCOUNT, homeOf } from '../accounts';
import {
  AccountView,
  BackupAction,
  BackupIndexSession,
  BackupMachine,
  BackupSettings,
  DataJobKind,
  JobView,
  SessionSummary,
  TransferMode,
  TransferPlan,
  TransferSessionPlan,
} from '../models';
import { urlParam } from '../platform';
import { MOCK_ACCOUNT_WORK } from './fixtures';

const BACKUP_MODE = urlParam('mockbackup');
const PASS_MODE = urlParam('mockpass');
const UNDO_MODE = urlParam('mockundo');
const TRANSFER_MODE = urlParam('mocktransfer');
/** A folder, a passphrase and earlier backups to start with. */
const CONFIGURED = ['1', 'wrong', 'nokey', 'unfinished', 'tampered'].includes(BACKUP_MODE ?? '');

const S = 1000;
const M = 60 * S;
const H = 60 * M;
const D = 24 * H;
const UNDO_KEEP = 7 * D;
const HOST = 'marcus-mbp';
const OTHER_HOST = 'marcus-air';
/** The machines' per-install ids (sets tell them apart by these). */
const THIS_MACHINE = '9f3c2a7e5b1d4c6f8a0b2c4d6e8f0a1b';
const OTHER_MACHINE = '4e6a8c0e2a4c6e8a0c2e4a6c8e0a2c4e';
/** The demo's sessions another Mac changed: one newer in the set, one
 * changed on both (a backup reports them; a pull brings them). */
const NEWER_IN_SET = 'c4a2e3f5-6a7b-4c8d-9e0f-1a2b3c4d5e6f';
const CONFLICTED = 'd5b3f4a6-7b8c-4d9e-0f1a-2b3c4d5e6f7a';
const ICLOUD = '/Users/marcus/Library/Mobile Documents/com~apple~CloudDocs/Mother Claude';
const GDRIVE =
  '/Users/marcus/Library/CloudStorage/GoogleDrive-marcus@example.com/My Drive/Mother Claude';
const DATA_DIR = '/Users/marcus/Library/Application Support/dev.motherclaude.desktop';
const PASS_ALPHABET = '23456789ABCDEFGHJKLMNPQRSTUVWXYZ';

/** What the data mock needs from the fixture backend. */
export interface DataHost {
  readonly desktop: boolean;
  /** Every session the mock knows (live list and history). */
  sessions(): SessionSummary[];
  accounts(): AccountView[];
  /** The account a session runs under (the Default when none). */
  accountOf(sessionId: string): string;
  /** A move landed: the session now belongs to `account`'s directory. */
  reassign(sessionId: string, account: string): void;
}

interface SetState {
  exists: boolean;
  /** In the folder, but not its key (deleted, or a cloud placeholder). */
  keyMissing?: boolean;
  lastPush?: number;
  lastError?: string;
  sessions?: number;
  machines: BackupMachine[];
}

/** A session only the backup set has (pushed from another Mac). */
interface RemoteSession {
  id: string;
  cwd: string;
  title: string;
  bytes: number;
  pushedAt: number;
  newestMtime: number;
  /** For an existing session: what the set's copy is compared with here. */
  action?: BackupAction;
}

interface TransferUndo {
  kind: 'move' | 'copy';
  sessions: string[];
  from: string;
  to: string;
  /** Each moved session's account before it moved. */
  before: Map<string, string>;
}

/** What a job ends with: a result, or the refusal it failed with. */
interface JobEnd {
  result?: unknown;
  sessions?: string[];
  undo?: boolean;
  onUndo?: () => void;
  error?: { code: string; message: string };
}

export class MockData {
  private readonly jobs = new Map<string, JobView>();
  private readonly undos = new Map<string, TransferUndo | (() => void)>();
  /** Session ids that have a copy in another account's directory (by home). */
  private readonly copies = new Map<string, Set<string>>();
  private settings: BackupSettings = { folder: null, everyHours: 0, pullOnLaunch: false };
  private passphraseSet = false;
  /** A passphrase change did not finish (some set has the new one). */
  private passphraseUnfinished = BACKUP_MODE === 'unfinished';
  /** `?mockpass=fail` fails the first change only. */
  private passFailed = false;
  private lastPull?: number;
  private readonly setStates = new Map<string, SetState>();
  /** What each set holds that this Mac does not (by set account). */
  private readonly remote = new Map<string, RemoteSession[]>();
  /** The backup or pull running per set: a second one answers it. */
  private readonly flights = new Map<string, string>();
  private seq = 0;

  constructor(private readonly host: DataHost) {
    const now = Date.now();
    if (CONFIGURED) {
      this.settings = { folder: ICLOUD, everyHours: 6, pullOnLaunch: false };
      this.passphraseSet = true;
      this.lastPull = now - 26 * H;
      this.setStates.set(DEFAULT_ACCOUNT, {
        exists: true,
        lastPush: now - 2 * H,
        sessions: 143,
        machines: [
          { id: THIS_MACHINE, host: HOST, lastPush: now - 2 * H },
          { id: OTHER_MACHINE, host: OTHER_HOST, lastPush: now - 5 * H },
        ],
      });
      if (BACKUP_MODE === 'nokey') {
        this.setStates.set(MOCK_ACCOUNT_WORK, {
          exists: false,
          keyMissing: true,
          machines: [],
        });
      }
      this.remote.set(DEFAULT_ACCOUNT, [
        {
          id: 'b3f1c2d4-5e6f-4a7b-8c9d-0e1f2a3b4c5d',
          cwd: '/Users/marcus/dev/website',
          title: 'Website redesign: hero section and pricing table',
          bytes: 412_000,
          pushedAt: now - 5 * H,
          newestMtime: now - 5 * H - 4 * M,
        },
        {
          id: NEWER_IN_SET,
          cwd: '/Users/marcus/dev/api-server',
          title: 'Rate limiter for the public API',
          bytes: 1_840_000,
          pushedAt: now - 5 * H,
          newestMtime: now - 7 * H,
          action: 'update',
        },
        {
          id: CONFLICTED,
          cwd: '/Users/marcus/dev/payments',
          title: 'Stripe webhook retries',
          bytes: 96_000,
          pushedAt: now - 5 * H,
          newestMtime: now - 6 * H,
          action: 'conflict',
        },
        {
          id: 'e6c4a5b7-8c9d-4e0f-1a2b-3c4d5e6f7a8b',
          cwd: '/Users/marcus/dev/legacy-cli',
          title: 'Legacy CLI flags cleanup',
          bytes: 230_000,
          pushedAt: now - 40 * D,
          newestMtime: now - 45 * D,
        },
      ]);
    }
  }

  /** Answers a data or backup route, or null when `path` is none of them. */
  route(
    path: string,
    method: string,
    body: unknown,
    q: URLSearchParams,
  ): Promise<Response> | Response | null {
    const b = (body ?? {}) as Record<string, unknown>;
    if (path === '/data/move' && method === 'POST') return this.desk(() => this.move(b));
    if (path === '/data/jobs' && method === 'GET') return json({ jobs: [...this.jobs.values()] });
    const job = /^\/data\/jobs\/([^/]+)(\/undo)?$/.exec(path);
    if (job) {
      const id = decodeURIComponent(job[1]);
      if (job[2] && method === 'POST') return this.desk(() => this.undo(id));
      if (!job[2] && method === 'GET') {
        const j = this.jobs.get(id);
        return j ? json(j) : fail(404, 'job_not_found', 'No such data job.');
      }
    }
    if (path === '/backup' && method === 'GET') return json(this.view());
    if (path === '/backup/settings' && method === 'PUT')
      return this.desk(() => this.putSettings(b));
    if (path === '/backup/passphrase' && method === 'POST') {
      return this.desk(() => this.passphrase(b));
    }
    if (path === '/backup/push' && method === 'POST') return this.desk(() => this.push(b));
    if (path === '/backup/index' && method === 'GET') return this.index(q);
    if (path === '/backup/restore' && method === 'POST') return this.desk(() => this.restore(b));
    if (path === '/backup/pull' && method === 'POST') return this.desk(() => this.pull(b));
    if (path.startsWith('/data/') || path.startsWith('/backup')) {
      return fail(404, 'not_found', `No mock for ${method} ${path}`);
    }
    return null;
  }

  /** "Continue on" a directory account copies the conversation first (P3.3). */
  copyForContinue(sessionId: string, to: string): void {
    this.copyTo(sessionId, this.homeId(to));
  }

  // ─────────────────────────────────────────────── Moving and copying

  private move(b: Record<string, unknown>): Response {
    if (TRANSFER_MODE === 'nodata') {
      return fail(409, 'no_data_dir', 'This build has no data directory to keep an undo in.');
    }
    const plan = this.plan(b);
    if (plan instanceof Response) return plan;
    if (b['dryRun'] === true) return json({ plan: plan.view });
    const id = this.newJobId();
    const sessions = plan.sessions;
    const undo: TransferUndo = {
      kind: plan.view.mode,
      sessions,
      from: plan.from,
      to: plan.to,
      before: new Map(sessions.map((s) => [s, this.host.accountOf(s)])),
    };
    const job = this.start(id, plan.view.mode, plan.view.files, plan.view.bytes, sessions, {
      from: plan.from,
      to: plan.to,
    });
    this.progress(id, 1_400, () => {
      for (const s of sessions) {
        if (plan.view.mode === 'move') this.host.reassign(s, plan.to);
        else this.copyTo(s, this.homeId(plan.to));
      }
      this.undos.set(id, undo);
      return { undo: true };
    });
    return json({ job }, 202);
  }

  /** The server's preconditions, in its order, then the dry run. */
  private plan(
    b: Record<string, unknown>,
  ): Response | { view: TransferPlan; sessions: string[]; from: string; to: string } {
    const mode = b['mode'];
    const what = b['what'];
    const from = typeof b['from'] === 'string' ? b['from'] : '';
    const to = typeof b['to'] === 'string' ? b['to'] : '';
    if ((mode !== 'move' && mode !== 'copy') || (what !== 'session' && what !== 'project')) {
      return fail(
        422,
        'invalid_transfer',
        'Send what ("session" or "project"), from, to and mode.',
      );
    }
    const fromA = this.account(from);
    const toA = this.account(to);
    if (!fromA || !toA)
      return fail(404, 'not_found', `no account ${JSON.stringify(fromA ? to : from)}`);
    const fromHome = this.homeId(from);
    const toHome = this.homeId(to);
    if (fromHome === toHome) {
      return fail(
        422,
        'same_directory',
        'both accounts use the same directory: there is nothing to move',
      );
    }
    const inFrom = this.host
      .sessions()
      .filter((s) => this.homeId(this.host.accountOf(s.id)) === fromHome);
    let sessions: SessionSummary[];
    let cwd: string | undefined;
    if (what === 'session') {
      const id = typeof b['id'] === 'string' ? b['id'] : '';
      sessions = inFrom.filter((s) => s.id === id);
      if (!sessions.length)
        return fail(404, 'not_found', `no session ${id} in ${fromA.name}'s directory`);
    } else {
      cwd = typeof b['cwd'] === 'string' ? b['cwd'].replace(/\/+$/, '') : '';
      sessions = inFrom.filter((s) => s.cwd === cwd);
      if (!sessions.length)
        return fail(404, 'not_found', `no conversation of ${cwd} in ${fromA.name}'s directory`);
    }
    const live = sessions.find((s) => s.live);
    if (live) return fail(409, 'session_live', `session ${live.id} is running; stop it first`);
    const there = sessions.find((s) => this.copies.get(s.id)?.has(toHome));
    if (there)
      return fail(409, 'already_exists', `the target account already has session ${there.id}`);
    const trust = b['trust'] === true;
    const writesShared = trust || what === 'project';
    if (writesShared && this.liveIn(toHome)) {
      return fail(
        409,
        'directory_in_use',
        `a Claude Code process is using ${this.dirOf(toA)}; quit its sessions first`,
      );
    }
    if (writesShared && TRANSFER_MODE === 'locked') {
      return fail(
        409,
        'config_locked',
        `Claude Code is writing ${this.dirOf(toA)}/.claude.json right now; try again in a few seconds`,
      );
    }
    if (mode === 'move' && what === 'project' && this.liveIn(fromHome)) {
      return fail(
        409,
        'directory_in_use',
        `a Claude Code process is using ${this.dirOf(fromA)}; quit its sessions first`,
      );
    }
    const plans: TransferSessionPlan[] = sessions.map((s) => this.sessionPlan(s, fromA, toA));
    const files = plans.reduce((n, p) => n + p.files.length, 0);
    const bytes = plans.reduce((n, p) => n + p.files.reduce((m, f) => m + f.size, 0), 0);
    const view: TransferPlan = {
      what: what === 'session' ? { what, id: sessions[0].id } : { what, cwd: cwd ?? '' },
      mode: mode as TransferMode,
      sessions: plans,
      files,
      bytes,
      addsProjectEntry: trust ? (cwd ?? sessions[0].cwd) : null,
      removesProjectEntry: mode === 'move' && what === 'project' ? (cwd ?? null) : null,
      historyLines: what === 'project' ? 3 + sessions.length * 4 : 0,
      writesShared,
    };
    return { view, sessions: sessions.map((s) => s.id), from, to };
  }

  private sessionPlan(s: SessionSummary, from: AccountView, to: AccountView): TransferSessionPlan {
    const enc = s.cwd.replace(/[^a-zA-Z0-9]/g, '-');
    const size = 40_000 + s.messageCount * 2_600;
    const rel = [
      [`projects/${enc}/${s.id}.jsonl`, size],
      [`file-history/${s.id}/index.json`, 3_200],
      [`session-env/${s.id}/env.json`, 420],
    ] as const;
    return {
      id: s.id,
      cwd: s.cwd,
      files: rel.map(([r, n]) => ({
        from: `${this.dirOf(from)}/${r}`,
        to: `${this.dirOf(to)}/${r}`,
        size: n,
      })),
    };
  }

  private undo(id: string): Response {
    const j = this.jobs.get(id);
    if (!j) return fail(404, 'job_not_found', 'No such data job.');
    const how = this.undos.get(id);
    if (j.state !== 'done' || !how || !j.undoUntil || j.undoUntil <= Date.now()) {
      return fail(409, 'undo_conflict', 'there is nothing to undo for this job any more');
    }
    if (UNDO_MODE === 'conflict') {
      return fail(
        409,
        'undo_conflict',
        'a file it created was changed since (the copied conversation was continued); nothing was undone',
      );
    }
    if (typeof how === 'function') how();
    else if (how.kind === 'move') {
      for (const s of how.sessions) this.host.reassign(s, how.before.get(s) ?? how.from);
    } else {
      for (const s of how.sessions) this.copies.get(s)?.delete(this.homeId(how.to));
    }
    this.undos.delete(id);
    const next: JobView = { ...j, state: 'undone' };
    delete next.undoUntil;
    this.jobs.set(id, next);
    return json(next);
  }

  // ───────────────────────────────────────────────────────── Backups

  private view() {
    const folder = this.settings.folder;
    return {
      settings: { ...this.settings },
      passphraseSet: this.passphraseSet,
      passphraseChangeUnfinished: this.passphraseUnfinished,
      host: HOST,
      // Like the server: every account, a token account's set being the
      // Default's (its details are that set's).
      sets: folder
        ? this.host.accounts().map((a) => {
            const owner = this.owner(a);
            const st = this.setStates.get(owner.id) ?? { exists: false, machines: [] };
            return {
              account: a.id,
              name: a.name,
              dir: `${folder}/${owner.name}`,
              exists: st.exists,
              ...(owner.id !== a.id ? { sharedWith: owner.id } : {}),
              ...(st.keyMissing ? { keyMissing: true } : {}),
              ...(st.lastPush ? { lastPush: st.lastPush } : {}),
              ...(st.lastError ? { lastError: st.lastError } : {}),
              ...(st.sessions !== undefined ? { sessions: st.sessions } : {}),
              machines: st.machines,
            };
          })
        : [],
      suggestions: [
        { label: 'iCloud Drive', path: ICLOUD },
        { label: 'Google Drive (marcus@example.com)', path: GDRIVE },
      ],
      ...(this.lastPull ? { lastPull: this.lastPull } : {}),
      jobs: [...this.jobs.values()].filter((j) => ['backup', 'restore', 'pull'].includes(j.kind)),
    };
  }

  private putSettings(b: Record<string, unknown>): Response {
    const unknown = Object.keys(b).find(
      (k) => !['folder', 'everyHours', 'pullOnLaunch'].includes(k),
    );
    if (unknown) return fail(422, 'invalid_body', `unknown field \`${unknown}\``);
    const next = { ...this.settings };
    if ('folder' in b) {
      const f = typeof b['folder'] === 'string' ? b['folder'].trim().replace(/\/+$/, '') : '';
      if (f && !f.startsWith('/') && !f.startsWith('~')) {
        return fail(422, 'invalid_folder', 'the folder must be an absolute path');
      }
      next.folder = f ? f.replace(/^~(?=\/|$)/, '/Users/marcus') : null;
    }
    if ('everyHours' in b) {
      const h = b['everyHours'];
      if (typeof h !== 'number' || !Number.isInteger(h) || h < 0 || h > 168) {
        return fail(
          422,
          'invalid_schedule',
          'Back up at least once a week (everyHours ≤ 168), or 0 for by hand only.',
        );
      }
      next.everyHours = h;
    }
    if ('pullOnLaunch' in b) next.pullOnLaunch = b['pullOnLaunch'] === true;
    this.settings = next;
    return json(this.view());
  }

  /** `POST /backup/passphrase`: every set of an account here is re-keyed,
   * all or nothing as far as it can be (`ops::change_passphrase`). */
  private passphrase(b: Record<string, unknown>): Response {
    const generate = b['generate'] === true && b['passphrase'] === undefined;
    const own =
      typeof b['passphrase'] === 'string' && b['generate'] !== true ? b['passphrase'] : null;
    if (!generate && own === null) {
      return fail(422, 'invalid_body', 'Send {"generate": true} or {"passphrase": "…"}.');
    }
    if (own !== null && [...own].length < 12) {
      return fail(422, 'invalid_backup', 'a passphrase has at least 12 characters');
    }
    if (own !== null && /[\r\n]/.test(own)) {
      return fail(422, 'invalid_backup', 'a passphrase is one line');
    }
    const folder = this.settings.folder;
    const sets = folder ? this.owners().filter((a) => this.setStates.has(a.id)) : [];
    // A set of an account here without its key: nothing changes.
    const nokey = sets.find((a) => this.setStates.get(a.id)?.keyMissing);
    if (nokey) return fail(409, 'set_key_missing', this.keyMissingMessage(nokey));
    const passphrase = generate ? newPassphrase() : null;
    const rekeyed = sets.filter((a) => this.setStates.get(a.id)?.exists).map((a) => a.name);
    if (PASS_MODE === 'fail' && !this.passFailed && rekeyed.length) {
      // The sets have it; keeping it as the passphrase did not work.
      this.passFailed = true;
      this.passphraseUnfinished = true;
      return json(
        {
          error: {
            code: 'backup_io',
            message: 'the Keychain refused the new passphrase: User interaction is not allowed',
            applied: true,
            ...(passphrase ? { passphrase } : {}),
          },
        },
        500,
      );
    }
    this.passphraseSet = true;
    this.passphraseUnfinished = false;
    // A set of no account here that opens with neither passphrase.
    const left = folder && CONFIGURED ? ['Personal'] : [];
    return json({ passphrase, rekeyed, left });
  }

  private prerequisites(): Response | null {
    if (!this.settings.folder) return fail(409, 'no_backup_folder', 'no backup folder is set');
    if (!this.passphraseSet)
      return fail(409, 'no_backup_passphrase', 'no backup passphrase is set');
    return null;
  }

  /** `POST /backup/push`: one set, or every set (each directory once). */
  private push(b: Record<string, unknown>): Response {
    const pre = this.prerequisites();
    if (pre) return pre;
    const asked = typeof b['account'] === 'string' ? b['account'] : undefined;
    const a = asked ? this.account(asked) : undefined;
    if (asked && !a) return fail(422, 'invalid_backup', `no account ${JSON.stringify(asked)}`);
    const owners = a ? [this.owner(a)] : this.owners();
    return json({ jobs: owners.map((o) => this.pushSet(o)) }, 202);
  }

  /** Back up one set; a set that is being backed up answers that job. */
  private pushSet(owner: AccountView): string {
    const key = `backup:${owner.id}`;
    const running = this.flights.get(key);
    if (running) return running;
    const mine = this.host
      .sessions()
      .filter((s) => this.homeId(this.host.accountOf(s.id)) === owner.id);
    const live = mine.filter((s) => s.live).map((s) => s.id);
    const archived = mine.filter((s) => !s.live).map((s) => s.id);
    const before = this.setStates.get(owner.id);
    const id = this.newJobId();
    this.flights.set(key, id);
    this.start(id, 'backup', archived.length, archived.length * 180_000, [], {
      account: owner.id,
    });
    this.progress(id, 1_800, () => {
      this.flights.delete(key);
      const refused = this.refusal(owner);
      if (refused) {
        if (before) this.setStates.set(owner.id, { ...before, lastError: refused.message });
        return { error: refused };
      }
      const unchanged = before?.exists ? (before.sessions ?? 0) : 0;
      const now = Date.now();
      const machines = (before?.machines ?? []).filter((m) => m.id !== THIS_MACHINE);
      this.setStates.set(owner.id, {
        exists: true,
        lastPush: now,
        sessions: unchanged + archived.length,
        machines: [{ id: THIS_MACHINE, host: HOST, lastPush: now }, ...machines],
      });
      // Another Mac pushed these since this one last had them: a pull
      // brings them (an update), or brings its copy beside (a conflict).
      const pending = (action: BackupAction) =>
        (this.remote.get(owner.id) ?? [])
          .filter((r) => r.action === action && !this.copies.get(r.id)?.has(owner.id))
          .map((r) => r.id);
      const bundle = owner.id === DEFAULT_ACCOUNT && CONFIGURED;
      return {
        result: {
          archived,
          unchanged,
          skippedLive: live,
          failed: [],
          conflicts: pending('conflict'),
          newerInSet: pending('update'),
          config: `config/${new Date(now).toISOString()}-${randomHex(12)}.tar.gz.age`,
          bytes: archived.length * 180_000,
          warnings: bundle
            ? ['agents/review.md could not be read and is not in the configuration bundle']
            : [],
          removed: before?.exists ? 2 : 0,
        },
      };
    });
    return id;
  }

  /** A set's sessions compared with `into`'s directory. */
  private compare(account: string, into: string): BackupIndexSession[] {
    const intoHome = this.homeId(into);
    const pull = this.homeId(account) === intoHome;
    const now = Date.now();
    const st = this.setStates.get(account);
    const local = this.host
      .sessions()
      .filter((s) => this.homeId(this.host.accountOf(s.id)) === this.homeId(account))
      .map((s): BackupIndexSession => {
        const hereToo =
          this.homeId(this.host.accountOf(s.id)) === intoHome ||
          this.copies.get(s.id)?.has(intoHome);
        const action: BackupAction = !hereToo
          ? 'new'
          : s.live
            ? 'live'
            : s.status === 'idle'
              ? 'localNewer'
              : 'same';
        return {
          id: s.id,
          action,
          cwd: s.cwd,
          title: s.title ?? s.name,
          remoteBytes: 40_000 + s.messageCount * 2_600,
          ...(hereToo ? { localBytes: 40_000 + s.messageCount * 2_600 } : {}),
          host: HOST,
          pushedAt: st?.lastPush ?? now - 2 * H,
          newestMtime: s.lastActivityAt ?? now - 3 * H,
        };
      });
    const remote = (this.remote.get(account) ?? []).map((r): BackupIndexSession => {
      const here = this.copies.get(r.id)?.has(intoHome);
      const action: BackupAction = here ? 'same' : pull ? (r.action ?? 'new') : 'new';
      const old = action === 'new' && r.newestMtime < now - 30 * D;
      return {
        id: r.id,
        action,
        cwd: r.cwd,
        title: r.title,
        remoteBytes: r.bytes,
        ...(here || (pull && r.action) ? { localBytes: Math.round(r.bytes * 0.8) } : {}),
        host: OTHER_HOST,
        pushedAt: r.pushedAt,
        newestMtime: r.newestMtime,
        ...(old
          ? {
              pruneWarning:
                'Claude Code will delete this again at its next cleanup unless cleanupPeriodDays (now 30) is raised.',
            }
          : {}),
      };
    });
    return [...remote, ...local].sort((a, b) => b.pushedAt - a.pushedAt);
  }

  private index(q: URLSearchParams): Response {
    const pre = this.prerequisites();
    if (pre) return pre;
    const account = q.get('account') ?? '';
    const into = q.get('into') || account;
    const a = this.account(account);
    if (!a || !this.account(into)) {
      return fail(422, 'invalid_backup', `no account ${JSON.stringify(a ? into : account)}`);
    }
    const owner = this.owner(a);
    const missing = this.noSet(owner);
    if (missing) return missing;
    const refused = this.refusal(owner);
    if (refused) return fail(refused.status, refused.code, refused.message);
    const now = Date.now();
    return json({
      account: a.id,
      name: a.name,
      set: `${this.settings.folder}/${owner.name}`,
      machines: this.setStates.get(owner.id)?.machines ?? [],
      sessions: this.compare(owner.id, into),
      config: [
        {
          name: 'config/2026-10-21T08-00-00Z-3f9a1c2b7d4e.tar.gz.age',
          createdAt: now - 2 * H,
          host: HOST,
          machine: THIS_MACHINE,
          fingerprint: 'c1d2e3f4a5b6',
          archiveSha256: '3f9a1c2b7d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8',
          archiveBytes: 48_213,
        },
        {
          name: 'config/2026-10-20T19-00-00Z-a1b2c3d4e5f6.tar.gz.age',
          createdAt: now - 5 * H,
          host: OTHER_HOST,
          machine: OTHER_MACHINE,
          fingerprint: 'f6e5d4c3b2a1',
          archiveSha256: 'a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90',
          archiveBytes: 51_877,
        },
      ],
    });
  }

  private restore(b: Record<string, unknown>): Response {
    const pre = this.prerequisites();
    if (pre) return pre;
    const account = typeof b['account'] === 'string' ? b['account'] : '';
    const a = this.account(account);
    if (!a) return fail(422, 'invalid_backup', `no account ${JSON.stringify(account)}`);
    const owner = this.owner(a);
    const missing = this.noSet(owner);
    if (missing) return missing;
    if (b['config'] === true) {
      const refused = this.refusal(owner);
      if (refused) return fail(refused.status, refused.code, refused.message);
      return json({ folder: `${DATA_DIR}/restored-config/${owner.name}` });
    }
    const ids = Array.isArray(b['ids'])
      ? b['ids'].filter((x): x is string => typeof x === 'string')
      : [];
    if (!ids.length) return fail(422, 'invalid_body', 'Name the sessions to restore (`ids`).');
    const into = typeof b['into'] === 'string' && b['into'] ? b['into'] : account;
    if (!this.account(into))
      return fail(422, 'invalid_backup', `no account ${JSON.stringify(into)}`);
    const plan = new Map(this.compare(owner.id, into).map((s) => [s.id, s]));
    const id = this.newJobId();
    this.start(id, 'restore', ids.length, 0, [], { account: owner.id, to: into });
    this.progress(id, 1_500, () => {
      const refused = this.refusal(owner);
      if (refused) return { error: refused };
      const restored: string[] = [];
      const skipped: { id: string; code: string; reason: string }[] = [];
      const warnings: { id: string; text: string }[] = [];
      for (const sid of ids) {
        const s = plan.get(sid);
        if (!s)
          skipped.push({ id: sid, code: 'not_found', reason: `the set has no session ${sid}` });
        else if (s.action === 'live')
          skipped.push({
            id: sid,
            code: 'session_live',
            reason: `session ${sid} is running; stop it first`,
          });
        else if (s.action !== 'new')
          skipped.push({
            id: sid,
            code: 'already_exists',
            reason: `the target account already has session ${sid}`,
          });
        else {
          restored.push(sid);
          if (s.pruneWarning) warnings.push({ id: sid, text: s.pruneWarning });
        }
      }
      const intoHome = this.homeId(into);
      for (const sid of restored) this.copyTo(sid, intoHome);
      return {
        result: { restored, skipped, warnings },
        sessions: restored,
        undo: restored.length > 0,
        onUndo: () => restored.forEach((sid) => this.copies.get(sid)?.delete(intoHome)),
      };
    });
    return json({ job: id }, 202);
  }

  /** `POST /backup/pull`: one set, or every set — a set that can't be
   * started is named in `skipped` and the others go ahead. */
  private pull(b: Record<string, unknown>): Response {
    const pre = this.prerequisites();
    if (pre) return pre;
    const asked = typeof b['account'] === 'string' ? b['account'] : undefined;
    const wantedIds = Array.isArray(b['ids'])
      ? new Set(b['ids'].filter((x): x is string => typeof x === 'string'))
      : null;
    if (asked) {
      const a = this.account(asked);
      if (!a) return fail(422, 'invalid_backup', `no account ${JSON.stringify(asked)}`);
      const owner = this.owner(a);
      const missing = this.noSet(owner);
      if (missing) return missing;
      return json({ jobs: [this.pullSet(owner, wantedIds)], skipped: [] }, 202);
    }
    const jobs: string[] = [];
    const skipped: { account: string; code: string; message: string }[] = [];
    for (const owner of this.owners()) {
      const st = this.setStates.get(owner.id);
      if (st?.keyMissing) {
        skipped.push({
          account: owner.id,
          code: 'set_key_missing',
          message: this.keyMissingMessage(owner),
        });
      } else if (!st?.exists) {
        skipped.push({
          account: owner.id,
          code: 'invalid_backup',
          message: `there is no backup of ${owner.name} in ${this.settings.folder}`,
        });
      } else {
        jobs.push(this.pullSet(owner, wantedIds));
      }
    }
    return json({ jobs, skipped }, 202);
  }

  /** Pull one set into its directory; one being pulled answers that job.
   * A passphrase that does not open it shows on the job. */
  private pullSet(owner: AccountView, wantedIds: Set<string> | null): string {
    const key = `pull:${owner.id}`;
    const running = this.flights.get(key);
    if (running) return running;
    const id = this.newJobId();
    this.flights.set(key, id);
    this.start(id, 'pull', 0, 0, [], { account: owner.id });
    this.progress(id, 1_800, () => {
      this.flights.delete(key);
      const refused = this.refusal(owner);
      if (refused) return { error: refused };
      const home = owner.id;
      const plan = this.compare(owner.id, owner.id).filter(
        (s) => !wantedIds || wantedIds.has(s.id),
      );
      const restored = plan.filter((s) => s.action === 'new').map((s) => s.id);
      const updated = plan.filter((s) => s.action === 'update').map((s) => s.id);
      const conflicts = plan
        .filter((s) => s.action === 'conflict')
        .map((s): [string, string] => [
          s.id,
          `${this.dirOf(owner)}/projects/${(s.cwd ?? '').replace(/[^a-zA-Z0-9]/g, '-')}/${s.id}.conflict-${OTHER_HOST}-${s.pushedAt}.jsonl`,
        ]);
      const skipped = plan
        .filter((s) => s.action === 'live')
        .map((s) => ({
          id: s.id,
          code: 'session_live',
          reason: `session ${s.id} is running here`,
        }));
      const warnings = plan
        .filter((s) => s.action === 'new' && s.pruneWarning)
        .map((s) => ({ id: s.id, text: s.pruneWarning as string }));
      for (const sid of [...restored, ...updated, ...conflicts.map((c) => c[0])])
        this.copyTo(sid, home);
      this.lastPull = Date.now();
      const changed = restored.length + updated.length + conflicts.length;
      return {
        result: { restored, updated, conflicts, skipped, warnings, historyLines: 12 },
        sessions: [...restored, ...updated],
        undo: changed > 0,
        onUndo: () => {
          for (const sid of [...restored, ...updated, ...conflicts.map((c) => c[0])]) {
            this.copies.get(sid)?.delete(home);
          }
        },
      };
    });
    return id;
  }

  /** The account whose set holds `a`'s sessions: itself, or the Default for
   * a token account (it uses the Default's directory). */
  private owner(a: AccountView): AccountView {
    return a.kind === 'token' ? (this.account(DEFAULT_ACCOUNT) ?? a) : a;
  }

  /** The accounts with a set of their own (what "all" goes through). */
  private owners(): AccountView[] {
    return this.host.accounts().filter((a) => a.kind === 'directory');
  }

  /** Why there is nothing to read in `owner`'s set: none, or no key here. */
  private noSet(owner: AccountView): Response | null {
    const st = this.setStates.get(owner.id);
    if (st?.keyMissing) return fail(409, 'set_key_missing', this.keyMissingMessage(owner));
    if (!st?.exists) {
      return fail(
        422,
        'invalid_backup',
        `there is no backup of ${owner.name} in ${this.settings.folder}`,
      );
    }
    return null;
  }

  /** Why the set can't be used (its key missing, its check failed, or no
   * passphrase here opens it), or null. */
  private refusal(owner: AccountView): { status: number; code: string; message: string } | null {
    const dir = `${this.settings.folder}/${owner.name}`;
    if (this.setStates.get(owner.id)?.keyMissing) {
      return { status: 409, code: 'set_key_missing', message: this.keyMissingMessage(owner) };
    }
    if (BACKUP_MODE === 'tampered' && owner.id === DEFAULT_ACCOUNT) {
      return {
        status: 409,
        code: 'backup_unauthenticated',
        message: `the index of the backup set in ${dir} is not authentic: it was not written with this set's key (changed by someone without the passphrase, or made by an older build)`,
      };
    }
    if (BACKUP_MODE === 'wrong') {
      return {
        status: 403,
        code: 'wrong_passphrase',
        message: 'the passphrase does not open this backup set',
      };
    }
    return null;
  }

  private keyMissingMessage(owner: AccountView): string {
    return `the backup set in ${this.settings.folder}/${owner.name} has no key (identity.age) here; if the folder is in the cloud, wait until it has downloaded (or choose Keep Downloaded); to start over, move the set away`;
  }

  // ───────────────────────────────────────────────────────── Helpers

  private desk(fn: () => Response): Response {
    return this.host.desktop
      ? fn()
      : fail(403, 'desktop_only', 'Only the desktop app can do this.');
  }

  private account(id: string | undefined): AccountView | undefined {
    return id ? this.host.accounts().find((a) => a.id === id) : undefined;
  }

  /** The directory an account's sessions live in, as an account id. */
  private homeId(id: string): string {
    const a = this.account(id);
    return a ? homeOf(a) : DEFAULT_ACCOUNT;
  }

  private dirOf(a: AccountView): string {
    if (a.kind === 'token') return '/Users/marcus/.claude';
    return a.dir ?? '/Users/marcus/.claude';
  }

  /** A live session uses that directory. */
  private liveIn(home: string): boolean {
    return this.host
      .sessions()
      .some((s) => s.live && this.homeId(this.host.accountOf(s.id)) === home);
  }

  private copyTo(sessionId: string, home: string): void {
    const set = this.copies.get(sessionId) ?? new Set<string>();
    set.add(home);
    this.copies.set(sessionId, set);
  }

  private newJobId(): string {
    this.seq += 1;
    return `job_${Date.now().toString(36)}${this.seq}`;
  }

  private start(
    id: string,
    kind: DataJobKind,
    files: number,
    bytes: number,
    sessions: string[],
    names: Pick<JobView, 'account' | 'from' | 'to'> = {},
  ): JobView {
    const job: JobView = {
      id,
      kind,
      state: 'running',
      startedAt: Date.now(),
      filesDone: 0,
      filesTotal: files,
      bytesDone: 0,
      bytesTotal: bytes,
      sessions,
      // Like the server: the accounts it names.
      ...names,
    };
    this.jobs.set(id, job);
    return job;
  }

  /** Let a job make progress for `ms`, then finish it the way `done` says. */
  private progress(id: string, ms: number, done: () => JobEnd): void {
    const steps = 4;
    for (let i = 1; i < steps; i++) {
      setTimeout(
        () => {
          const j = this.jobs.get(id);
          if (!j || j.state !== 'running') return;
          this.jobs.set(id, {
            ...j,
            filesDone: Math.floor((j.filesTotal * i) / steps),
            bytesDone: Math.floor((j.bytesTotal * i) / steps),
          });
        },
        (ms * i) / steps,
      );
    }
    setTimeout(() => {
      const j = this.jobs.get(id);
      if (!j) return;
      const out = done();
      const now = Date.now();
      if (out.error) {
        this.jobs.set(id, {
          ...j,
          state: 'failed',
          finishedAt: now,
          error: out.error.message,
          errorCode: out.error.code,
        });
        return;
      }
      if (out.onUndo) this.undos.set(id, out.onUndo);
      this.jobs.set(id, {
        ...j,
        state: 'done',
        finishedAt: now,
        filesDone: j.filesTotal,
        bytesDone: j.bytesTotal,
        ...(out.sessions ? { sessions: out.sessions } : {}),
        ...(out.result !== undefined ? { result: out.result } : {}),
        ...(out.undo ? { undoUntil: now + UNDO_KEEP } : {}),
      });
    }, ms);
  }
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function fail(status: number, code: string, message: string): Response {
  return json({ error: { code, message } }, status);
}

/** A generated passphrase: eight groups of four (the server's alphabet). */
function newPassphrase(): string {
  return Array.from({ length: 8 }, () =>
    Array.from(
      { length: 4 },
      () => PASS_ALPHABET[Math.floor(Math.random() * PASS_ALPHABET.length)],
    ).join(''),
  ).join('-');
}

function randomHex(n: number): string {
  let out = '';
  for (let i = 0; i < n; i++) out += Math.floor(Math.random() * 16).toString(16);
  return out;
}
