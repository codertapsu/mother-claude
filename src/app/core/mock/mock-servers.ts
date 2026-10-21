// Dev-only: API servers besides the main one (docs/ACCOUNTS.md §13.5) for
// `?mock=1` — `GET /api/bridge`'s `servers`, the `/api/bridge/servers*`
// routes with the server's refusals, the tunnel that points at one
// (`GET /api/remote`) and the account removal rule. Adding a server works in
// any mock; `?mockservers=1` starts with the main server running, "Work"
// running (account Work, only this account, port 5613, one conversation, a
// tunnel pointing at it) and "Personal" stopped (the token account, only it),
// and turns the demo accounts on. Never bundled in production builds.

import { DEFAULT_ACCOUNT } from '../accounts';
import { MAIN_SERVER, ServerDefaults, normalizeDefaults } from '../api-servers';
import {
  AccountRef,
  AccountView,
  BridgeStatus,
  BridgeThread,
  DetectedTunnel,
  RemoteInfo,
} from '../models';
import { urlParam } from '../platform';
import { MOCK_ACCOUNT_PRO, MOCK_ACCOUNT_WORK } from './fixtures';

/** `?mockservers=1`: start with the demo servers (and the demo accounts). */
export const MOCK_SERVERS = urlParam('mockservers') === '1';

const M = 60_000;
const H = 60 * M;
const FIRST_PORT = 5613;
/** The dashboard's and the tunnel listener's ports: no server may take them. */
const RESERVED_PORTS = [6725, 6726];
const NAME_MAX = 40;

/** What the servers mock needs from the fixture backend. */
export interface ServersHost {
  readonly desktop: boolean;
  /** The main server's status (`GET /api/bridge` before servers). */
  main(): BridgeStatus;
  setMain(status: BridgeStatus): void;
  accounts(): AccountView[];
  remote(): RemoteInfo;
  /** An account's chip, while several accounts exist. */
  accountRef(id: string): AccountRef | undefined;
}

interface Server {
  id: string;
  name: string;
  port: number;
  defaults: ServerDefaults;
  running: boolean;
  startedAt?: number;
  /** Started open to tunnels (never saved). */
  open: boolean;
  /** Its running conversations (their account is added when shown). */
  threads: Omit<BridgeThread, 'account'>[];
  activeThread?: string;
  /** A detected tunnel points at its port. */
  tunnelUrl?: string;
}

type Body = Record<string, unknown>;

export class MockServers {
  private readonly servers = new Map<string, Server>();

  constructor(private readonly host: ServersHost) {
    if (MOCK_SERVERS) this.seed();
  }

  /** The answer to a request this mock takes, or null to let the fixture
   * backend answer it. */
  route(path: string, method: string, body: unknown): Response | null {
    const b = (body && typeof body === 'object' ? body : {}) as Body;
    if (path === '/bridge' && method === 'GET') {
      return this.servers.size ? json(this.status()) : null;
    }
    if (path === '/remote' && method === 'GET') {
      return this.servers.size && this.host.desktop ? json(this.remote()) : null;
    }
    const account = /^\/accounts\/([^/]+)$/.exec(path);
    if (account && method === 'DELETE') return this.accountRemoval(decodeURIComponent(account[1]));
    if (path === '/bridge/servers' && method === 'POST') return this.add(b);
    const m = /^\/bridge\/servers\/([^/]+)(?:\/(start|stop|active-thread))?$/.exec(path);
    if (!m) return null;
    const id = decodeURIComponent(m[1]);
    // The main server keeps its own routes (`/bridge/start|stop|active-thread`).
    if (id === MAIN_SERVER) {
      return m[2]
        ? null
        : fail(409, 'main_server_fixed', 'The main server cannot be changed or removed.');
    }
    const s = this.servers.get(id);
    if (!s) return fail(404, 'server_not_found', `No API server ${JSON.stringify(id)}.`);
    if (!m[2] && method === 'PATCH') return this.patch(s, b);
    if (!m[2] && method === 'DELETE') {
      this.servers.delete(s.id);
      return new Response(null, { status: 204 });
    }
    if (method !== 'POST') return null;
    if (m[2] === 'start') return this.start(s, b);
    if (m[2] === 'stop') {
      // Stopping closes the port; its conversations stay.
      Object.assign(s, { running: false, startedAt: undefined, open: false });
      return json(this.view(s));
    }
    const thread = b['threadId'];
    s.activeThread = typeof thread === 'string' && thread ? thread : undefined;
    return json(this.view(s));
  }

  // ───────────────────────────────────────────────────────────── Views

  private status(): BridgeStatus & { servers: unknown[] } {
    const main = this.host.main();
    return { ...main, servers: [this.mainEntry(main), ...this.list().map((s) => this.view(s))] };
  }

  private mainEntry(main: BridgeStatus) {
    return {
      id: MAIN_SERVER,
      name: 'Main',
      running: main.running,
      port: main.port,
      url: main.url ?? null,
      prefix: '',
      startedAt: main.startedAt ?? null,
      requireToken: main.requireToken,
      tokenPolicy: main.tokenPolicy,
      exposedByTunnel: main.exposedByTunnel,
      tunnelUrl: main.tunnelUrl ?? null,
      defaults: main.defaults,
      activeThread: main.activeThread ?? null,
      threads: main.threads,
      docsUrl: main.docsUrl ?? null,
      consoleUrl: main.consoleUrl ?? null,
    };
  }

  private view(s: Server) {
    const url = `http://127.0.0.1:${s.port}`;
    const exposed = !!s.tunnelUrl;
    const missing =
      !!s.defaults.account && !this.host.accounts().some((a) => a.id === s.defaults.account);
    const ref = this.host.accountRef(s.defaults.account || DEFAULT_ACCOUNT);
    return {
      id: s.id,
      name: s.name,
      running: s.running,
      port: s.port,
      url,
      prefix: `/servers/${s.id}`,
      startedAt: s.running ? (s.startedAt ?? null) : null,
      requireToken: false,
      tokenPolicy: s.running && s.open ? 'never' : 'local',
      exposedByTunnel: exposed,
      tunnelUrl: s.tunnelUrl ?? null,
      defaults: s.defaults,
      activeThread: s.activeThread ?? null,
      threads: s.threads.map((t) => ({ ...t, ...(ref && !missing ? { account: ref } : {}) })),
      docsUrl: `${url}/docs/`,
      consoleUrl: `${url}/example/`,
      ...(missing ? { accountMissing: true } : {}),
    };
  }

  /** `GET /api/remote` with the tunnel to a server and the servers' list. */
  private remote(): RemoteInfo {
    const r = this.host.remote();
    const main = this.host.main();
    const tunnels: DetectedTunnel[] = [];
    const warnings: string[] = [];
    for (const s of this.list()) {
      if (!s.tunnelUrl) continue;
      tunnels.push({
        provider: 'ngrok',
        url: s.tunnelUrl,
        upstream: `http://127.0.0.1:${s.port}`,
        upstreamPort: s.port,
        inspect: false,
        kind: 'http',
        target: 'bridge',
        server: s.id,
      } as DetectedTunnel);
      warnings.push(
        `A tunnel (${s.tunnelUrl}) points at the API server ${s.name} (port ${s.port}). Anyone who finds that URL could try to drive Claude. Tokenless access on port ${s.port} is off while it runs — stop that tunnel.`,
      );
    }
    const servers = [
      {
        id: MAIN_SERVER,
        name: 'Main',
        port: main.port,
        running: main.running,
        exposed: main.exposedByTunnel,
      },
      ...this.list().map((s) => ({
        id: s.id,
        name: s.name,
        port: s.port,
        running: s.running,
        exposed: !!s.tunnelUrl,
      })),
    ];
    return {
      ...r,
      internet: {
        ...r.internet,
        detected: [...r.internet.detected, ...tunnels],
        warnings: [...r.internet.warnings, ...warnings],
      },
      bridge: { ...r.bridge, servers } as RemoteInfo['bridge'],
    };
  }

  private list(): Server[] {
    return [...this.servers.values()];
  }

  // ─────────────────────────────────────────────────────────── Changes

  private add(b: Body): Response {
    const name = typeof b['name'] === 'string' ? b['name'].trim() : '';
    const refusal = this.nameRefusal(name);
    if (refusal) return refusal;
    let port: number;
    if (b['port'] === undefined || b['port'] === null) {
      port = this.firstFreePort();
    } else {
      const bad = this.portRefusal(b['port']);
      if (bad) return bad;
      port = b['port'] as number;
    }
    const defaults = normalizeDefaults(b['defaults']);
    const invalid = this.defaultsRefusal(defaults);
    if (invalid) return invalid;
    const s: Server = {
      id: this.newId(name),
      name,
      port,
      defaults,
      running: false,
      open: false,
      threads: [],
    };
    this.servers.set(s.id, s);
    return json(this.view(s), 201);
  }

  private patch(s: Server, b: Body): Response {
    if (b['name'] !== undefined) {
      const name = typeof b['name'] === 'string' ? b['name'].trim() : '';
      const refusal = this.nameRefusal(name, s.id);
      if (refusal) return refusal;
      s.name = name;
    }
    if (b['port'] !== undefined) {
      const bad = this.portRefusal(b['port'], s.id);
      if (bad) return bad;
      s.port = b['port'] as number;
    }
    if (b['defaults'] !== undefined) {
      const defaults = normalizeDefaults(b['defaults']);
      const invalid = this.defaultsRefusal(defaults);
      if (invalid) return invalid;
      s.defaults = defaults;
    }
    // A running server restarts with them.
    if (s.running) s.startedAt = Date.now();
    return json(this.view(s));
  }

  private start(s: Server, b: Body): Response {
    if (b['defaults'] !== undefined) {
      const defaults = normalizeDefaults(b['defaults']);
      const invalid = this.defaultsRefusal(defaults);
      if (invalid) return invalid;
      if (defaults.permissionMode === 'bypassPermissions' && !this.host.desktop) {
        return fail(
          403,
          'desktop_only',
          'Starting an API server with a permission mode that skips approvals is only possible from the Mother Claude desktop app.',
        );
      }
      // The start's defaults are the server's from now on.
      s.defaults = defaults;
    }
    if (s.defaults.account && !this.host.accounts().some((a) => a.id === s.defaults.account)) {
      return fail(
        409,
        'account_not_found',
        `The account of ${s.name} was removed — choose another one first.`,
        { field: 'account' },
      );
    }
    const open = b['openToTunnels'] === true;
    if (open && !this.host.desktop) {
      return fail(
        403,
        'desktop_only',
        'Opening an API port to callers without a token, through tunnels too, is only possible from the Mother Claude desktop app.',
      );
    }
    s.running = true;
    s.startedAt = Date.now();
    s.open = open;
    return json(this.view(s));
  }

  /** Like the server: not an account a running server uses. */
  private accountRemoval(id: string): Response | null {
    if (!this.host.desktop) return null;
    const user = this.list().find((s) => s.running && s.defaults.account === id);
    if (!user) return null;
    return fail(
      409,
      'account_used_by_bridge',
      `The API server ${user.name} runs with this account. Stop it, or start it with another account, first.`,
      { server: user.id, serverName: user.name },
    );
  }

  // ──────────────────────────────────────────────────────────── Checks

  private nameRefusal(name: string, self?: string): Response | null {
    const chars = [...name];
    if (!name || chars.length > NAME_MAX || chars.some((c) => (c.codePointAt(0) ?? 0) < 0x20)) {
      return fail(422, 'invalid_field', 'A name of 1 to 40 characters, please.', {
        field: 'name',
      });
    }
    const lower = name.toLowerCase();
    if (
      lower === 'main' ||
      this.list().some((s) => s.id !== self && s.name.toLowerCase() === lower)
    ) {
      return fail(409, 'server_name_taken', `An API server is already called ${name}.`, {
        field: 'name',
      });
    }
    return null;
  }

  private portRefusal(raw: unknown, self?: string): Response | null {
    const port = typeof raw === 'number' && Number.isInteger(raw) ? raw : NaN;
    if (!(port >= 1024 && port <= 65535)) {
      return fail(422, 'invalid_field', 'A port from 1024 to 65535.', { field: 'port' });
    }
    const holder = this.list().find((s) => s.id !== self && s.port === port);
    if (holder || port === this.host.main().port || RESERVED_PORTS.includes(port)) {
      return fail(409, 'port_unavailable', `Port ${port} is taken.`, {
        field: 'port',
        port,
        ...(holder ? { server: holder.id } : {}),
      });
    }
    return null;
  }

  private defaultsRefusal(d: ServerDefaults): Response | null {
    if (d.account && !this.host.accounts().some((a) => a.id === d.account)) {
      return fail(
        422,
        'invalid_field',
        `No account ${JSON.stringify(d.account)} — pick another, or leave it unset.`,
        { field: 'account' },
      );
    }
    if (d.accountOnly && !d.account) {
      return fail(422, 'invalid_field', 'Only this account needs an account.', {
        field: 'accountOnly',
      });
    }
    return null;
  }

  private firstFreePort(): number {
    const used = new Set([
      this.host.main().port,
      ...RESERVED_PORTS,
      ...this.list().map((s) => s.port),
    ]);
    let port = FIRST_PORT;
    while (used.has(port)) port += 1;
    return port;
  }

  /** A slug of the name, made unique (`main` is taken). */
  private newId(name: string): string {
    const base =
      name
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, '-')
        .replace(/^-+|-+$/g, '')
        .slice(0, 32)
        .replace(/-+$/, '') || 'server';
    let id = base;
    for (let n = 2; id === MAIN_SERVER || this.servers.has(id); n++) id = `${base}-${n}`;
    return id;
  }

  // ─────────────────────────────────────────────────────────── The demo

  private seed(): void {
    const now = Date.now();
    const main = this.host.main();
    const thread = 'b7d3e9a1-4c2f-4e8a-9b6d-1f3a5c7e9b2d';
    const ref = this.host.accountRef(DEFAULT_ACCOUNT);
    this.host.setMain({
      ...main,
      running: true,
      startedAt: now - 42 * M,
      defaults: { cwd: '/Users/marcus/dev' },
      activeThread: thread,
      threads: [
        {
          threadId: thread,
          cwd: '/Users/marcus/dev/api-server',
          createdAt: now - 40 * M,
          permissionMode: 'default',
          title: 'Release checklist',
          persisted: true,
          ...(ref ? { account: ref } : {}),
        },
      ],
    });
    const work = 'f2a8c4e6-1b3d-4f5a-8c7e-9d0b2a4c6e8f';
    this.servers.set('work', {
      id: 'work',
      name: 'Work',
      port: 5613,
      defaults: {
        cwd: '/Users/marcus/dev/acme',
        permissionMode: 'acceptEdits',
        account: MOCK_ACCOUNT_WORK,
        accountOnly: true,
      },
      running: true,
      startedAt: now - 3 * H,
      open: false,
      activeThread: work,
      threads: [
        {
          threadId: work,
          cwd: '/Users/marcus/dev/acme',
          createdAt: now - 2 * H,
          permissionMode: 'acceptEdits',
          title: 'Triage the on-call queue',
          persisted: true,
        },
      ],
      tunnelUrl: 'https://acme-work.ngrok-free.app',
    });
    this.servers.set('personal', {
      id: 'personal',
      name: 'Personal',
      port: 5614,
      defaults: { account: MOCK_ACCOUNT_PRO, accountOnly: true },
      running: false,
      open: false,
      threads: [],
    });
  }
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function fail(status: number, code: string, message: string, extra: Body = {}): Response {
  return json({ error: { code, message, ...extra } }, status);
}
