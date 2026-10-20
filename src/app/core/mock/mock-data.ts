// Dev-only: the data side of the fixture backend for `?mock=1` — moves and
// copies between accounts (`/data/move`), data jobs (`/data/jobs*`) and
// backups (`/backup*`), with the server's refusals. Never bundled in
// production builds.
//
// `?mockbackup=1` starts with a folder, a passphrase and earlier backups;
// `?mockbackup=wrong` adds a passphrase that does not open the sets.
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

const BACKUP_MODE = urlParam('mockbackup');
const UNDO_MODE = urlParam('mockundo');
const TRANSFER_MODE = urlParam('mocktransfer');

const S = 1000;
const M = 60 * S;
const H = 60 * M;
const D = 24 * H;
const UNDO_KEEP = 7 * D;
const HOST = 'marcus-mbp';
const OTHER_HOST = 'marcus-air';
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
  lastPush?: number;
  lastError?: string;
  sessions?: number;
  machines: BackupMachine[];
}

/** A session only the backup set has (pushed from another Mac). */
interface RemoteSession {
  id: string;
  cwd: string;
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

export class MockData {
  private readonly jobs = new Map<string, JobView>();
  private readonly undos = new Map<string, TransferUndo | (() => void)>();
  /** Session ids that have a copy in another account's directory (by home). */
  private readonly copies = new Map<string, Set<string>>();
  private settings: BackupSettings = { folder: null, everyHours: 0, pullOnLaunch: false };
  private passphraseSet = false;
  private lastPull?: number;
  private readonly setStates = new Map<string, SetState>();
  /** What each set holds that this Mac does not (by set account). */
  private readonly remote = new Map<string, RemoteSession[]>();
  private seq = 0;

  constructor(private readonly host: DataHost) {
    const now = Date.now();
    if (BACKUP_MODE === '1' || BACKUP_MODE === 'wrong') {
      this.settings = { folder: ICLOUD, everyHours: 6, pullOnLaunch: false };
      this.passphraseSet = true;
      this.lastPull = now - 26 * H;
      this.setStates.set(DEFAULT_ACCOUNT, {
        exists: true,
        lastPush: now - 2 * H,
        sessions: 143,
        machines: [
          { host: HOST, lastPush: now - 2 * H },
          { host: OTHER_HOST, lastPush: now - 5 * H },
        ],
      });
      this.remote.set(DEFAULT_ACCOUNT, [
        {
          id: 'b3f1c2d4-5e6f-4a7b-8c9d-0e1f2a3b4c5d',
          cwd: '/Users/marcus/dev/website',
          bytes: 412_000,
          pushedAt: now - 5 * H,
          newestMtime: now - 5 * H - 4 * M,
        },
        {
          id: 'c4a2e3f5-6a7b-4c8d-9e0f-1a2b3c4d5e6f',
          cwd: '/Users/marcus/dev/api-server',
          bytes: 1_840_000,
          pushedAt: now - 5 * H,
          newestMtime: now - 7 * H,
          action: 'update',
        },
        {
          id: 'd5b3f4a6-7b8c-4d9e-0f1a-2b3c4d5e6f7a',
          cwd: '/Users/marcus/dev/payments',
          bytes: 96_000,
          pushedAt: now - 5 * H,
          newestMtime: now - 6 * H,
          action: 'conflict',
        },
        {
          id: 'e6c4a5b7-8c9d-4e0f-1a2b-3c4d5e6f7a8b',
          cwd: '/Users/marcus/dev/legacy-cli',
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
    const job = this.start(id, plan.view.mode, plan.view.files, plan.view.bytes, sessions);
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
      host: HOST,
      sets: folder
        ? this.host.accounts().map((a) => {
            const st = this.setStates.get(a.id) ?? { exists: false, machines: [] };
            return {
              account: a.id,
              name: a.name,
              dir: `${folder}/${a.name}`,
              exists: st.exists,
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

  private passphrase(b: Record<string, unknown>): Response {
    if (b['generate'] === true && b['passphrase'] === undefined) {
      const groups = Array.from({ length: 8 }, () =>
        Array.from(
          { length: 4 },
          () => PASS_ALPHABET[Math.floor(Math.random() * PASS_ALPHABET.length)],
        ).join(''),
      );
      this.passphraseSet = true;
      return json({ passphrase: groups.join('-') });
    }
    if (typeof b['passphrase'] === 'string' && b['generate'] !== true) {
      if ([...b['passphrase']].length < 12) {
        return fail(422, 'invalid_backup', 'a passphrase has at least 12 characters');
      }
      this.passphraseSet = true;
      return json({ passphrase: null });
    }
    return fail(422, 'invalid_body', 'Send {"generate": true} or {"passphrase": "…"}.');
  }

  private prerequisites(): Response | null {
    if (!this.settings.folder) return fail(409, 'no_backup_folder', 'no backup folder is set');
    if (!this.passphraseSet)
      return fail(409, 'no_backup_passphrase', 'no backup passphrase is set');
    return null;
  }

  private push(b: Record<string, unknown>): Response {
    const pre = this.prerequisites();
    if (pre) return pre;
    const asked = typeof b['account'] === 'string' ? b['account'] : undefined;
    if (asked && !this.account(asked))
      return fail(422, 'invalid_backup', `no account ${JSON.stringify(asked)}`);
    const list = asked ? [this.account(asked) as AccountView] : this.host.accounts();
    const ids = list.map((a) => {
      const mine = this.host
        .sessions()
        .filter((s) => this.homeId(this.host.accountOf(s.id)) === this.homeId(a.id));
      const live = mine.filter((s) => s.live).map((s) => s.id);
      const archived = mine.filter((s) => !s.live).map((s) => s.id);
      const before = this.setStates.get(a.id);
      const id = this.newJobId();
      this.start(id, 'backup', archived.length, archived.length * 180_000, []);
      this.progress(id, 1_800, () => {
        const unchanged = before?.exists ? (before.sessions ?? 0) : 0;
        const now = Date.now();
        const machines = (before?.machines ?? []).filter((m) => m.host !== HOST);
        this.setStates.set(a.id, {
          exists: true,
          lastPush: now,
          sessions: unchanged + archived.length,
          machines: [{ host: HOST, lastPush: now }, ...machines],
        });
        return {
          result: {
            archived,
            unchanged,
            skippedLive: live,
            failed: [],
            config: `config/${new Date(now).toISOString()}.tar.gz.age`,
            bytes: archived.length * 180_000,
          },
        };
      });
      return id;
    });
    return json({ jobs: ids }, 202);
  }

  /** A set's sessions compared with `into`'s directory. */
  private compare(account: string, into: string): BackupIndexSession[] {
    const intoHome = this.homeId(into);
    const pull = account === into;
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
    if (!this.setStates.get(a.id)?.exists) {
      return fail(
        422,
        'invalid_backup',
        `there is no backup of ${a.name} in ${this.settings.folder}`,
      );
    }
    if (BACKUP_MODE === 'wrong') {
      return fail(403, 'wrong_passphrase', 'the passphrase does not open this backup set');
    }
    return json({
      account: a.id,
      name: a.name,
      set: `${this.settings.folder}/${a.name}`,
      machines: this.setStates.get(a.id)?.machines ?? [],
      sessions: this.compare(a.id, into),
      config: [
        {
          name: 'config/2026-10-21T08-00-00Z.tar.gz.age',
          createdAt: Date.now() - 2 * H,
          host: HOST,
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
    if (!this.setStates.get(a.id)?.exists) {
      return fail(
        422,
        'invalid_backup',
        `there is no backup of ${a.name} in ${this.settings.folder}`,
      );
    }
    if (b['config'] === true) {
      return json({ folder: `${DATA_DIR}/restored-config/${a.name}` });
    }
    const ids = Array.isArray(b['ids'])
      ? b['ids'].filter((x): x is string => typeof x === 'string')
      : [];
    if (!ids.length) return fail(422, 'invalid_body', 'Name the sessions to restore (`ids`).');
    const into = typeof b['into'] === 'string' && b['into'] ? b['into'] : account;
    if (!this.account(into))
      return fail(422, 'invalid_backup', `no account ${JSON.stringify(into)}`);
    const plan = new Map(this.compare(a.id, into).map((s) => [s.id, s]));
    const id = this.newJobId();
    this.start(id, 'restore', ids.length, 0, []);
    this.progress(id, 1_500, () => {
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

  private pull(b: Record<string, unknown>): Response {
    const pre = this.prerequisites();
    if (pre) return pre;
    const asked = typeof b['account'] === 'string' ? b['account'] : undefined;
    const wantedIds = Array.isArray(b['ids'])
      ? new Set(b['ids'].filter((x): x is string => typeof x === 'string'))
      : null;
    const list = asked ? [this.account(asked)] : this.host.accounts();
    if (asked && !list[0])
      return fail(422, 'invalid_backup', `no account ${JSON.stringify(asked)}`);
    const jobs: string[] = [];
    const skippedSets: string[] = [];
    for (const a of list as AccountView[]) {
      if (!this.setStates.get(a.id)?.exists) {
        const msg = `there is no backup of ${a.name} in ${this.settings.folder}`;
        if (asked) return fail(422, 'invalid_backup', msg);
        skippedSets.push(msg);
        continue;
      }
      if (BACKUP_MODE === 'wrong') {
        return fail(403, 'wrong_passphrase', 'the passphrase does not open this backup set');
      }
      const plan = this.compare(a.id, a.id).filter((s) => !wantedIds || wantedIds.has(s.id));
      const id = this.newJobId();
      this.start(id, 'pull', 0, 0, []);
      jobs.push(id);
      this.progress(id, 1_800, () => {
        const home = this.homeId(a.id);
        const restored = plan.filter((s) => s.action === 'new').map((s) => s.id);
        const updated = plan.filter((s) => s.action === 'update').map((s) => s.id);
        const conflicts = plan
          .filter((s) => s.action === 'conflict')
          .map((s): [string, string] => [
            s.id,
            `${this.dirOf(a)}/projects/${(s.cwd ?? '').replace(/[^a-zA-Z0-9]/g, '-')}/${s.id}.conflict-${OTHER_HOST}-${Date.now()}.jsonl`,
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
    }
    return json({ jobs, skipped: skippedSets }, 202);
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
    };
    this.jobs.set(id, job);
    return job;
  }

  /** Let a job make progress for `ms`, then finish it with `done`'s result. */
  private progress(
    id: string,
    ms: number,
    done: () => { result?: unknown; sessions?: string[]; undo?: boolean; onUndo?: () => void },
  ): void {
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
