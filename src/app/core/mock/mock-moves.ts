// Dev-only: moving sessions between computers (docs/PLAN-MACHINES.md §5.8)
// for `?mock=1&mockmachines=1`: Machines on with DESKTOP-PC (online) and
// BUILD-BOX (offline) paired, their sessions as `remote` frames, a session
// of this computer that lives on DESKTOP-PC (Open there, Bring it back
// here) and one on BUILD-BOX (Take over…), and a move that runs: the
// preflight's checks (a folder with changes of its own when the code would
// land in it for "Refactor auth middleware"), its steps as `move` frames,
// Cancel, Undo and Try again, and DESKTOP-PC's own moves through its proxy
// (asked about until they end, as the app does). Never bundled in production
// builds.

import { MovePreflight, MoveRecord, MoveState, SessionMoveBody } from '../moves';
import { ServerFrame, SessionDetail, SessionSummary, UiEvent } from '../models';
import { ev, ownedCaps } from './fixtures';
import { urlParam } from '../platform';

/** `?mockmachines=1`: Machines on, two computers paired, moves. */
export const MOCK_MACHINES = urlParam('mockmachines') === '1';

export const MOCK_SELF = '0'.repeat(32);
export const MOCK_DESKTOP = 'f'.repeat(32);
export const MOCK_BUILDBOX = 'e'.repeat(32);

const S = 1000;
const M = 60 * S;
const H = 60 * M;
const WEEK = 7 * 24 * H;

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function err(status: number, code: string, message: string, extra: object = {}): Response {
  return json({ error: { code, message, ...extra } }, status);
}

function moveId(): string {
  let hex = '';
  for (let i = 0; i < 16; i++) hex += Math.floor(Math.random() * 16).toString(16);
  return `mv_${hex}`;
}

/** What the mock needs of the fixture backend. */
export interface MovesHost {
  /** This computer's sessions (the live list). */
  sessions(): SessionSummary[];
  /** Change one of this computer's sessions and send the diff. */
  patchSession(id: string, change: Partial<SessionSummary>): void;
  /** A session new to this computer (it came from another one). */
  addSession(s: SessionSummary): void;
  broadcast(frame: ServerFrame): void;
}

interface Peer {
  id: string;
  name: string;
  os: string;
  online: boolean;
  since: number | null;
  /** Its sessions, as its frames carry them. */
  sessions: Map<string, SessionSummary>;
  /** Its own move records (served through its proxy). */
  moves: Map<string, MoveRecord>;
}

export class MockMoves {
  /** This computer's move records. */
  private readonly moves = new Map<string, MoveRecord>();
  private readonly peers: Peer[];
  private readonly timers = new Map<string, ReturnType<typeof setTimeout>>();

  constructor(private readonly host: MovesHost) {
    const now = Date.now();
    this.peers = [
      {
        id: MOCK_DESKTOP,
        name: 'DESKTOP-PC',
        os: 'windows',
        online: true,
        since: null,
        sessions: new Map(),
        moves: new Map(),
      },
      {
        id: MOCK_BUILDBOX,
        name: 'BUILD-BOX',
        os: 'linux',
        online: false,
        since: now - 40 * M,
        sessions: new Map(),
        moves: new Map(),
      },
    ];
    this.seed(now);
  }

  // ─────────────────────────────────────────────────────────── State

  /** A session of this computer moved to DESKTOP-PC two hours ago, and one
   * to BUILD-BOX, which is offline now. */
  private seed(now: number): void {
    const [desk, box] = this.peers;
    const release = this.host.sessions().find((s) => s.id === 's-release');
    if (release) {
      const there = this.copyFor(release, desk, { live: true, status: 'idle' });
      desk.sessions.set(there.id, there);
      release.livesOn = { machine: desk.id, name: desk.name, since: now - 2 * H, epoch: 1 };
      const rec = this.record(release, desk, now - 2 * H - 40 * S);
      this.walk(rec, ['freezing', 'packaging', 'sending', 'landing', 'landed', 'committed'], 3 * S);
      rec.state = 'resumed';
      rec.steps.push({ state: 'resumed', at: rec.createdAt + 25 * S });
      rec.finished = true;
      rec.finishedAt = rec.createdAt + 25 * S;
      rec.bytesSent = rec.bytesTotal;
      rec.undoUntil = rec.finishedAt + WEEK;
      rec.targetCwd = there.cwd;
      rec.targetFolder = there.cwd;
      this.moves.set(rec.id, rec);
    }
    const cache = this.host.sessions().find((s) => s.id === 's-cache');
    if (cache) {
      box.sessions.set(cache.id, this.copyFor(cache, box, { live: false, status: 'idle' }));
      cache.livesOn = { machine: box.id, name: box.name, since: now - 5 * H, epoch: 1 };
    }
    // Three days ago "Summarise this week's commits" went to DESKTOP-PC into a
    // separate folder (a worktree) and came back since: *Remove the
    // worktree* is offered.
    const haiku = this.host.sessions().find((s) => s.id === 's-haiku');
    if (haiku) {
      const rec = this.record(haiku, desk, now - 3 * 24 * H, { to: desk.id, apply: 'worktree' });
      this.walk(
        rec,
        ['freezing', 'packaging', 'sending', 'landing', 'landed', 'committed', 'resumed'],
        3 * S,
      );
      const wt = `C:\\Users\\marcus\\AppData\\Roaming\\dev.motherclaude.desktop\\machines\\worktrees\\${haiku.id}`;
      rec.finished = true;
      rec.finishedAt = rec.updatedAt;
      rec.bytesSent = rec.bytesTotal;
      rec.targetCwd = wt;
      rec.targetFolder = 'D:\\development\\projects\\mother-claude';
      rec.code = { ...rec.code!, landed: { landing: 'worktree', path: wt } };
      rec.worktree = { path: wt, removable: true, removedAt: null };
      this.moves.set(rec.id, rec);
    }
    desk.sessions.set('r-tray', {
      ...this.copyFor(
        {
          ...(this.host.sessions()[0] ?? ({} as SessionSummary)),
          id: 'r-tray',
          name: 'tray',
          title: 'Fix tray icon',
          projectName: 'mother-claude',
          cwd: '/Users/marcus/dev/mother-claude',
          lastPrompt: 'The tray icon is blurry on 150 % scaling.',
          lastMessage: 'Editing src-tauri/src/tray.rs',
        },
        desk,
        { live: true, status: 'working' },
      ),
      activity: { kind: 'tool', label: 'Editing src-tauri/src/tray.rs', tool: 'Edit' },
      pending: [],
    });
    box.sessions.set('r-installer', {
      ...this.copyFor(
        {
          ...(this.host.sessions()[0] ?? ({} as SessionSummary)),
          id: 'r-installer',
          name: 'installer',
          title: 'Build installer',
          projectName: 'mother-claude',
          cwd: '/home/marcus/src/mother-claude',
        },
        box,
        { live: false, status: 'ended' },
      ),
      pending: [],
    });
  }

  /** A session as another computer shows it. */
  private copyFor(s: SessionSummary, peer: Peer, over: Partial<SessionSummary>): SessionSummary {
    const name = s.projectName || 'project';
    const cwd =
      peer.os === 'windows' ? `D:\\development\\projects\\${name}` : `/home/marcus/src/${name}`;
    const live = over.live ?? s.live;
    return {
      ...s,
      cwd,
      surface: 'mother-claude',
      owned: true,
      control: ownedCaps(),
      account: undefined,
      livesOn: undefined,
      pending: [],
      queued: 0,
      prLink: undefined,
      todos: undefined,
      agents: { running: 0, total: 0, workflowsRunning: 0, backgroundRunning: 0 },
      activity: live ? { kind: 'idle', label: 'Idle' } : { kind: 'ended', label: 'Ended' },
      lastActivityAt: Date.now() - 4 * M,
      ...over,
    } as SessionSummary;
  }

  private record(s: SessionSummary, to: Peer, at: number, body?: SessionMoveBody): MoveRecord {
    const conversation = 18 * 1024 * 1024 + Math.round(Math.random() * 4 * 1024 * 1024);
    const code = 2 * 1024 * 1024;
    const noCode = body?.code === 'none';
    return {
      id: moveId(),
      role: 'out',
      session: s.id,
      targetSession: body?.asCopy ? crypto.randomUUID() : s.id,
      title: s.title ?? s.name,
      from: { id: MOCK_SELF, name: 'MacBook', os: 'macos' },
      to: { id: to.id, name: to.name, os: to.os },
      state: 'preflight',
      finished: false,
      asCopy: !!body?.asCopy,
      keepDriving: body?.keepDriving ?? true,
      when: body?.when ?? 'idle',
      account: { name: s.account?.name ?? 'Default', isDefault: !s.account },
      epoch: 2,
      sourceCwd: s.cwd,
      targetCwd: null,
      targetFolder: null,
      code: noCode
        ? null
        : {
            mode: body?.code ?? 'bundle',
            apply: body?.apply ?? 'checkout',
            branch: 'refs/heads/feat/windows-tray',
            changed: 8,
            untracked: 2,
            warnings: [],
          },
      files: [
        { name: 'session.tar.gz', size: conversation, sha256: '0'.repeat(64), sent: 0 },
        ...(noCode
          ? []
          : [
              { name: 'meta.json', size: 4 * 1024, sha256: '1'.repeat(64), sent: 0 },
              { name: 'move.bundle', size: code, sha256: '2'.repeat(64), sent: 0 },
            ]),
      ],
      bytesTotal: conversation + (noCode ? 0 : code + 4 * 1024),
      bytesSent: 0,
      conversationBytes: conversation,
      wasRunning: s.live && s.owned,
      heldCount: 0,
      recipe: body?.recipe ? ['npm ci'] : [],
      trust: !!body?.trust,
      steps: [{ state: 'preflight', at }],
      error: null,
      warnings: [],
      landing: null,
      resumeError: null,
      by: 'desktop',
      createdAt: at,
      updatedAt: at,
      finishedAt: null,
      undoUntil: null,
    };
  }

  /** Steps reached one after the other, `gap` apart (the seed's past). */
  private walk(rec: MoveRecord, states: MoveState[], gap: number): void {
    let at = rec.createdAt;
    for (const state of states) {
      at += gap;
      rec.steps.push({ state, at });
      rec.state = state;
    }
    rec.updatedAt = at;
  }

  private peer(id: string): Peer | undefined {
    return this.peers.find((p) => p.id === id);
  }

  // ─────────────────────────────────────────────────────────── Socket

  /** The other computers' sessions as this computer republishes them. */
  initialFrames(): ServerFrame[] {
    const out: ServerFrame[] = [];
    for (const p of this.peers) {
      out.push({
        t: 'remote',
        machine: p.id,
        frame: { t: 'sessions', full: true, sessions: [...p.sessions.values()], seq: 1 },
      });
      if (!p.online)
        out.push({ t: 'remote', machine: p.id, frame: { t: 'offline', since: p.since } });
    }
    return out;
  }

  private remoteUpsert(p: Peer, s: SessionSummary): void {
    p.sessions.set(s.id, s);
    this.host.broadcast({
      t: 'remote',
      machine: p.id,
      frame: { t: 'sessions', full: false, upsert: [s], seq: Date.now() },
    });
  }

  private publish(rec: MoveRecord): void {
    this.host.broadcast({ t: 'move', move: { ...rec } });
  }

  // ─────────────────────────────────────────────────────────── REST

  route(path: string, method: string, body: unknown): Response | null {
    const b = (body ?? {}) as SessionMoveBody;
    const proxy = /^\/machines\/([0-9a-f]{32})\/proxy(\/.*)$/.exec(path);
    if (proxy) return this.proxied(proxy[1], proxy[2], method, b);
    if (path === '/moves' && method === 'GET') {
      return json({ moves: [...this.moves.values()].sort((x, y) => y.createdAt - x.createdAt) });
    }
    const one = /^\/moves\/(mv_[0-9a-f]{16})(?:\/(cancel|undo|retry|remove-worktree))?$/.exec(path);
    if (one) {
      const rec = this.moves.get(one[1]);
      if (!rec) return err(404, 'move_not_found', 'No such move here.');
      if (!one[2]) return json(rec);
      if (method !== 'POST') return null;
      if (one[2] === 'remove-worktree') {
        if (!rec.worktree) {
          return err(409, 'no_worktree', "This move's code did not land in a separate folder.");
        }
        rec.worktree = { ...rec.worktree, removable: false, removedAt: Date.now() };
        rec.undoUntil = null;
        rec.updatedAt = Date.now();
        this.publish(rec);
        return json(rec);
      }
      return this.act(rec, one[2] as 'cancel' | 'undo' | 'retry');
    }
    const move = /^\/sessions\/([^/]+)\/move(\/preflight)?$/.exec(path);
    if (move && method === 'POST') {
      const s = this.host.sessions().find((x) => x.id === move[1]);
      if (!s) return err(404, 'session_not_found', 'No such session.');
      if (b.takeOver) return this.takeOver(s, b);
      if (s.livesOn) {
        return err(
          409,
          'session_lives_elsewhere',
          `This session lives on ${s.livesOn.name} now: open it there, or bring it back to this computer first.`,
          { machine: s.livesOn.machine, name: s.livesOn.name },
        );
      }
      const pre = this.preflight(s, b);
      if (move[2]) return json(pre);
      if (!pre.ok) {
        return err(409, 'move_blocked', `It can't move yet: ${pre.blockers[0]?.message ?? ''}`, {
          preflight: pre,
        });
      }
      return json(this.start(s, b), 202);
    }
    return null;
  }

  /** What DESKTOP-PC (or BUILD-BOX) answers through its proxy. */
  private proxied(id: string, path: string, method: string, b: SessionMoveBody): Response {
    const p = this.peer(id);
    if (!p) return err(404, 'machines_unknown', 'Not a paired computer.');
    if (!p.online) {
      return err(502, 'machine_unreachable', `${p.name} can't be reached right now: timed out`, {
        machine: id,
      });
    }
    if (path === '/moves' && method === 'GET') {
      return json({ moves: [...p.moves.values()].sort((x, y) => y.createdAt - x.createdAt) });
    }
    const one = /^\/moves\/(mv_[0-9a-f]{16})$/.exec(path);
    if (one) {
      const rec = p.moves.get(one[1]);
      return rec ? json(rec) : err(404, 'move_not_found', 'No such move here.');
    }
    const move = /^\/sessions\/([^/]+)\/move(\/preflight)?$/.exec(path);
    if (move && method === 'POST') {
      const s = p.sessions.get(move[1]);
      if (!s) return err(404, 'session_not_found', 'No such session.');
      const back = b.to === MOCK_SELF;
      const pre = this.preflightThere(p, s, b);
      if (move[2]) return json(pre);
      if (!pre.ok) {
        return err(409, 'move_blocked', `It can't move yet: ${pre.blockers[0]?.message ?? ''}`, {
          preflight: pre,
        });
      }
      return back ? json(this.bringBack(p, s), 202) : err(422, 'invalid_move', 'Not in the mock.');
    }
    const detail = /^\/sessions\/([^/]+)$/.exec(path);
    if (detail && method === 'GET') {
      const s = p.sessions.get(detail[1]);
      if (!s) return err(404, 'session_not_found', 'No such session.');
      const d: Partial<SessionDetail> = {
        ...s,
        pendingDetail: [],
        agentsDetail: [],
        workflows: [],
      };
      return json(d);
    }
    const transcript = /^\/sessions\/([^/]+)\/transcript$/.exec(path);
    if (transcript && method === 'GET') {
      const now = Date.now();
      const events: UiEvent[] = [
        ev('rm1:0', now - 30 * M, 'user', 'text', { text: 'Pick up where we left off.' }),
        ev('rm2:0', now - 29 * M, 'assistant', 'text', {
          text: `Going on on ${p.name}: the project is in ${p.sessions.get(transcript[1])?.cwd ?? 'its folder'}.`,
        }),
      ];
      return json({ events, tail: '2' });
    }
    return err(404, 'not_found', `No mock for ${method} ${path} on ${p.name}`);
  }

  // ─────────────────────────────────────────────────────────── Checks

  private preflight(s: SessionSummary, b: SessionMoveBody): MovePreflight {
    const p = this.peer(b.to);
    const name = p?.name ?? 'That computer';
    const blockers: MovePreflight['blockers'] = [];
    const warnings: MovePreflight['warnings'] = [];
    const foreign = s.live && !s.owned;
    if (foreign && !b.asCopy) {
      blockers.push({
        code: 'session_foreign',
        message: `It runs in ${s.surface} — Mother Claude can't stop it. A copy can go on on ${name} as a new session while this one goes on here, or end it there first.`,
        offer: 'as-copy',
      });
    }
    const folder = `D:\\development\\projects\\${s.projectName || 'project'}`;
    const repo = b.code !== 'none';
    if (!p?.online) {
      blockers.push({
        code: 'target_offline',
        message: `${name} can't be reached right now: timed out`,
        fix: `Check that Mother Claude runs on ${name} and that both computers are online.`,
      });
    } else {
      warnings.push({
        code: 'claude_older',
        message: `Claude Code on ${name} is older (2.1.322, here 2.1.330).`,
      });
      if (repo && b.apply !== 'worktree' && s.id === 's-auth') {
        blockers.push({
          code: 'checkout_dirty',
          message: `${folder} has changes of its own (3 files). The code can't land in the project folder.`,
          fix: 'Use a separate folder (worktree) there.',
          offer: 'worktree',
        });
      }
      // Its branch went on there (plan §5.10): a worktree, or replacing it.
      if (repo && b.apply !== 'worktree' && s.id === 's-db') {
        if (b.replaceBranch) {
          warnings.push({
            code: 'branch_replaced',
            message: `feat/windows-tray on ${name} is replaced by this computer's: its own commits there are kept for Undo move.`,
          });
        } else {
          blockers.push({
            code: 'branch_diverged',
            message: `feat/windows-tray on ${name} has commits this computer doesn't.`,
            fix: "Use a separate folder (worktree) there, or replace the branch there with this computer's (its commits are kept for Undo move).",
            offer: 'worktree-or-reset',
            detail: { branch: 'feat/windows-tray', tip: 'b'.repeat(40) },
          });
        }
      }
      if (b.folder === undefined && s.id === 's-docs') {
        blockers.push({
          code: 'folder_elsewhere',
          message: `${name} has no folder for ${s.cwd}: choose its folder there.`,
          offer: 'choose-folder',
          detail: { candidates: [`D:\\development\\projects\\docs`, `D:\\src\\docs-site`] },
        });
      }
    }
    return {
      ok: blockers.length === 0,
      blockers,
      warnings,
      session: {
        id: s.id,
        title: s.title ?? s.name,
        owned: s.owned,
        live: s.live,
        foreign,
        account: s.account ? { id: s.account.id, name: s.account.name } : null,
        cwd: s.cwd,
        bytes: 18 * 1024 * 1024,
        transcriptBytes: 16 * 1024 * 1024,
        files: 12,
        subagents: 3,
      },
      to: { id: b.to, name, os: p?.os ?? 'other', online: !!p?.online },
      code: repo
        ? {
            mode: b.code ?? 'bundle',
            apply: b.apply ?? 'checkout',
            repo: true,
            top: s.cwd,
            branch: 'refs/heads/feat/windows-tray',
            head: 'a'.repeat(40),
            changed: 8,
            untracked: 2,
            origin: 'git@github.com:marcus/app.git',
            relation: s.id === 's-db' ? 'diverged' : 'fastForward',
            replaceBranch: s.id === 's-db' && !!b.replaceBranch && b.apply !== 'worktree',
            // The files git ignores the project lists to carry (*Not in
            // git*): they travel unless unticked.
            ignored: ['.env.local', 'config/dev.secrets.json'],
            ignoredBytes: 1_842,
            carryIgnored: b.carryIgnored !== false,
          }
        : { mode: 'none', repo: true },
      target: p?.online
        ? {
            account: { id: 'default', name: s.account?.name ?? 'Default', signedIn: true },
            folder:
              s.id === 's-docs' && b.folder === undefined
                ? null
                : { path: b.folder ?? folder, how: b.folder ? 'chosen' : 'root', trusted: true },
            candidates: [],
            cwd: b.folder ?? folder,
            code: {
              repo: true,
              clean: s.id !== 's-auth',
              parked: false,
              changed: s.id === 's-auth' ? 3 : 0,
              untracked: 0,
              origin: 'git@github.com:marcus/app.git',
              branch: 'refs/heads/main',
            },
            session: { here: false, live: false },
            claude: '2.1.322',
            freeBytes: 120 * 1024 * 1024 * 1024,
          }
        : null,
      recipe: repo ? ['npm ci'] : [],
      asCopy: !!b.asCopy,
    };
  }

  /** DESKTOP-PC's preflight of a move back to this computer. */
  private preflightThere(p: Peer, s: SessionSummary, b: SessionMoveBody): MovePreflight {
    const pre = this.preflight(s, { ...b, to: b.to });
    if (b.to === MOCK_SELF) {
      pre.blockers = [];
      pre.warnings = [];
      pre.ok = true;
      pre.to = { id: MOCK_SELF, name: 'MacBook', os: 'macos', online: true };
      pre.session.cwd = s.cwd;
      if (pre.target) {
        pre.target.folder = { path: '/Users/marcus/dev/mother-claude', how: 'root', trusted: true };
        pre.target.cwd = '/Users/marcus/dev/mother-claude';
        pre.target.code = { ...pre.target.code!, clean: true, parked: true };
      } else {
        pre.target = {
          account: { id: 'default', name: 'Default', signedIn: true },
          folder: { path: '/Users/marcus/dev/mother-claude', how: 'root', trusted: true },
          candidates: [],
          cwd: '/Users/marcus/dev/mother-claude',
          code: { repo: true, clean: true, parked: true, changed: 0, untracked: 0 },
          session: { here: true, live: false },
          claude: '2.1.330',
          freeBytes: 80 * 1024 * 1024 * 1024,
        };
      }
      pre.session.bytes = 2 * 1024 * 1024;
      pre.code = { ...pre.code, changed: 1, untracked: 0 };
    } else if (!this.peer(b.to)?.online) {
      pre.ok = false;
    }
    void p;
    return pre;
  }

  // ─────────────────────────────────────────────────────────── Moving

  /** Runs a move of this computer's: each step a `move` frame. */
  private start(s: SessionSummary, b: SessionMoveBody): MoveRecord {
    const to = this.peer(b.to)!;
    const rec = this.record(s, to, Date.now(), b);
    this.moves.set(rec.id, rec);
    this.publish(rec);
    const plan: { state: MoveState; after: number }[] = [
      { state: 'freezing', after: 700 },
      { state: 'packaging', after: 1500 },
      { state: 'sending', after: 1200 },
      { state: 'landing', after: 4500 },
      { state: 'landed', after: 1400 },
      { state: 'committed', after: 600 },
      { state: 'resumed', after: 1100 },
    ];
    this.run(rec, plan, (r) => {
      if (r.state === 'sending') this.sendBytes(r);
      if (r.state === 'landed' && r.code && b.carryIgnored !== false) {
        // DESKTOP-PC had a .env.local of its own: kept for Undo move.
        r.warnings = [
          ...r.warnings,
          {
            code: 'ignored_replaced',
            message: `${to.name} had its own .env.local: kept for Undo move.`,
          },
        ];
        r.code = { ...r.code, ignored: ['.env.local', 'config/dev.secrets.json'] };
      }
      if (r.state === 'landed' && r.code && b.replaceBranch && b.apply !== 'worktree') {
        // The branch there was replaced: its own commits kept for Undo move.
        const undoRef = 'refs/mother-claude/undo/20261113T101500-3fa2';
        r.warnings = [
          ...r.warnings,
          {
            code: 'branch_replaced',
            message: `feat/windows-tray had commits of its own on ${to.name}: kept for Undo move under ${undoRef}/branch.`,
          },
        ];
        r.code = {
          ...r.code,
          replaces: 'b'.repeat(40),
          landed: { landing: 'checkout', path: r.targetCwd ?? '', action: 'reset', undoRef },
        };
      }
      if (r.state === 'committed' && !r.asCopy) {
        r.undoUntil = Date.now() + WEEK;
        this.host.patchSession(r.session, {
          livesOn: { machine: to.id, name: to.name, since: Date.now(), epoch: r.epoch },
          live: false,
          status: 'ended',
        });
      }
      if (r.state === 'resumed') {
        const there = this.copyFor(s, to, {
          id: r.targetSession ?? s.id,
          live: true,
          status: 'idle',
        });
        r.targetCwd = there.cwd;
        r.targetFolder = there.cwd;
        this.remoteUpsert(to, there);
        this.host.broadcast({
          t: 'notice',
          level: 'success',
          text: r.asCopy
            ? `A copy of ${r.title} now runs on ${to.name}.`
            : `${r.title} now runs on ${to.name}.`,
          session: r.session,
        });
      }
    });
    return rec;
  }

  /** Bytes sent while it sends (a frame every 450 ms). */
  private sendBytes(rec: MoveRecord): void {
    const tick = () => {
      if (rec.state !== 'sending') return;
      rec.bytesSent = Math.min(rec.bytesTotal, rec.bytesSent + Math.round(rec.bytesTotal / 9));
      let left = rec.bytesSent;
      for (const f of rec.files) {
        f.sent = Math.min(f.size, left);
        left -= f.sent;
      }
      rec.updatedAt = Date.now();
      this.publish(rec);
      if (rec.bytesSent < rec.bytesTotal) this.timers.set(`${rec.id}:bytes`, setTimeout(tick, 450));
    };
    this.timers.set(`${rec.id}:bytes`, setTimeout(tick, 300));
  }

  private run(
    rec: MoveRecord,
    plan: { state: MoveState; after: number }[],
    onStep: (r: MoveRecord) => void,
    publish: (r: MoveRecord) => void = (r) => this.publish(r),
  ): void {
    const next = (i: number) => {
      const step = plan[i];
      if (!step) return;
      this.timers.set(
        rec.id,
        setTimeout(() => {
          if (rec.finished) return;
          const at = Date.now();
          rec.state = step.state;
          rec.steps.push({ state: step.state, at });
          rec.updatedAt = at;
          if (step.state === 'resumed') {
            rec.finished = true;
            rec.finishedAt = at;
            rec.bytesSent = rec.bytesTotal;
          }
          if (step.state === 'landing') rec.bytesSent = rec.bytesTotal;
          onStep(rec);
          publish(rec);
          next(i + 1);
        }, step.after),
      );
    };
    next(0);
  }

  private act(rec: MoveRecord, what: 'cancel' | 'undo' | 'retry'): Response {
    const now = Date.now();
    if (what === 'cancel') {
      if (!['preflight', 'freezing', 'packaging', 'sending'].includes(rec.state)) {
        return err(
          409,
          'move_too_late',
          `It is being placed on ${rec.to.name} now: once it is there, Undo move takes it back.`,
        );
      }
      const before = { ...rec };
      clearTimeout(this.timers.get(rec.id));
      clearTimeout(this.timers.get(`${rec.id}:bytes`));
      setTimeout(() => {
        rec.error = { code: 'cancelled', message: 'It was cancelled.', step: before.state };
        rec.state = 'cancelled';
        rec.steps.push({ state: 'cancelled', at: Date.now() });
        rec.finished = true;
        rec.finishedAt = Date.now();
        rec.updatedAt = Date.now();
        this.publish(rec);
      }, 400);
      return json(before);
    }
    if (what === 'undo') {
      if (rec.asCopy || !['committed', 'resumed'].includes(rec.state)) {
        return err(409, 'move_not_undoable', 'Only a move that finished can be undone.');
      }
      rec.state = 'undone';
      rec.steps.push({ state: 'undone', at: now });
      rec.undoUntil = null;
      rec.finished = true;
      rec.updatedAt = now;
      this.host.patchSession(rec.session, { livesOn: undefined });
      const to = this.peer(rec.to.id);
      if (to) {
        to.sessions.delete(rec.targetSession ?? rec.session);
        this.host.broadcast({
          t: 'remote',
          machine: to.id,
          frame: {
            t: 'sessions',
            full: false,
            remove: [rec.targetSession ?? rec.session],
            seq: now,
          },
        });
      }
      this.publish(rec);
      this.host.broadcast({
        t: 'notice',
        level: 'success',
        text: `${rec.title} lives on this computer again: the move was undone.`,
        session: rec.session,
      });
      return json(rec);
    }
    if (!rec.resumeError) {
      return err(
        409,
        'move_not_retryable',
        'Only a move that is on the other computer but did not start there is tried again; start a new move instead.',
      );
    }
    rec.resumeError = null;
    rec.state = 'resumed';
    rec.steps.push({ state: 'resumed', at: now });
    rec.finished = true;
    rec.updatedAt = now;
    this.publish(rec);
    return json(rec);
  }

  /** DESKTOP-PC sends a session back here: its record, asked about through
   * its proxy; this computer's session lives here again at the end. */
  private bringBack(p: Peer, s: SessionSummary): MoveRecord {
    const self: Peer = {
      id: MOCK_SELF,
      name: 'MacBook',
      os: 'macos',
      online: true,
      since: null,
      sessions: new Map(),
      moves: new Map(),
    };
    const rec = this.record(s, self, Date.now(), { to: MOCK_SELF, keepDriving: false });
    rec.from = { id: p.id, name: p.name, os: p.os };
    rec.bytesTotal = 2 * 1024 * 1024;
    rec.files = [{ name: 'session.tar.gz', size: rec.bytesTotal, sha256: '0'.repeat(64), sent: 0 }];
    rec.code = null;
    p.moves.set(rec.id, rec);
    const plan: { state: MoveState; after: number }[] = [
      { state: 'freezing', after: 600 },
      { state: 'packaging', after: 1200 },
      { state: 'sending', after: 1000 },
      { state: 'landing', after: 2200 },
      { state: 'landed', after: 1000 },
      { state: 'committed', after: 500 },
      { state: 'resumed', after: 900 },
    ];
    this.run(
      rec,
      plan,
      (r) => {
        if (r.state === 'sending') r.bytesSent = Math.round(r.bytesTotal / 2);
        if (r.state === 'committed') {
          this.host.patchSession(s.id, { livesOn: undefined });
          const there = p.sessions.get(s.id);
          if (there) {
            this.remoteUpsert(p, {
              ...there,
              live: false,
              status: 'ended',
              livesOn: { machine: MOCK_SELF, name: 'MacBook', since: Date.now(), epoch: 3 },
            });
          }
        }
        if (r.state === 'resumed') {
          r.undoUntil = Date.now() + WEEK;
          // It goes on here as a session Mother Claude runs (new here when it
          // never lived here).
          const runs: Partial<SessionSummary> = {
            live: true,
            status: 'idle',
            owned: true,
            surface: 'mother-claude',
            control: ownedCaps(),
            activity: { kind: 'idle', label: 'Idle' },
            livesOn: undefined,
          };
          if (this.host.sessions().some((x) => x.id === s.id)) {
            this.host.patchSession(s.id, runs);
          } else {
            this.host.addSession({ ...s, ...runs, cwd: '/Users/marcus/dev/mother-claude' });
          }
          this.host.broadcast({
            t: 'notice',
            level: 'success',
            text: `${r.title} now runs on this computer (it came from ${p.name}).`,
            session: s.id,
          });
        }
      },
      // DESKTOP-PC does not pass its moves on: the app asks about them.
      () => undefined,
    );
    return rec;
  }

  private takeOver(s: SessionSummary, b: SessionMoveBody): Response {
    if (b.to !== MOCK_SELF) {
      return err(
        422,
        'invalid_move',
        'A take over brings it to this computer: name this computer.',
      );
    }
    const home = s.livesOn ? this.peer(s.livesOn.machine) : undefined;
    if (!home) return err(409, 'not_elsewhere', 'It lives on this computer already.');
    if (home.online) {
      return err(409, 'home_online', `${home.name} is online: bring it back here instead.`, {
        machine: home.id,
      });
    }
    this.host.patchSession(s.id, { livesOn: undefined });
    return json({
      takenOver: true,
      session: s.id,
      home: MOCK_SELF,
      from: home.id,
      epoch: (s.livesOn?.epoch ?? 1) + 1,
      warning: `It goes on here from this computer's copy. If ${home.name} went on with it while it was away, both versions are kept side by side when it next moves there.`,
    });
  }
}
