// Dev-only fixture backend for `?mock=1`: answers every /api endpoint of the
// contract and runs a fake WebSocket v2 stream, so every page can be used and
// screenshot without the Rust server. Never bundled in production builds.

import {
  AgentInfo,
  FleetSession,
  PairCode,
  PendingItem,
  RespondBody,
  ServerFrame,
  SessionDetail,
  SessionSummary,
  UiEvent,
} from '../models';
import { Fixtures, buildFixtures, ev, ownedCaps, toPreview } from './fixtures';
import { MockBackend, MockRole, WebSocketLike, mockRole } from './mock-api';
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

  constructor(readonly role: MockRole) {
    this.fx = buildFixtures(Date.now());
    this.token = role === 'unpaired' ? '' : TOKEN;
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
    if (path === '/sessions' && method === 'POST') return json(this.spawn(body), 201);
    if (path === '/agents') return json({ sessions: this.fleet() });
    if (path === '/inbox') return json({ items: this.pending() });
    if (path === '/projects') return json({ projects: this.fx.projects });
    if (path === '/defaults') return json(this.fx.defaults);
    if (path === '/services') {
      return json({
        mcpServers: { github: {}, linear: {}, playwright: {} },
        daemon: { reachable: true, raw: 'claude daemon 2.1.282 · pid 4411 · up 3d 4h' },
        backgroundJobs: [{ id: 'k7m2', cwd: '/Users/marcus/dev/monorepo', state: 'running' }],
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
      if (method === 'POST' && sub === 'continue' && s)
        return json({ id: s.live ? this.fork(s) : s.id });
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
        this.patch(s.id, { effort: String((body as { effort?: string }).effort ?? '') });
        return empty();
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
      if (method === 'POST' && ['stop', 'respawn', 'rm', 'keys'].includes(sub)) return empty();
      return json({ error: { code: 'not_found', message: `No mock for ${method} ${path}` } }, 404);
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
      this.fx.bridge = { ...this.fx.bridge, running: true, startedAt: Date.now() };
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
    socket.emit({ t: 'fleet', sessions: this.fleet() });
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
      if (s.fleet) s.emit({ t: 'fleet', sessions: this.fleet() });
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

    if (this.tick % 10 === 0) this.broadcast({ t: 'ping' });
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
      .map(([sessionId, list]) => ({
        sessionId,
        name: this.find(sessionId)?.title ?? sessionId,
        agents: list.filter((a) => a.status === 'running'),
      }))
      .filter((f) => f.agents.length);
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
    const b = (body ?? {}) as { cwd?: string; prompt?: string };
    const id = `s-new-${Date.now()}`;
    const cwd = b.cwd || '/Users/marcus/dev/project';
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
      startedAt: now,
      lastActivityAt: now,
    };
    this.fx.sessions.unshift(s);
    this.fx.transcripts[id] = b.prompt
      ? [ev(`${id}-p`, now, 'user', 'prompt', { text: b.prompt })]
      : [];
    this.seq += 1;
    this.broadcast({ t: 'sessions', full: false, upsert: [s], seq: this.seq });
    return { id };
  }

  private fork(s: SessionSummary): string {
    const { id } = this.spawn({ cwd: s.cwd, prompt: '' });
    this.patch(id, {
      title: `${s.title ?? s.name} (fork)`,
      status: 'idle',
      activity: { kind: 'idle', label: 'Idle' },
    });
    return id;
  }

  private stopAgent(sessionId: string, taskId: string): void {
    const list = this.fx.agents[sessionId] ?? [];
    this.fx.agents[sessionId] = list.map((a: AgentInfo) =>
      a.taskId === taskId || a.id === taskId ? { ...a, status: 'stopped', endedAt: Date.now() } : a,
    );
    for (const s of this.sockets)
      if (s.authed && s.fleet) s.emit({ t: 'fleet', sessions: this.fleet() });
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
