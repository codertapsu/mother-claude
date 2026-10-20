// Dev-only fixture backend for `?mock=1`: answers every /api endpoint of the
// contract and runs a fake WebSocket v2 stream, so every page can be used and
// screenshot without the Rust server. Never bundled in production builds.

import {
  ACCOUNT_COLORS,
  DEFAULT_ACCOUNT,
  accountForCwd,
  accountNameProblem,
  isAccountColor,
  needsCopy,
} from '../accounts';
import {
  AccountDefaults,
  AccountRef,
  AccountView,
  AccountsView,
  AgentInfo,
  BridgeDefaults,
  CaptureView,
  FleetSession,
  FleetSummary,
  PairCode,
  PendingItem,
  PlanUsage,
  RespondBody,
  ServerFrame,
  SessionDetail,
  SessionSummary,
  UiEvent,
} from '../models';
import {
  Fixtures,
  MOCK_SESSION_ACCOUNTS,
  buildAccountUsage,
  buildAccounts,
  buildFixtures,
  ev,
  ownedCaps,
  toPreview,
} from './fixtures';
import { MockBackend, MockRole, WebSocketLike, mockRole } from './mock-api';
import { MockData } from './mock-data';
import { urlParam } from '../platform';

const TOKEN = 'mock-token';
const TICK_MS = 2500;

export function createMockBackend(): MockBackend {
  const backend = new FixtureBackend(mockRole());
  // Dev-only hook for screenshots and manual testing (e.g. from DevTools:
  // `__mcMock.resolveElsewhere('req-db-1')`).
  (window as unknown as { __mcMock?: FixtureBackend }).__mcMock = backend;
  return backend;
}

/** `?mockws=fail`: the socket never connects (exercise the polling mode). */
const WS_FAILS = urlParam('mockws') === 'fail';

/** Plan usage states (`?mockusage=`):
 * - `off`     an account without plan usage (an API key);
 * - `none`    an older server without /usage and its frame;
 * - `stream`  only a running session's report so far (5-hour and weekly);
 * - `mixed`   a full read 12 min ago, and a running session's report of the
 *             5-hour and weekly windows since (`streamAt`);
 * - `error`   the last read failed: Claude Code's cached reading, the error,
 *             and no scheduled read while the Mac backs off;
 * - `empty`   plan usage exists but hasn't been read yet.
 * A refresh (or a scheduled read) then reads the full set. */
const USAGE_MODE = urlParam('mockusage');

/** Intervals the server accepts for the plan-usage refresh (seconds). */
const PLAN_INTERVALS = [0, 180, 300, 600];

/** Claude accounts (`?mockaccounts=`):
 * - absent  only the Default account (every setup before accounts);
 * - `1`     plus "Work" (its own directory) and "Marcus (Pro)" (a token);
 * - `limit` the same, with the Default at its 5-hour limit (the offer to
 *           continue on another account shows).
 * `?mockcapture=code|fail`: the token sign-in asks for the browser's code,
 * or ends without a token. */
const ACCOUNTS_MODE = urlParam('mockaccounts');
const CAPTURE_MODE = urlParam('mockcapture');
/** What a token looks like (a pasted one is "checked" against it). */
const TOKEN_RE = /^sk-ant-oat\d\d-[\w-]{8,}$/;

const STREAM_TEXT =
  "I cached the compiled pattern in a module-level constant and switched the tokenizer to a single pass. On the 40k-document fixture the build drops from **38.2 s** to **14.9 s** (2.6x). Next I'll check memory, since the cache keeps one regex per language.";

class FixtureBackend implements MockBackend {
  readonly token: string;
  private fx: Fixtures;
  private readonly sockets = new Set<FakeSocket>();
  private seq = 1;
  private tick = 0;
  private timer?: ReturnType<typeof setInterval>;
  private pairPolls = new Map<string, number>();
  private eventCounter = 0;
  private streamTimer?: ReturnType<typeof setInterval>;
  private streamPos = 0;
  private streamMsg = 0;
  private streamPause = 0;
  /** The last read of the (fake) account: refreshes within 60 s get a 429. */
  private lastUsageRead = 0;
  /** What a full read of the (fake) account returns. */
  private readonly fullUsage: PlanUsage;
  /** Which account each session runs under (absent: the Default). */
  private readonly sessionAccounts = new Map<string, string>();
  /** Token accounts' `setup-token` runs. */
  private readonly captures = new Map<string, CaptureView>();
  /** When each other account's usage was last read (429 within a minute). */
  private readonly accountReads = new Map<string, number>();
  /** Moves, copies, data jobs and backups (`/data/*`, `/backup*`). */
  private readonly data: MockData;

  constructor(readonly role: MockRole) {
    const now = Date.now();
    this.fx = buildFixtures(now);
    this.data = new MockData({
      desktop: role === 'desktop',
      sessions: () => [...this.fx.sessions, ...this.fx.history],
      accounts: () => this.fx.accounts.accounts,
      accountOf: (id) => this.sessionAccounts.get(id) ?? DEFAULT_ACCOUNT,
      reassign: (id, account) => {
        this.sessionAccounts.set(id, account);
        this.syncSessionAccounts(true);
      },
    });
    if (ACCOUNTS_MODE === '1' || ACCOUNTS_MODE === 'limit') {
      this.fx.accounts = buildAccounts(now, true);
      this.fx.accountUsage = buildAccountUsage(now);
      for (const [sid, acc] of Object.entries(MOCK_SESSION_ACCOUNTS)) {
        this.sessionAccounts.set(sid, acc);
      }
      if (ACCOUNTS_MODE === 'limit') {
        this.fx.planUsage = {
          ...this.fx.planUsage,
          windows: this.fx.planUsage.windows.map((w) =>
            w.key === 'five_hour' ? { ...w, pct: 100, severity: 'critical' } : w,
          ),
        };
      }
    }
    this.syncSessionAccounts(false);
    this.token = role === 'unpaired' ? '' : TOKEN;
    this.lastUsageRead = this.fx.planUsage.fetchedAt ?? 0;
    this.fullUsage = this.fx.planUsage;
    const u = this.fx.planUsage;
    if (USAGE_MODE === 'off') {
      this.fx.planUsage = {
        available: false,
        windows: [],
        refreshing: false,
        intervalSecs: 300,
      };
    } else if (USAGE_MODE === 'stream') {
      this.lastUsageRead = 0;
      this.fx.planUsage = {
        available: true,
        windows: u.windows.filter((w) => w.key === 'five_hour' || w.key === 'seven_day'),
        fetchedAt: now - 40 * 1000,
        source: 'stream',
        refreshing: false,
        nextRefreshAt: now + 4 * 60_000,
        intervalSecs: u.intervalSecs,
      };
    } else if (USAGE_MODE === 'mixed') {
      // As the server merges a report: the 5-hour window moved (the CLI's
      // severity went with the old percentage), dated `streamAt`; the rest
      // keeps the full read's time and source.
      this.lastUsageRead = now - 12 * 60_000;
      this.fx.planUsage = {
        ...u,
        windows: u.windows.map((w) =>
          w.key === 'five_hour'
            ? { key: w.key, label: w.label, pct: w.pct + 3, resetsAt: w.resetsAt }
            : w,
        ),
        fetchedAt: now - 12 * 60_000,
        streamAt: now - 30 * 1000,
      };
    } else if (USAGE_MODE === 'error') {
      this.lastUsageRead = 0;
      this.fx.planUsage = {
        ...u,
        fetchedAt: now - 50 * 60_000,
        source: 'cache',
        nextRefreshAt: undefined,
        error: 'Usage endpoint is rate limited. Please try again in a moment.',
      };
    } else if (USAGE_MODE === 'empty') {
      this.lastUsageRead = 0;
      this.fx.planUsage = {
        available: true,
        windows: [],
        refreshing: false,
        intervalSecs: u.intervalSecs,
      };
    }
  }

  // ─────────────────────────────────────────────────────────────── REST

  async fetch(url: string, init: RequestInit = {}): Promise<Response> {
    await delay(80 + Math.random() * 120);
    const u = new URL(url, location.origin);
    const path = u.pathname.replace(/^\/api/, '');
    const method = (init.method ?? 'GET').toUpperCase();
    const headers = new Headers(init.headers);
    const body = parseBody(init.body);
    const q = u.searchParams;
    const authed = headers.get('Authorization') === `Bearer ${TOKEN}`;

    if (!path.startsWith('/pair/exchange') && !path.startsWith('/pair/pending') && !authed) {
      return json({ error: { code: 'unauthorized', message: 'Pair this device first.' } }, 401);
    }

    const seg = path.split('/').filter(Boolean);
    const desktop = this.role === 'desktop';
    const deskOnly = () =>
      json({ error: { code: 'desktop_only', message: 'Only the desktop app can do this.' } }, 403);

    // /me
    if (path === '/me') return json(this.me());

    // Moves, copies, data jobs, backups
    const data = this.data.route(path, method, body, q);
    if (data) return data;

    // Sessions
    if (path === '/sessions' && method === 'GET') {
      if (q.get('scope') === 'all') {
        const all = [...this.fx.sessions, ...this.fx.history].sort(
          (a, b) => (b.lastActivityAt ?? 0) - (a.lastActivityAt ?? 0),
        );
        const start = Number(q.get('before') ?? 0) || 0;
        const limit = Number(q.get('limit') ?? 25) || 25;
        const page = all.slice(start, start + limit);
        const next = start + limit < all.length ? String(start + limit) : undefined;
        return json({ sessions: page, ...(next ? { nextBefore: next } : {}) });
      }
      const etag = `"${this.seq}"`;
      if (headers.get('If-None-Match') === etag) return new Response(null, { status: 304 });
      return json({ sessions: this.fx.sessions }, 200, { etag });
    }
    if (path === '/sessions' && method === 'POST') {
      const refusal = this.accountRefusal((body as { account?: unknown }).account);
      return refusal ?? json(this.spawn(body), 201);
    }
    if (path === '/agents') return json({ sessions: this.fleet(), summary: this.fleetSummary() });
    if (path === '/inbox') return json({ items: this.pending() });
    if (path === '/projects') return json({ projects: this.fx.projects });
    if (path === '/defaults') return json(this.fx.defaults);
    if (path === '/usage' || path.startsWith('/usage/')) {
      if (USAGE_MODE === 'none') {
        return json(
          { error: { code: 'not_found', message: `No mock for ${method} ${path}` } },
          404,
        );
      }
      if (path === '/usage' && method === 'GET') {
        return json(
          this.multi() ? { ...this.fx.planUsage, accounts: this.otherUsage() } : this.fx.planUsage,
        );
      }
      if (path === '/usage/refresh' && method === 'POST') {
        const account = (body as { account?: unknown }).account;
        return typeof account === 'string' && account && account !== DEFAULT_ACCOUNT
          ? this.refreshAccountUsage(account)
          : this.refreshUsage();
      }
      if (path === '/usage/settings' && method === 'PUT') {
        if (!desktop) return deskOnly();
        const secs = (body as { intervalSecs?: unknown }).intervalSecs;
        if (typeof secs !== 'number' || !PLAN_INTERVALS.includes(secs)) {
          return json(
            {
              error: {
                code: 'invalid_interval',
                message: 'intervalSecs must be 0, 180, 300 or 600.',
                field: 'intervalSecs',
              },
            },
            422,
          );
        }
        this.setUsage({
          intervalSecs: secs,
          nextRefreshAt: secs ? Date.now() + secs * 1000 : undefined,
        });
        return json(this.fx.planUsage);
      }
    }
    if (path === '/services') {
      return json({
        mcpServers: { github: {}, linear: {}, playwright: {} },
        daemon: { reachable: true, raw: 'claude daemon 2.1.282 · pid 4411 · up 3d 4h' },
        // Like the server: the session's id, the daemon's short job id.
        backgroundJobs: [
          { id: 's-audit', jobId: 'k7m2', cwd: '/Users/marcus/dev/monorepo', state: 'working' },
        ],
      });
    }

    if (seg[0] === 'sessions' && seg[1]) {
      const id = decodeURIComponent(seg[1]);
      const s = this.find(id);
      const sub = seg.slice(2).join('/');
      if (!s && sub !== 'media') {
        return json({ error: { code: 'not_found', message: 'No such session.' } }, 404);
      }
      if (!sub && s) return json(this.detail(s));
      if (sub === 'transcript') return json(this.transcript(id, q.get('agent') ?? undefined, q));
      if (sub === 'uploads' && method === 'POST') {
        const name = String((body as { name?: string }).name ?? 'file');
        return json(
          { path: `/Users/marcus/Library/Application Support/Mother Claude/uploads/${id}/${name}` },
          201,
        );
      }
      if (sub === 'mcp') {
        return json({
          servers: [
            { name: 'github', status: 'connected' },
            { name: 'linear', status: 'connected' },
            { name: 'playwright', status: 'failed', error: 'npx exited with code 1' },
          ],
        });
      }
      if (sub.startsWith('media/')) return svgImage();
      if (sub === 'agents') {
        return json({ agents: this.fx.agents[id] ?? [], workflows: this.fx.workflows[id] ?? [] });
      }
      if (sub === 'diff') return json(this.fx.diff);
      if (sub === 'file-patch') {
        return json({
          patch:
            '@@ -1,4 +1,6 @@\n import { Router } from "express";\n-import { validateSession } from "../auth";\n+import { requireSession } from "../middleware/session";\n \n-router.get("/me", (req, res) => {\n+router.use(requireSession());\n+router.get("/me", (req, res) => {\n',
        });
      }
      if (sub === 'files')
        return json({ files: ['src/index.ts', 'src/middleware/session.ts', 'README.md'] });
      if (sub === 'commands') {
        return json({
          commands: [
            { name: 'compact', description: 'Summarise the conversation to free context' },
            { name: 'context', description: 'Show context usage' },
            { name: 'model', description: 'Switch model', argumentHint: '[model]' },
          ],
        });
      }
      if (sub === 'terminal') {
        return json({ text: this.fx.terminals[id] ?? '$ claude\n> Working on it…\n' });
      }
      if (sub.startsWith('tasks/') && sub.endsWith('/output')) {
        const lines = Array.from(
          { length: 18 + (this.tick % 6) },
          (_, i) => `  ✓ test ${String(i + 1).padStart(3, '0')} passed (${(i * 7) % 90} ms)`,
        );
        return new Response(
          ` RUN  v3.2.4 /Users/marcus/dev/api-server\n\n${lines.join('\n')}\n\n Test Files  24 passed (24)\n      Tests  142 passed (142)\n`,
          { status: 200, headers: { 'content-type': 'text/plain' } },
        );
      }
      if (method === 'POST' && sub === 'respond' && s) return this.respond(s, body as RespondBody);
      if (method === 'POST' && sub === 'message' && s) return json(this.message(s, body));
      if (method === 'POST' && sub === 'continue' && s) return this.continueSession(s, body);
      if (method === 'POST' && sub === 'interrupt' && s) {
        this.patch(s.id, { status: 'idle', activity: { kind: 'idle', label: 'Interrupted' } });
        return empty();
      }
      if (method === 'POST' && sub === 'mode' && s) {
        this.patch(s.id, { permissionMode: String((body as { mode?: string }).mode ?? 'default') });
        return empty();
      }
      if (method === 'POST' && sub === 'model' && s) return empty();
      if (method === 'POST' && sub === 'effort' && s) {
        const effort = String((body as { effort?: string }).effort ?? '');
        if (!s.owned) {
          this.patch(s.id, { effort });
          return empty();
        }
        // Like the server: requested now, applied once Claude Code confirms.
        this.patch(s.id, { effort, effortSource: 'requested' });
        setTimeout(() => this.patch(s.id, { effortSource: 'applied' }), 1500);
        return empty();
      }
      if (method === 'POST' && (sub === 'ultracode' || sub === 'thinking') && s) {
        return this.setSwitch(s, sub, (body as { enabled?: unknown }).enabled === true);
      }
      if (method === 'POST' && sub === 'rename' && s) {
        this.patch(s.id, { title: String((body as { name?: string }).name ?? s.title) });
        return empty();
      }
      if (method === 'POST' && sub.startsWith('tasks/') && sub.endsWith('/stop')) {
        const taskId = decodeURIComponent(seg[3] ?? '');
        this.stopAgent(id, taskId);
        return empty();
      }
      if (method === 'DELETE' && sub.startsWith('queue/')) {
        const mid = decodeURIComponent(seg[3] ?? '');
        this.fx.queued[id] = (this.fx.queued[id] ?? []).filter((m) => m.id !== mid);
        this.patch(id, { queued: this.fx.queued[id].length });
        return empty();
      }
      if (method === 'POST' && sub === 'stop' && s?.owned && s.live) {
        // Like the server: the process goes, and the session is let go of.
        setTimeout(
          () =>
            this.patch(s.id, {
              live: false,
              status: 'stopped',
              activity: { kind: 'ended', label: 'Stopped' },
            }),
          400,
        );
        return empty();
      }
      if (method === 'POST' && ['stop', 'respawn', 'rm', 'keys'].includes(sub)) return empty();
      return json({ error: { code: 'not_found', message: `No mock for ${method} ${path}` } }, 404);
    }

    // Accounts
    if (path === '/accounts' && method === 'GET') return json(this.accountsView(desktop, true));
    if (path === '/accounts' && method === 'POST')
      return desktop ? this.addAccount(body) : deskOnly();
    if (path === '/accounts/defaults' && method === 'PUT') {
      return desktop ? this.putAccountDefaults(body) : deskOnly();
    }
    if (path === '/accounts/copy-consent' && method === 'PUT') {
      return desktop ? this.putCopyConsent(body) : deskOnly();
    }
    if (seg[0] === 'accounts' && seg[1]) {
      return this.accountRoute(decodeURIComponent(seg[1]), seg.slice(2).join('/'), method, body);
    }

    // Pairing
    if (path === '/pair/codes' && method === 'POST') {
      if (!desktop) return deskOnly();
      const target = (body as { target?: string }).target === 'internet' ? 'internet' : 'lan';
      const baseUrl =
        (body as { baseUrl?: string }).baseUrl ||
        (target === 'lan'
          ? this.fx.remote.lan.addresses[0].url
          : this.fx.remote.internet.publicUrl);
      if (!baseUrl) {
        return json(
          { error: { code: 'no_public_url', message: 'No public URL is known yet.' } },
          409,
        );
      }
      const code = randomCode();
      const out: PairCode = {
        code,
        url: `${baseUrl}/#/pair?c=${code}`,
        svg: fakeQr(code),
        expiresAt: Date.now() + 5 * 60_000,
        target,
        baseUrl,
      };
      return json(out, 201);
    }
    if (path === '/pair/exchange' && method === 'POST') {
      const code = String((body as { code?: string }).code ?? '');
      if (code.startsWith('0000')) {
        return json({ error: { code: 'invalid_code', message: 'Unknown or expired code.' } }, 404);
      }
      if (code.startsWith('1111')) {
        return json({ deviceId: 'dev-new', token: TOKEN, name: 'New device' }, 201);
      }
      const pendingId = `pend-${code.slice(0, 4)}`;
      this.pairPolls.set(pendingId, 0);
      return json({ pendingId, pollSecret: 'secret', fingerprint: '4821' }, 202);
    }
    if (seg[0] === 'pair' && seg[1] === 'pending' && seg[2]) {
      const n = (this.pairPolls.get(seg[2]) ?? 0) + 1;
      this.pairPolls.set(seg[2], n);
      return json(
        n >= 3 ? { status: 'approved', token: TOKEN, deviceId: 'dev-new' } : { status: 'pending' },
      );
    }

    // Devices
    if (path === '/devices') {
      if (!desktop) return deskOnly();
      return json({ devices: this.fx.devices, pending: this.fx.pendingPairs });
    }
    if (seg[0] === 'devices' && seg[1] === 'pending' && seg[2]) {
      if (!desktop) return deskOnly();
      const p = this.fx.pendingPairs.find((x) => x.id === seg[2]);
      this.fx.pendingPairs = this.fx.pendingPairs.filter((x) => x.id !== seg[2]);
      if (p && seg[3] === 'approve') {
        this.fx.devices.push({
          id: `dev-${p.id}`,
          name: p.deviceName ?? 'Device',
          platform: p.platform,
          createdAt: Date.now(),
          lastSeenAt: Date.now(),
          lastIp: p.ip,
          ingress: p.ingress,
          allowDangerous: false,
        });
      }
      this.broadcast({ t: 'devices' });
      return empty();
    }
    if (seg[0] === 'devices' && seg[1]) {
      if (!desktop) return deskOnly();
      if (method === 'DELETE') {
        this.fx.devices = this.fx.devices.filter((d) => d.id !== seg[1]);
        return empty();
      }
      if (method === 'PATCH') {
        this.fx.devices = this.fx.devices.map((d) =>
          d.id === seg[1] ? { ...d, ...(body as object) } : d,
        );
        return json(this.fx.devices.find((d) => d.id === seg[1]) ?? null);
      }
    }

    // Remote
    if (path === '/remote') {
      if (!desktop) return deskOnly();
      if (method === 'PUT') {
        const b = body as {
          internetEnabled?: boolean;
          manualUrl?: string;
          requireApproval?: boolean;
        };
        const i = this.fx.remote.internet;
        if (b.internetEnabled !== undefined) i.enabled = b.internetEnabled;
        if (b.requireApproval !== undefined) i.requireApproval = b.requireApproval;
        if (b.manualUrl !== undefined) i.manualUrl = b.manualUrl || undefined;
        i.publicUrl = i.enabled ? i.manualUrl || i.detected[0]?.url : undefined;
      }
      return json(this.fx.remote);
    }

    // Foreign control
    if (path === '/control/foreign') return desktop ? json(this.fx.foreign) : deskOnly();
    if (path === '/control/foreign/install' || path === '/control/foreign/uninstall') {
      if (!desktop) return deskOnly();
      await delay(900);
      this.fx.foreign = {
        ...this.fx.foreign,
        installed: path.endsWith('install') && !path.endsWith('uninstall'),
        pluginVersion: '1.0.0',
      };
      return json(this.fx.foreign);
    }

    // Push
    if (path === '/push/vapid') {
      return json({
        publicKey:
          'BEl62iUYgUivxIkv69yViEuiBIa-Ib9-SkvMeAtA3LFgDzkrxZJjSgSnfckjBJuBkr3qBUYIHBQFLXYp5Nksh8U',
      });
    }
    if (path === '/push/prefs') {
      if (method === 'PUT') this.fx.pushPrefs = { ...this.fx.pushPrefs, ...(body as object) };
      return json(this.fx.pushPrefs);
    }
    if (path.startsWith('/push/')) return empty();

    // Bridge
    if (path === '/bridge') return json(this.fx.bridge);
    if (path === '/bridge/start') {
      const defaults = (body as { defaults?: BridgeDefaults }).defaults ?? {};
      if (defaults.account && !this.account(defaults.account)) {
        return json(
          {
            error: {
              code: 'invalid_field',
              message: `No account "${defaults.account}" — pick another, or leave it unset.`,
              field: 'account',
            },
          },
          422,
        );
      }
      this.fx.bridge = { ...this.fx.bridge, running: true, startedAt: Date.now(), defaults };
      return json(this.fx.bridge);
    }
    if (path === '/bridge/stop') {
      this.fx.bridge = { ...this.fx.bridge, running: false };
      return json(this.fx.bridge);
    }
    if (path === '/bridge/active-thread') return json(this.fx.bridge);

    return json({ error: { code: 'not_found', message: `No mock for ${method} ${path}` } }, 404);
  }

  // ─────────────────────────────────────────────────────────── Socket

  openSocket(): WebSocketLike {
    const socket = new FakeSocket(this, WS_FAILS);
    this.sockets.add(socket);
    return socket;
  }

  /** Called by a socket once its auth frame was accepted. */
  onAuthed(socket: FakeSocket, token: string): void {
    if (token !== TOKEN) {
      socket.serverClose(4401, 'unauthorized');
      return;
    }
    socket.emit({ t: 'hello', v: 2, me: this.me(), serverTime: Date.now() });
    socket.emit({ t: 'sessions', full: true, sessions: this.fx.sessions, seq: this.seq });
    if (USAGE_MODE !== 'none') socket.emit({ t: 'planUsage', usage: this.fx.planUsage });
    // Like the server: accounts only once there is more than the Default.
    if (this.multi()) {
      socket.emit({ t: 'accounts', accounts: this.accountsView(false, false) });
      for (const [accountId, usage] of Object.entries(this.otherUsage())) {
        socket.emit({ t: 'accountUsage', accountId, usage });
      }
    }
    this.ensureTicker();
  }

  onSub(socket: FakeSocket, session: string, agent?: string): void {
    if (agent) return;
    socket.emit({
      t: 'agents',
      session,
      agents: this.fx.agents[session] ?? [],
      workflows: this.fx.workflows[session] ?? [],
    });
    for (const item of this.fx.pendingItems.filter((p) => p.sessionId === session)) {
      socket.emit({ t: 'pending', item });
    }
  }

  onSubInbox(socket: FakeSocket): void {
    for (const item of this.fx.pendingItems) socket.emit({ t: 'pending', item });
  }

  onSubFleet(socket: FakeSocket): void {
    socket.emit({ t: 'fleet', sessions: this.fleet(), summary: this.fleetSummary() });
  }

  forget(socket: FakeSocket): void {
    this.sockets.delete(socket);
    if (!this.sockets.size) {
      clearInterval(this.timer);
      clearInterval(this.streamTimer);
      this.timer = undefined;
      this.streamTimer = undefined;
    }
  }

  private ensureTicker(): void {
    if (this.timer) return;
    this.timer = setInterval(() => this.onTick(), TICK_MS);
    this.streamTimer = setInterval(() => this.streamStep(), 140);
  }

  /** s-stream writes a message token by token (owned `stream` frames),
   * then flushes it to the transcript under the same messageId. */
  private streamStep(): void {
    const subs = [...this.sockets].filter((x) => x.authed && x.subs.has('s-stream'));
    if (!subs.length) return;
    if (this.streamPause > 0) {
      this.streamPause -= 1;
      return;
    }
    const messageId = `msg_live_${this.streamMsg}`;
    const words = STREAM_TEXT.split(/(?<= )/);
    const emit = (f: ServerFrame) => subs.forEach((x) => x.emit(f));
    if (this.streamPos === 0) {
      emit({ t: 'stream', session: 's-stream', messageId, kind: 'status', status: 'thinking' });
      emit({
        t: 'stream',
        session: 's-stream',
        messageId,
        index: 0,
        kind: 'thinking',
        delta: 'The regex is compiled per document; hoisting it should remove most of the cost.',
      });
    }
    if (this.streamPos < words.length) {
      emit({
        t: 'stream',
        session: 's-stream',
        messageId,
        index: 1,
        kind: 'text',
        delta: words[this.streamPos],
      });
      this.streamPos += 1;
      return;
    }
    emit({ t: 'stream', session: 's-stream', messageId, kind: 'status', status: 'message_stop' });
    const now = Date.now();
    this.append('s-stream', [
      ev(`${messageId}:1`, now, 'assistant', 'text', { messageId, text: STREAM_TEXT }),
    ]);
    this.streamPos = 0;
    this.streamMsg += 1;
    this.streamPause = 45;
  }

  private broadcast(frame: ServerFrame): void {
    for (const s of this.sockets) if (s.authed) s.emit(frame);
  }

  /** Make the dashboard look alive: activity, tokens, transcript, agents. */
  private onTick(): void {
    this.tick += 1;
    const now = Date.now();
    const tools = [
      { tool: 'Bash', detail: 'npm audit --json' },
      { tool: 'Read', detail: 'package-lock.json' },
      { tool: 'Grep', detail: '"lodash"' },
      { tool: 'Bash', detail: 'npm outdated --long' },
    ];
    const t = tools[this.tick % tools.length];
    const audit = this.find('s-audit');
    if (audit) {
      this.patch('s-audit', {
        activity: {
          kind: 'tool',
          label: `Running ${t.tool}`,
          tool: t.tool,
          detail: t.detail,
          since: now,
        },
        lastActivityAt: now,
        costUsd: (audit.costUsd ?? 0) + 0.004,
        usage: { ...audit.usage, output: audit.usage.output + 420 },
      });
    }
    const docs = this.find('s-docs');
    if (docs && this.tick % 2 === 0) {
      this.patch('s-docs', { lastActivityAt: now, costUsd: (docs.costUsd ?? 0) + 0.01 });
    }

    // Agents drift a little.
    for (const [sid, list] of Object.entries(this.fx.agents)) {
      this.fx.agents[sid] = list.map((a) =>
        a.status === 'running' ? { ...a, tokens: (a.tokens ?? 0) + 350, lastActivityAt: now } : a,
      );
    }
    for (const s of this.sockets) {
      if (!s.authed) continue;
      if (s.fleet) s.emit({ t: 'fleet', sessions: this.fleet(), summary: this.fleetSummary() });
      for (const sid of s.subs) {
        s.emit({
          t: 'agents',
          session: sid,
          agents: this.fx.agents[sid] ?? [],
          workflows: this.fx.workflows[sid] ?? [],
        });
      }
    }

    // New transcript lines for a working session every few ticks.
    if (this.tick % 3 === 0) {
      const id = `live-${++this.eventCounter}`;
      this.append('s-audit', [
        ev(`${id}-t`, now, 'assistant', 'tool', {
          tool: { id: `toolu_${id}`, name: t.tool, summary: t.detail },
        }),
        ev(`${id}-r`, now, 'user', 'result', {
          result: { toolUseId: `toolu_${id}`, isError: false, text: 'ok' },
        }),
      ]);
    }

    // Live subagent transcripts for `sub{session, agent}`.
    if (this.tick % 2 === 0) {
      const agentEvents: UiEvent[] = [
        ev(`agx-${this.tick}:0`, now, 'assistant', 'tool', {
          agentId: 'ag-explore',
          tool: {
            id: `toolu_agx_${this.tick}`,
            name: 'Read',
            summary: `src/legacy/handler-${this.tick}.ts`,
          },
        }),
      ];
      const list = (this.fx.agentTranscripts['ag-explore'] ??= []);
      list.push(...agentEvents);
      for (const sock of this.sockets) {
        if (sock.authed && sock.agentSubs.has('s-auth/ag-explore')) {
          sock.emit({
            t: 'transcript',
            session: 's-auth',
            agent: 'ag-explore',
            events: agentEvents,
            tail: String(list.length),
          });
        }
      }
    }

    // The server's scheduled plan-usage read comes due.
    const u = this.fx.planUsage;
    const due = !!u.nextRefreshAt && u.nextRefreshAt <= now && !u.refreshing;
    if (USAGE_MODE !== 'none' && u.available && due) {
      this.readUsage();
    }

    if (this.tick % 10 === 0) this.broadcast({ t: 'ping' });
  }

  /** `POST /sessions/{id}/ultracode|thinking`, answered as the server does:
   * 409 outside live owned sessions (by ownership, not by the caps), 422
   * with Claude Code's refusal, 409 on a session that takes no control
   * requests (the SDK sidecar). */
  private async setSwitch(
    s: SessionSummary,
    sub: 'ultracode' | 'thinking',
    on: boolean,
  ): Promise<Response> {
    const what = sub === 'ultracode' ? 'Ultracode' : 'Thinking';
    if (!s.owned || !s.live) {
      return json(
        {
          error: {
            code: 'not_supported',
            message: `${what} can only be switched in a session Mother Claude owns: continue this session here first.`,
          },
        },
        409,
      );
    }
    if (!s.control.setThinking) {
      return json(
        {
          error: {
            code: 'not_supported',
            message: 'This session takes no control requests.',
          },
        },
        409,
      );
    }
    await delay(300);
    if (sub === 'thinking') {
      const adaptive = s.model?.id !== 'claude-haiku-4-5';
      this.patch(s.id, {
        thinking: on
          ? adaptive
            ? { mode: 'adaptive', source: 'control' }
            : { mode: 'on', source: 'control', budget: 31_999 }
          : { mode: 'off', source: 'control' },
      });
      return empty();
    }
    // Ultracode: turning it on needs a model and workspace that can run it
    // (the ones whose caps offer the switch); withdrawing always works.
    if (on && s.control.setUltracode === false) {
      return json(
        {
          error: {
            code: 'cli_error',
            message: "Ultracode isn't available: it needs a model that supports it and workflows.",
            errorCode: 'ultracode_unavailable',
          },
        },
        422,
      );
    }
    const cur = this.find(s.id) ?? s;
    // A withdrawn request leaves nothing to switch on this setup.
    const withdrawn = !on && cur.ultracode === 'unavailable';
    // Like the server: requested now, applied once Claude Code confirms.
    this.patch(s.id, {
      ultracode: on ? 'on' : 'off',
      ultracodeSource: 'requested',
      ...(withdrawn ? { control: { ...cur.control, setUltracode: false } } : {}),
    });
    setTimeout(() => this.patch(s.id, { ultracodeSource: 'applied' }), 1200);
    return empty();
  }

  // ──────────────────────────────────────────────────────── Plan usage

  private refreshUsage(): Response {
    const since = Date.now() - this.lastUsageRead;
    if (since < 60_000) {
      const wait = Math.ceil((60_000 - since) / 1000);
      return json(
        { error: { code: 'rate_limited', message: `Try again in ${wait}s.`, retryAfter: wait } },
        429,
        { 'retry-after': String(wait) },
      );
    }
    this.setUsage({ refreshing: true });
    setTimeout(() => this.readUsage(), 1200);
    return json(this.fx.planUsage, 202);
  }

  /** A read of the (fake) account: every window (a report from a running
   * session has only two), the session window creeping up. */
  private readUsage(): void {
    const now = Date.now();
    this.lastUsageRead = now;
    const u = this.fx.planUsage;
    const full = this.fullUsage;
    const windows = full.windows.map((w) => u.windows.find((c) => c.key === w.key) ?? w);
    this.setUsage({
      windows: windows.map((w) =>
        w.key === 'five_hour' ? { ...w, pct: Math.min(100, w.pct + 1) } : w,
      ),
      extraUsage: u.extraUsage ?? full.extraUsage,
      fetchedAt: now,
      source: 'live',
      // Every window is this fresh now (the server sends `streamAt` only
      // while it is newer than `fetchedAt`).
      streamAt: undefined,
      refreshing: false,
      nextRefreshAt: u.intervalSecs ? now + u.intervalSecs * 1000 : undefined,
      error: undefined,
    });
  }

  private setUsage(change: Partial<PlanUsage>): void {
    const next: PlanUsage = { ...this.fx.planUsage, ...change };
    for (const k of Object.keys(next) as (keyof PlanUsage)[]) {
      if (next[k] === undefined) delete next[k];
    }
    this.fx.planUsage = next;
    this.broadcast({ t: 'planUsage', usage: next });
  }

  // ───────────────────────────────────────────────────────────── State

  private me() {
    return this.role === 'desktop'
      ? {
          principal: 'desktop' as const,
          ingress: 'loopback' as const,
          local: true,
          allowDangerous: true,
        }
      : {
          principal: 'device' as const,
          deviceId: 'dev-iphone',
          name: "Marcus's iPhone · Safari",
          ingress: 'lan' as const,
          local: false,
          allowDangerous: false,
        };
  }

  private find(id: string): SessionSummary | undefined {
    return this.fx.sessions.find((s) => s.id === id) ?? this.fx.history.find((s) => s.id === id);
  }

  private pending(): PendingItem[] {
    return this.fx.pendingItems;
  }

  /** Dev hook: someone answers `id` at the Mac (or on another device). */
  resolveElsewhere(id: string, byName = 'VS Code'): void {
    const item = this.fx.pendingItems.find((p) => p.id === id);
    if (!item) return;
    this.dropPending(item);
    this.broadcast({
      t: 'pendingResolved',
      id,
      session: item.sessionId,
      outcome: 'withdrawn',
      by: { kind: 'host', name: byName },
      at: Date.now(),
    });
  }

  private dropPending(item: PendingItem): void {
    this.fx.pendingItems = this.fx.pendingItems.filter((p) => p.id !== item.id);
    const s = this.find(item.sessionId);
    if (!s) return;
    const rest = this.fx.pendingItems.filter((p) => p.sessionId === s.id).map(toPreview);
    this.patch(s.id, {
      pending: rest,
      ...(rest.length
        ? {}
        : {
            status: 'working',
            activity: { kind: 'thinking', label: 'Thinking', since: Date.now() },
          }),
    });
  }

  private fleet(): FleetSession[] {
    return Object.entries(this.fx.agents)
      .map(([sessionId, list]) => {
        // Like the server: the session's account while there are several.
        const account = this.multi()
          ? this.accountRef(this.sessionAccounts.get(sessionId) ?? DEFAULT_ACCOUNT)
          : undefined;
        return {
          sessionId,
          name: this.find(sessionId)?.title ?? sessionId,
          agents: list.filter((a) => a.status === 'running'),
          ...(account ? { account } : {}),
        };
      })
      .filter((f) => f.agents.length);
  }

  /** The fleet header, by the server's rules (per account once there are
   * several). */
  private fleetSummary(): FleetSummary {
    const out: FleetSummary = { working: 0, agentsRunning: 0, needYou: 0, costToday: 0 };
    const accounts: NonNullable<FleetSummary['accounts']> = {};
    for (const s of this.fx.sessions) {
      const working = s.live && s.status === 'working';
      const needYou = s.pending.some((p) => p.answerable);
      out.working += Number(working);
      out.needYou += Number(needYou);
      out.agentsRunning += s.agents.running;
      out.costToday = (out.costToday ?? 0) + (s.costToday ?? 0);
      if (s.account) {
        const a = (accounts[s.account.id] ??= { working: 0, needYou: 0, live: 0, costToday: 0 });
        a.working += Number(working);
        a.needYou += Number(needYou);
        a.live += Number(s.live);
        a.costToday = Math.round((a.costToday + (s.costToday ?? 0)) * 100) / 100;
      }
    }
    out.costToday = Math.round((out.costToday ?? 0) * 100) / 100;
    return Object.keys(accounts).length ? { ...out, accounts } : out;
  }

  private detail(s: SessionSummary): SessionDetail {
    const todos = this.fx.transcripts[s.id]?.find((e) => e.kind === 'todos')?.todos ?? [];
    return {
      ...s,
      agentsDetail: this.fx.agents[s.id] ?? [],
      workflows: this.fx.workflows[s.id] ?? [],
      tasks: this.fx.tasks[s.id] ?? [],
      todosDetail: todos,
      queuedMessages: this.fx.queued[s.id] ?? [],
      pendingDetail: this.fx.pendingItems.filter((p) => p.sessionId === s.id),
      filesChanged: this.fx.diff.files.map((f) => ({
        path: f.path,
        added: f.additions,
        removed: f.deletions,
      })),
    };
  }

  private transcript(id: string, agent: string | undefined, q: URLSearchParams) {
    const all = agent
      ? (this.fx.agentTranscripts[agent] ?? [
          ev(`${agent}-1`, Date.now() - 60_000, 'assistant', 'text', {
            agentId: agent,
            text: 'Working on the task I was given…',
          }),
        ])
      : (this.fx.transcripts[id] ?? [
          ev(`${id}-1`, Date.now() - 3_600_000, 'user', 'prompt', { text: 'Take a look at this.' }),
          ev(`${id}-2`, Date.now() - 3_500_000, 'assistant', 'text', { text: 'On it.' }),
        ]);
    if (q.get('after')) {
      const from = Math.min(all.length, Number(q.get('after')) || 0);
      return { events: all.slice(from), tail: String(all.length) };
    }
    const end = q.get('before') ? Number(q.get('before')) : all.length;
    const limit = Number(q.get('limit') ?? 150) || 150;
    const start = Math.max(0, end - limit);
    return {
      events: all.slice(start, end),
      ...(start > 0 ? { before: String(start) } : {}),
      tail: String(all.length),
    };
  }

  private patch(id: string, change: Partial<SessionSummary>): void {
    const idx = this.fx.sessions.findIndex((s) => s.id === id);
    if (idx < 0) return;
    const next = { ...this.fx.sessions[idx], ...change };
    this.fx.sessions[idx] = next;
    this.seq += 1;
    this.broadcast({ t: 'sessions', full: false, upsert: [next], seq: this.seq });
  }

  private append(id: string, events: UiEvent[]): void {
    const list = (this.fx.transcripts[id] ??= []);
    list.push(...events);
    const tail = String(list.length);
    for (const s of this.sockets) {
      if (s.authed && s.subs.has(id)) s.emit({ t: 'transcript', session: id, events, tail });
    }
  }

  private respond(s: SessionSummary, body: RespondBody): Response {
    const item = this.fx.pendingItems.find((p) => p.id === body?.requestId);
    if (!item) {
      return json(
        { error: { code: 'already_decided', message: 'That request was already answered.' } },
        409,
      );
    }
    if (item.dangerous && body.decision === 'allow' && this.role !== 'desktop') {
      return json(
        { error: { code: 'dangerous_blocked', message: 'Dangerous approval blocked.' } },
        403,
      );
    }
    const now = Date.now();
    this.dropPending(item);
    if (body.mode) this.patch(s.id, { permissionMode: body.mode });
    const who = this.role === 'desktop' ? 'the Mac' : "Marcus's iPhone";
    const outcome = body.answers ? 'answered' : body.decision === 'allow' ? 'allowed' : 'denied';
    this.broadcast({
      t: 'pendingResolved',
      id: item.id,
      session: s.id,
      outcome,
      by:
        this.role === 'desktop' ? { kind: 'desktop', name: 'Mac' } : { kind: 'device', name: who },
      at: now,
    });
    const suggestion =
      body.suggestion !== undefined
        ? item.suggestions.find((x) => x.index === body.suggestion)
        : null;
    this.append(s.id, [
      ev(`resp-${now}:0`, now, 'system', 'decision', {
        text:
          outcome === 'answered'
            ? `Answered by ${who}: ${Object.values(body.answers ?? {}).join(' · ')}`
            : outcome === 'allowed'
              ? `Allowed by ${who}${suggestion ? ` · ${suggestion.label}` : ''}`
              : `Denied by ${who}${body.message ? `: ${body.message}` : ''}`,
      }),
    ]);
    return empty();
  }

  private message(s: SessionSummary, body: unknown) {
    const text = String((body as { text?: string })?.text ?? '');
    const now = Date.now();
    if (s.status === 'working' && s.control.message === 'direct') {
      const q = (this.fx.queued[s.id] ??= []);
      q.push({
        id: `q-${now}`,
        text,
        createdAt: now,
        via: 'owned',
        status: 'queued',
        cancellable: true,
      });
      this.patch(s.id, { queued: q.length });
      return { delivery: 'queued', messageId: `q-${now}` };
    }
    this.append(s.id, [
      ev(`msg-${now}`, now, 'user', s.control.message === 'relay' ? 'relayed' : 'prompt', { text }),
    ]);
    this.patch(s.id, {
      status: 'working',
      activity: { kind: 'thinking', label: 'Thinking', since: now },
      lastPrompt: text,
      lastActivityAt: now,
    });
    return {
      delivery: s.control.message === 'relay' ? 'relayed' : 'sent',
      messageId: `msg-${now}`,
    };
  }

  private spawn(body: unknown): { id: string } {
    const b = (body ?? {}) as { cwd?: string; prompt?: string; account?: string };
    const id = `s-new-${Date.now()}`;
    const cwd = b.cwd || '/Users/marcus/dev/project';
    // The account asked for, else the project's, else the default.
    const account =
      b.account && this.account(b.account) ? b.account : accountForCwd(this.fx.accounts, cwd).id;
    this.sessionAccounts.set(id, account);
    const ref = this.multi() ? this.accountRef(account) : undefined;
    const name = cwd.split('/').filter(Boolean).pop() ?? 'project';
    const now = Date.now();
    const s: SessionSummary = {
      id,
      name,
      title: b.prompt?.slice(0, 60) || `New session in ${name}`,
      projectName: name,
      cwd,
      surface: 'mother-claude',
      owned: true,
      live: true,
      status: 'starting',
      activity: { kind: 'starting', label: 'Starting', since: now },
      usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
      messageCount: 0,
      agents: { running: 0, total: 0, workflowsRunning: 0, backgroundRunning: 0 },
      queued: 0,
      pending: [],
      control: ownedCaps(),
      ultracode: 'off',
      ultracodeSource: 'applied',
      thinking: { mode: 'adaptive', source: 'control' },
      startedAt: now,
      lastActivityAt: now,
      ...(ref ? { account: ref } : {}),
    };
    this.fx.sessions.unshift(s);
    this.fx.transcripts[id] = b.prompt
      ? [ev(`${id}-p`, now, 'user', 'prompt', { text: b.prompt })]
      : [];
    this.seq += 1;
    this.broadcast({ t: 'sessions', full: false, upsert: [s], seq: this.seq });
    return { id };
  }

  /** A copy of `s` driven from here: under `account` when given, else
   * under the same account as `s`. */
  private fork(s: SessionSummary, account?: AccountView): string {
    const source = this.sessionAccounts.get(s.id) ?? DEFAULT_ACCOUNT;
    const { id } = this.spawn({ cwd: s.cwd, prompt: '', account: account?.id ?? source });
    this.patch(id, {
      title: `${s.title ?? s.name} (${account ? `on ${account.name}` : 'fork'})`,
      status: 'idle',
      activity: { kind: 'idle', label: 'Idle' },
    });
    return id;
  }

  /** `POST /sessions/{id}/continue`, with the server's account rules:
   * another account always forks (a session Mother Claude runs keeps
   * running); one with its own directory gets the conversation copied there
   * first, once the desktop allowed copies between the two accounts. */
  private continueSession(s: SessionSummary, body: unknown): Response {
    const asked = (body as { account?: unknown }).account;
    const source = this.sessionAccounts.get(s.id) ?? DEFAULT_ACCOUNT;
    if (asked === undefined || asked === source) {
      return json({ id: s.live ? this.fork(s) : s.id });
    }
    const refusal = this.accountRefusal(asked);
    if (refusal) return refusal;
    const target = this.account(String(asked)) as AccountView;
    const from = this.account(source) ?? { id: source, kind: 'directory' };
    if (needsCopy(from, target)) {
      const allowed = this.fx.accounts.copyConsent.some(
        (c) => c.from === source && c.to === target.id,
      );
      if (!allowed) {
        return json(
          {
            error: {
              code: 'copy_consent_required',
              message:
                'That account keeps its own directory: the conversation has to be copied there first. Allow copies between these two accounts (on the desktop), then try again.',
              from: source,
              to: target.id,
            },
          },
          409,
        );
      }
      this.data.copyForContinue(s.id, target.id);
    }
    return json({ id: this.fork(s, target), forked: true }, 201);
  }

  // ────────────────────────────────────────────────────────── Accounts

  private multi(): boolean {
    return this.fx.accounts.accounts.length > 1;
  }

  private account(id: string | undefined): AccountView | undefined {
    return id ? this.fx.accounts.accounts.find((a) => a.id === id) : undefined;
  }

  private accountRef(id: string): AccountRef | undefined {
    const a = this.account(id) ?? this.account(DEFAULT_ACCOUNT);
    return a ? { id: a.id, name: a.name, color: a.color, kind: a.kind } : undefined;
  }

  /** The registry as clients see it: the sign-in link only for the
   * desktop, live counts only in REST answers. */
  private accountsView(desktop: boolean, withLive: boolean): AccountsView {
    const live = new Map<string, number>();
    for (const s of this.fx.sessions) {
      if (!s.live) continue;
      const id = this.sessionAccounts.get(s.id) ?? DEFAULT_ACCOUNT;
      live.set(id, (live.get(id) ?? 0) + 1);
    }
    const view = structuredClone(this.fx.accounts);
    for (const a of view.accounts) {
      const capture = this.captures.get(a.id);
      if (capture) {
        a.capture = { ...capture };
        if (!desktop) delete a.capture.url;
      }
      if (withLive) a.live = live.get(a.id) ?? 0;
    }
    return view;
  }

  /** Every other account's usage (the accounts that still exist). */
  private otherUsage(): Record<string, PlanUsage> {
    const out: Record<string, PlanUsage> = {};
    for (const a of this.fx.accounts.accounts) {
      const u = this.fx.accountUsage[a.id];
      if (!a.isDefault && u) out[a.id] = u;
    }
    return out;
  }

  /** Something about the accounts changed: tell everyone (sessions name
   * their account only while there is more than the Default). */
  private publishAccounts(): void {
    this.syncSessionAccounts(true);
    this.broadcast({ t: 'accounts', accounts: this.accountsView(false, false) });
  }

  private syncSessionAccounts(broadcast: boolean): void {
    const multi = this.multi();
    const changed: SessionSummary[] = [];
    const sync = (list: SessionSummary[], track: boolean) => {
      list.forEach((s, i) => {
        const ref = multi
          ? this.accountRef(this.sessionAccounts.get(s.id) ?? DEFAULT_ACCOUNT)
          : undefined;
        if (JSON.stringify(s.account) === JSON.stringify(ref)) return;
        const next = { ...s };
        if (ref) next.account = ref;
        else delete next.account;
        list[i] = next;
        if (track) changed.push(next);
      });
    };
    sync(this.fx.sessions, true);
    sync(this.fx.history, false);
    if (broadcast && changed.length) {
      this.seq += 1;
      this.broadcast({ t: 'sessions', full: false, upsert: changed, seq: this.seq });
    }
  }

  /** A launch or continue that names an account it can't run under. */
  private accountRefusal(asked: unknown): Response | null {
    if (asked === undefined || asked === null || asked === '') return null;
    const a = typeof asked === 'string' ? this.account(asked) : undefined;
    if (!a) {
      return json(
        { error: { code: 'unknown_account', message: `no account ${JSON.stringify(asked)}` } },
        422,
      );
    }
    if (a.kind === 'token' && !a.tokenStored) {
      return json(
        {
          error: {
            code: 'account_token_missing',
            message: "the account's token is not stored on this machine; enter it again",
          },
        },
        409,
      );
    }
    return null;
  }

  private updateAccount(id: string, change: Partial<AccountView>): void {
    this.fx.accounts.accounts = this.fx.accounts.accounts.map((a) => {
      if (a.id !== id) return a;
      const next = { ...a, ...change };
      for (const k of Object.keys(next) as (keyof AccountView)[]) {
        if (next[k] === undefined) delete next[k];
      }
      return next;
    });
  }

  /** The account as signed in after a sign-in or a token landed. */
  private signedIn(a: AccountView): Partial<AccountView> {
    const slug = a.name
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, '.')
      .replace(/^\.|\.$/g, '');
    return {
      signedIn: true,
      attention: undefined,
      authError: undefined,
      email: a.email ?? `${slug || 'account'}@example.com`,
      subscription: a.subscription ?? (a.kind === 'token' ? 'pro' : 'max'),
      authMethod: a.kind === 'token' ? 'oauth_token' : 'claude.ai',
    };
  }

  private accountJson(id: string, status = 200): Response {
    const a = this.accountsView(this.role === 'desktop', true).accounts.find((x) => x.id === id);
    return json(a ?? null, status);
  }

  private addAccount(body: unknown): Response {
    const b = (body ?? {}) as Record<string, unknown>;
    const name = typeof b['name'] === 'string' ? b['name'].trim() : '';
    const problem = accountNameProblem(name, this.fx.accounts.accounts);
    if (problem) {
      const taken = problem.startsWith('Another');
      return json(
        { error: { code: taken ? 'account_name_taken' : 'invalid_account', message: problem } },
        taken ? 409 : 422,
      );
    }
    const used = new Set(this.fx.accounts.accounts.map((a) => a.color));
    const asked = b['color'];
    const color = isAccountColor(asked)
      ? asked.toLowerCase()
      : (ACCOUNT_COLORS.find((c) => !used.has(c)) ?? ACCOUNT_COLORS[1]);
    const id = `acc_${randomHex(12)}`;
    const base = {
      id,
      name,
      isDefault: false,
      color,
      createdAt: Date.now(),
      checking: false,
      signingIn: false,
    };
    let account: AccountView;
    if (b['kind'] === 'directory') {
      let dir = typeof b['dir'] === 'string' ? b['dir'].trim() : '';
      if (dir.startsWith('~/')) dir = `/Users/marcus/${dir.slice(2)}`;
      dir = dir.replace(/\/+$/, '');
      if (!dir.startsWith('/')) {
        return json(
          { error: { code: 'invalid_account', message: 'A directory account needs `dir`.' } },
          422,
        );
      }
      if (this.fx.accounts.accounts.some((a) => a.dir === dir)) {
        return json(
          {
            error: {
              code: 'account_dir_unavailable',
              message: `${dir} is already another account's directory`,
            },
          },
          422,
        );
      }
      // A fresh directory is signed out.
      account = {
        ...base,
        kind: 'directory',
        dir,
        signedIn: false,
        attention: 'signedOut',
        tokenStored: false,
      };
    } else if (b['kind'] === 'token') {
      const token = typeof b['token'] === 'string' ? b['token'].trim() : '';
      if (token && !TOKEN_RE.test(token)) {
        return json(
          { error: { code: 'token_rejected', message: 'Claude Code does not accept this token.' } },
          422,
        );
      }
      account = token
        ? { ...base, kind: 'token', signedIn: false, tokenStored: true }
        : {
            ...base,
            kind: 'token',
            signedIn: false,
            attention: 'tokenMissing',
            tokenStored: false,
          };
      if (token) account = { ...account, ...this.signedIn(account) };
    } else {
      return json(
        { error: { code: 'invalid_account', message: 'kind is "directory" or "token".' } },
        422,
      );
    }
    for (const k of Object.keys(account) as (keyof AccountView)[]) {
      if (account[k] === undefined) delete account[k];
    }
    this.fx.accounts.accounts.push(account);
    this.fx.accountUsage[id] = {
      available: account.kind === 'directory',
      windows: [],
      refreshing: false,
      intervalSecs: this.fx.planUsage.intervalSecs,
    };
    this.publishAccounts();
    this.broadcast({ t: 'accountUsage', accountId: id, usage: this.fx.accountUsage[id] });
    return this.accountJson(id, 201);
  }

  private async accountRoute(
    id: string,
    sub: string,
    method: string,
    body: unknown,
  ): Promise<Response> {
    const desktop = this.role === 'desktop';
    const deskOnly = () =>
      json({ error: { code: 'desktop_only', message: 'Only the desktop app can do this.' } }, 403);
    const a = this.account(id);
    if (!a) {
      return json(
        { error: { code: 'account_not_found', message: `No account ${JSON.stringify(id)}.` } },
        404,
      );
    }
    const b = (body ?? {}) as Record<string, unknown>;

    if (!sub && method === 'PATCH') {
      if (!desktop) return deskOnly();
      const change: Partial<AccountView> = {};
      if (typeof b['name'] === 'string') {
        const problem = accountNameProblem(b['name'], this.fx.accounts.accounts, a.id);
        if (problem) {
          const taken = problem.startsWith('Another');
          return json(
            { error: { code: taken ? 'account_name_taken' : 'invalid_account', message: problem } },
            taken ? 409 : 422,
          );
        }
        change.name = b['name'].trim();
      }
      if (b['color'] !== undefined) {
        if (!isAccountColor(b['color'])) {
          return json({ error: { code: 'invalid_account', message: 'a colour is #rrggbb' } }, 422);
        }
        change.color = b['color'].toLowerCase();
      }
      this.updateAccount(a.id, change);
      this.publishAccounts();
      return this.accountJson(a.id);
    }

    if (!sub && method === 'DELETE') {
      if (!desktop) return deskOnly();
      if (a.isDefault) {
        return json(
          {
            error: {
              code: 'default_account_fixed',
              message: 'the Default account cannot be removed',
            },
          },
          422,
        );
      }
      const running = this.fx.sessions.filter(
        (s) => s.owned && s.live && this.sessionAccounts.get(s.id) === a.id,
      ).length;
      if (running) {
        return json(
          {
            error: {
              code: 'account_in_use',
              message: `${running} session(s) Mother Claude started still run under this account.`,
            },
          },
          409,
        );
      }
      const reg = this.fx.accounts;
      reg.accounts = reg.accounts.filter((x) => x.id !== a.id);
      if (reg.defaults.newSession === a.id) reg.defaults.newSession = DEFAULT_ACCOUNT;
      if (reg.defaults.bridge === a.id) reg.defaults.bridge = DEFAULT_ACCOUNT;
      reg.defaults.byProject = Object.fromEntries(
        Object.entries(reg.defaults.byProject).filter(([, acc]) => acc !== a.id),
      );
      reg.copyConsent = reg.copyConsent.filter((c) => c.from !== a.id && c.to !== a.id);
      for (const [sid, acc] of [...this.sessionAccounts]) {
        if (acc === a.id) this.sessionAccounts.delete(sid);
      }
      delete this.fx.accountUsage[a.id];
      this.captures.delete(a.id);
      this.publishAccounts();
      return empty();
    }

    if (sub === 'refresh' && method === 'POST') {
      this.updateAccount(a.id, { checking: true });
      this.publishAccounts();
      setTimeout(() => {
        this.updateAccount(a.id, { checking: false });
        this.publishAccounts();
      }, 700);
      return this.accountJson(a.id);
    }

    if (sub === 'login' && method === 'POST') {
      if (!desktop) return deskOnly();
      if (a.kind === 'token') {
        return json(
          {
            error: {
              code: 'login_failed',
              message: 'A token account signs in with its token: get one, or paste one.',
            },
          },
          422,
        );
      }
      this.updateAccount(a.id, { signingIn: true });
      this.publishAccounts();
      setTimeout(() => {
        const cur = this.account(a.id);
        if (!cur) return;
        this.updateAccount(a.id, { signingIn: false, ...this.signedIn(cur) });
        this.publishAccounts();
      }, 3_500);
      return this.accountJson(a.id, 202);
    }

    if (sub === 'token') {
      if (!desktop) return deskOnly();
      if (method === 'GET') {
        return json({
          available: true,
          ...(this.captures.get(a.id) ? { capture: this.captures.get(a.id) } : {}),
          tokenStored: a.tokenStored,
        });
      }
      if (method === 'DELETE') {
        const cap = this.captures.get(a.id);
        if (cap?.state === 'running') this.finishCapture(a.id, 'cancelled');
        this.publishAccounts();
        return json({ capture: this.captures.get(a.id) ?? null });
      }
      if (method === 'POST') {
        if (a.kind !== 'token') {
          return json(
            { error: { code: 'not_a_token_account', message: 'this is not a token account' } },
            422,
          );
        }
        const token = typeof b['token'] === 'string' ? b['token'].trim() : '';
        const code = typeof b['code'] === 'string' ? b['code'].trim() : '';
        if (token && code) {
          return json(
            {
              error: { code: 'invalid_body', message: 'Send either `token` or `code`, not both.' },
            },
            422,
          );
        }
        if (token) {
          await delay(500);
          if (!TOKEN_RE.test(token)) {
            return json(
              {
                error: {
                  code: 'token_rejected',
                  message: 'Claude Code does not accept this token.',
                },
              },
              422,
            );
          }
          this.updateAccount(a.id, { tokenStored: true, ...this.signedIn(a) });
          this.publishAccounts();
          return this.accountJson(a.id);
        }
        if (code) {
          const cap = this.captures.get(a.id);
          if (cap?.state !== 'running') {
            return json(
              {
                error: {
                  code: 'capture_not_running',
                  message: 'No sign-in is waiting for a code.',
                },
              },
              409,
            );
          }
          this.captures.set(a.id, { ...cap, asksForCode: false });
          setTimeout(() => this.finishCapture(a.id, 'captured'), 1_200);
          return json({ capture: this.captures.get(a.id) });
        }
        const capture: CaptureView = {
          state: 'running',
          url: 'https://claude.ai/oauth/authorize?code=true&client_id=mock-client&response_type=code&scope=user%3Ainference',
          asksForCode: CAPTURE_MODE === 'code',
          startedAt: Date.now(),
        };
        this.captures.set(a.id, capture);
        this.publishAccounts();
        if (CAPTURE_MODE === 'fail') {
          setTimeout(
            () => this.finishCapture(a.id, 'failed', 'setup-token exited before it showed a token'),
            3_000,
          );
        } else if (CAPTURE_MODE !== 'code') {
          setTimeout(() => this.finishCapture(a.id, 'captured'), 4_000);
        }
        return json({ capture }, 202);
      }
    }

    if ((sub === 'hooks/install' || sub === 'hooks/uninstall') && method === 'POST') {
      if (!desktop) return deskOnly();
      if (a.isDefault) {
        return json(
          {
            error: {
              code: 'use_default_install',
              message:
                "The Default account's plugin is managed with foreign control's own install.",
            },
          },
          422,
        );
      }
      if (a.kind !== 'directory') {
        return json(
          {
            error: {
              code: 'not_a_directory_account',
              message: "A token account shares the Default account's directory and its plugin.",
            },
          },
          422,
        );
      }
      await delay(700);
      this.updateAccount(a.id, {
        hooksPlugin: sub === 'hooks/install' ? 'installed' : 'missing',
      });
      this.publishAccounts();
      return this.accountJson(a.id);
    }

    return json({ error: { code: 'not_found', message: `No mock for ${method} ${sub}` } }, 404);
  }

  private finishCapture(id: string, state: CaptureView['state'], error?: string): void {
    const cap = this.captures.get(id);
    if (!cap || cap.state !== 'running') return;
    this.captures.set(id, {
      ...cap,
      state,
      asksForCode: false,
      finishedAt: Date.now(),
      ...(error ? { error } : {}),
    });
    const a = this.account(id);
    if (state === 'captured' && a) {
      this.updateAccount(id, { tokenStored: true, ...this.signedIn(a) });
    }
    this.publishAccounts();
  }

  private putAccountDefaults(body: unknown): Response {
    const b = (body ?? {}) as Partial<AccountDefaults>;
    const ids = [b.newSession, b.bridge, ...Object.values(b.byProject ?? {})];
    const unknown = ids.find((id) => !id || !this.account(id));
    if (unknown !== undefined) {
      return json(
        { error: { code: 'account_not_found', message: `no account ${JSON.stringify(unknown)}` } },
        404,
      );
    }
    const byProject: Record<string, string> = {};
    for (const [dir, id] of Object.entries(b.byProject ?? {})) {
      if (!dir.startsWith('/') || dir.split('/').some((p) => p === '.' || p === '..')) {
        return json(
          {
            error: {
              code: 'invalid_account',
              message: `${JSON.stringify(dir)} is not an absolute project directory`,
            },
          },
          422,
        );
      }
      byProject[dir.replace(/\/+$/, '') || '/'] = id;
    }
    this.fx.accounts.defaults = {
      newSession: b.newSession as string,
      bridge: b.bridge as string,
      byProject,
    };
    this.publishAccounts();
    return json(this.fx.accounts.defaults);
  }

  private putCopyConsent(body: unknown): Response {
    const b = (body ?? {}) as { from?: string; to?: string; granted?: boolean };
    if (!this.account(b.from) || !this.account(b.to)) {
      return json({ error: { code: 'account_not_found', message: 'no such account' } }, 404);
    }
    const reg = this.fx.accounts;
    reg.copyConsent = reg.copyConsent.filter((c) => !(c.from === b.from && c.to === b.to));
    if (b.granted) reg.copyConsent.push({ from: b.from as string, to: b.to as string });
    this.publishAccounts();
    return json(this.accountsView(true, true));
  }

  /** `POST /usage/refresh {account}` for an account other than the Default. */
  private refreshAccountUsage(id: string): Response {
    const u = this.fx.accountUsage[id];
    if (!this.account(id) || !u) {
      return json(
        { error: { code: 'account_not_found', message: `No account ${JSON.stringify(id)}.` } },
        404,
      );
    }
    const since = Date.now() - (this.accountReads.get(id) ?? 0);
    if (since < 60_000) {
      const wait = Math.ceil((60_000 - since) / 1000);
      return json(
        { error: { code: 'rate_limited', message: `Try again in ${wait}s.`, retryAfter: wait } },
        429,
        { 'retry-after': String(wait) },
      );
    }
    this.accountReads.set(id, Date.now());
    const set = (change: Partial<PlanUsage>) => {
      const next = { ...this.fx.accountUsage[id], ...change };
      this.fx.accountUsage[id] = next;
      this.broadcast({ t: 'accountUsage', accountId: id, usage: next });
    };
    set({ refreshing: true });
    setTimeout(() => {
      if (!this.fx.accountUsage[id]) return;
      // A token is inference-only: nothing new to read.
      const token = this.account(id)?.kind === 'token';
      set(
        token
          ? { refreshing: false }
          : { refreshing: false, fetchedAt: Date.now(), source: 'live' },
      );
    }, 1_200);
    return json(this.fx.accountUsage[id], 202);
  }

  private stopAgent(sessionId: string, taskId: string): void {
    const list = this.fx.agents[sessionId] ?? [];
    this.fx.agents[sessionId] = list.map((a: AgentInfo) =>
      a.taskId === taskId || a.id === taskId ? { ...a, status: 'stopped', endedAt: Date.now() } : a,
    );
    for (const s of this.sockets) {
      if (s.authed && s.fleet) {
        s.emit({ t: 'fleet', sessions: this.fleet(), summary: this.fleetSummary() });
      }
    }
  }
}

/** An in-page stand-in for the server WebSocket (auth first frame, v2 frames). */
class FakeSocket implements WebSocketLike {
  readyState = 0;
  onopen: ((ev: Event) => void) | null = null;
  onmessage: ((ev: MessageEvent) => void) | null = null;
  onclose: ((ev: CloseEvent) => void) | null = null;
  onerror: ((ev: Event) => void) | null = null;
  authed = false;
  fleet = false;
  inbox = false;
  readonly subs = new Set<string>();
  readonly agentSubs = new Set<string>();

  constructor(
    private readonly backend: FixtureBackend,
    fail = false,
  ) {
    setTimeout(() => {
      if (this.readyState !== 0) return;
      if (fail) {
        this.finish(1006, 'unreachable');
        return;
      }
      this.readyState = 1;
      this.onopen?.(new Event('open'));
    }, 60);
  }

  send(data: string): void {
    let frame: { t?: string; token?: string; session?: string; agent?: string };
    try {
      frame = JSON.parse(data);
    } catch {
      return;
    }
    switch (frame.t) {
      case 'auth':
        this.authed = frame.token === TOKEN;
        this.backend.onAuthed(this, frame.token ?? '');
        break;
      case 'sub':
        if (frame.session && frame.agent) {
          this.agentSubs.add(`${frame.session}/${frame.agent}`);
        } else if (frame.session) {
          this.subs.add(frame.session);
          this.backend.onSub(this, frame.session);
        }
        break;
      case 'unsub':
        if (frame.session && frame.agent) this.agentSubs.delete(`${frame.session}/${frame.agent}`);
        else if (frame.session) this.subs.delete(frame.session);
        break;
      case 'subInbox':
        this.inbox = true;
        this.backend.onSubInbox(this);
        break;
      case 'unsubInbox':
        this.inbox = false;
        break;
      case 'subFleet':
        this.fleet = true;
        this.backend.onSubFleet(this);
        break;
      case 'unsubFleet':
        this.fleet = false;
        break;
      case 'ping':
        this.emit({ t: 'pong' });
        break;
      default:
        break;
    }
  }

  emit(frame: ServerFrame): void {
    if (this.readyState !== 1) return;
    const data = JSON.stringify(frame);
    setTimeout(() => this.onmessage?.(new MessageEvent('message', { data })), 0);
  }

  /** Client-initiated close. */
  close(code = 1000, reason = ''): void {
    this.finish(code, reason);
  }

  /** Server-initiated close (e.g. 4401). */
  serverClose(code: number, reason: string): void {
    setTimeout(() => this.finish(code, reason), 0);
  }

  private finish(code: number, reason: string): void {
    if (this.readyState === 3) return;
    this.readyState = 3;
    this.backend.forget(this);
    this.onclose?.(new CloseEvent('close', { code, reason, wasClean: true }));
  }
}

// ───────────────────────────────────────────────────────────── Helpers

function delay(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}

function json(body: unknown, status = 200, extra: Record<string, string> = {}): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json', ...extra },
  });
}

function empty(): Response {
  return new Response(null, { status: 204 });
}

function parseBody(body: BodyInit | null | undefined): unknown {
  if (typeof body !== 'string') return {};
  try {
    return JSON.parse(body);
  } catch {
    return {};
  }
}

function svgImage(): Response {
  const svg =
    '<svg xmlns="http://www.w3.org/2000/svg" width="320" height="200"><rect width="320" height="200" fill="#7c5cc9"/><text x="160" y="108" font-size="20" text-anchor="middle" fill="#fff">screenshot.png</text></svg>';
  return new Response(svg, { status: 200, headers: { 'content-type': 'image/svg+xml' } });
}

function randomHex(n: number): string {
  let out = '';
  for (let i = 0; i < n; i++) out += Math.floor(Math.random() * 16).toString(16);
  return out;
}

const CROCKFORD = '0123456789ABCDEFGHJKMNPQRSTVWXYZ';

function randomCode(): string {
  let out = '';
  for (let i = 0; i < 12; i++) out += CROCKFORD[Math.floor(Math.random() * CROCKFORD.length)];
  return out;
}

/** A QR-looking (not scannable) SVG, deterministic per code. */
function fakeQr(code: string): string {
  const n = 29;
  let seed = [...code].reduce((a, c) => (a * 31 + c.charCodeAt(0)) >>> 0, 7);
  const rand = () => {
    seed = (seed * 1103515245 + 12345) >>> 0;
    return seed / 2 ** 32;
  };
  /** Finder-pattern module (true/false) in the three corners, else null. */
  const finder = (x: number, y: number): boolean | null => {
    const inCorner = (x < 7 && y < 7) || (x >= n - 7 && y < 7) || (x < 7 && y >= n - 7);
    if (!inCorner) return null;
    const cx = x < 7 ? x : x - (n - 7);
    const cy = y < 7 ? y : y - (n - 7);
    const ring = Math.max(Math.abs(cx - 3), Math.abs(cy - 3));
    return ring === 3 || ring <= 1;
  };
  const rects: string[] = [];
  for (let y = 0; y < n; y++) {
    for (let x = 0; x < n; x++) {
      const f = finder(x, y);
      const on = f === null ? rand() > 0.52 : f;
      if (on) rects.push(`<rect x="${x}" y="${y}" width="1" height="1"/>`);
    }
  }
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${n} ${n}" shape-rendering="crispEdges"><g fill="#000">${rects.join('')}</g></svg>`;
}
