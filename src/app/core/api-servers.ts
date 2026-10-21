import { ApiError, describeError, errorMessage } from './api.service';
import { BridgeDefaults, BridgeStatus, BridgeThread, DetectedTunnel, RemoteInfo } from './models';

/**
 * API servers (docs/ACCOUNTS.md §13.5): besides the main one — id `main`,
 * port 5612, the one there always was — servers on ports of their own, each
 * with its defaults and, when it names one, the Claude account it runs as.
 * Pure helpers: the wire's shapes read tolerantly (`GET /api/bridge` lists
 * them in `servers` only while one besides the main one exists), the
 * addresses a server answers on, the Add form's checks and the refusals in
 * words.
 */

/** The main server's id; its routes stay `/api/bridge/start|stop|active-thread`. */
export const MAIN_SERVER = 'main';
export const MAIN_SERVER_NAME = 'Main';
/** Longest server name, in characters (the server's rule). */
export const SERVER_NAME_MAX = 40;
/** Where the server starts looking for a free port when none is given. */
export const FIRST_SERVER_PORT = 5613;
const PORT_MIN = 1024;
const PORT_MAX = 65535;

export type TokenPolicy = BridgeStatus['tokenPolicy'];
const POLICIES: readonly TokenPolicy[] = ['required', 'local', 'never'];

/** A server's defaults: the main server's {@link BridgeDefaults} plus
 * `accountOnly` — it runs only as `account`; sent only when true, and only
 * with an account. */
export interface ServerDefaults extends BridgeDefaults {
  accountOnly?: boolean;
}

/** One server as `GET /api/bridge` lists it in `servers` (main first). */
export interface ServerStatus {
  id: string;
  name: string;
  running: boolean;
  port?: number;
  url?: string;
  /** Its path on the main API port and on the LAN mount: `/servers/<id>`
   * ('' for the main server). */
  prefix: string;
  startedAt?: number;
  requireToken: boolean;
  tokenPolicy: TokenPolicy;
  /** A detected tunnel points at its port. */
  exposedByTunnel: boolean;
  tunnelUrl?: string;
  defaults: ServerDefaults;
  activeThread?: string;
  threads: BridgeThread[];
  docsUrl?: string;
  consoleUrl?: string;
  /** Its saved account was removed: it cannot start until another is chosen. */
  accountMissing: boolean;
}

/** `POST /api/bridge/servers` → 201 its status (not started). */
export interface NewServer {
  name: string;
  /** Unset: the first free port from 5613. */
  port?: number;
  defaults?: ServerDefaults;
}

/** `PATCH /api/bridge/servers/{id}` (a running server restarts with it). */
export interface ServerPatch {
  name?: string;
  port?: number;
  defaults?: ServerDefaults;
}

// ───────────────────────────────────────────────────────── Reading the wire

function str(v: unknown): string | undefined {
  return typeof v === 'string' && v.trim() ? v : undefined;
}

function num(v: unknown): number | undefined {
  return typeof v === 'number' && Number.isFinite(v) ? v : undefined;
}

/** An http(s) URL from the wire (it ends up in an href); the trailing slash
 * dropped from a base URL. */
function webUrl(v: unknown, keepSlash = false): string | undefined {
  const s = str(v)?.trim();
  if (!s || !/^https?:\/\/\S+$/i.test(s)) return undefined;
  return keepSlash ? s : s.replace(/\/+$/, '');
}

const STRING_DEFAULTS = [
  'cwd',
  'model',
  'effort',
  'permissionMode',
  'systemPromptAppend',
  'defaultThread',
  'account',
] as const;

/** A server's defaults from the wire: the known fields only. */
export function normalizeDefaults(raw: unknown): ServerDefaults {
  if (!raw || typeof raw !== 'object') return {};
  const r = raw as Record<string, unknown>;
  const out: ServerDefaults = {};
  for (const key of STRING_DEFAULTS) {
    const v = str(r[key]);
    if (v) out[key] = v;
  }
  const thinking = r['thinking'];
  if (thinking === 'on' || thinking === 'off') out.thinking = thinking;
  const sources = r['settingSources'];
  if (Array.isArray(sources)) {
    out.settingSources = sources.filter((s): s is string => typeof s === 'string');
  }
  if (r['accountOnly'] === true) out.accountOnly = true;
  return out;
}

function normalizeThreads(raw: unknown): BridgeThread[] {
  if (!Array.isArray(raw)) return [];
  return raw.filter(
    (t): t is BridgeThread =>
      !!t && typeof t === 'object' && typeof (t as { threadId?: unknown }).threadId === 'string',
  );
}

/** `/servers/<id>` from the wire's `prefix` (a `/v1` on its end dropped),
 * else made from the id; '' for the main server. */
export function serverPrefix(id: string, raw?: unknown): string {
  if (id === MAIN_SERVER) return '';
  const p = str(raw)?.trim().replace(/\/+$/, '').replace(/\/v1$/, '');
  return p && /^(\/[A-Za-z0-9._~-]+)+$/.test(p) ? p : `/servers/${encodeURIComponent(id)}`;
}

/** One server from the wire, or null when it isn't one. */
export function normalizeServer(raw: unknown): ServerStatus | null {
  if (!raw || typeof raw !== 'object') return null;
  const r = raw as Record<string, unknown>;
  const id = str(r['id'])?.trim();
  if (!id) return null;
  const port = num(r['port']);
  const startedAt = num(r['startedAt']);
  const url = webUrl(r['url']);
  const tunnelUrl = webUrl(r['tunnelUrl']);
  const activeThread = str(r['activeThread']);
  const docsUrl = webUrl(r['docsUrl'], true);
  const consoleUrl = webUrl(r['consoleUrl'], true);
  return {
    id,
    name: str(r['name'])?.trim() ?? (id === MAIN_SERVER ? MAIN_SERVER_NAME : id),
    running: r['running'] === true,
    ...(port !== undefined ? { port } : {}),
    ...(url ? { url } : {}),
    prefix: serverPrefix(id, r['prefix']),
    ...(startedAt !== undefined ? { startedAt } : {}),
    requireToken: r['requireToken'] === true,
    tokenPolicy: POLICIES.find((p) => p === r['tokenPolicy']) ?? 'local',
    exposedByTunnel: r['exposedByTunnel'] === true,
    ...(tunnelUrl ? { tunnelUrl } : {}),
    defaults: normalizeDefaults(r['defaults']),
    ...(activeThread ? { activeThread } : {}),
    threads: normalizeThreads(r['threads']),
    ...(docsUrl ? { docsUrl } : {}),
    ...(consoleUrl ? { consoleUrl } : {}),
    accountMissing: r['accountMissing'] === true,
  };
}

/** The `servers` field as it came, or undefined when there is none. */
function rawServers(status: BridgeStatus): unknown[] | undefined {
  const raw = (status as BridgeStatus & { servers?: unknown }).servers;
  return Array.isArray(raw) ? raw : undefined;
}

function idOf(raw: unknown): string | undefined {
  return raw && typeof raw === 'object' ? str((raw as { id?: unknown }).id)?.trim() : undefined;
}

/** The main server as a list entry. Its state is the top-level payload's
 * (`GET /api/bridge` is the main server's status, as before); only its name
 * may come from its `servers` entry. */
export function mainServerOf(status: BridgeStatus, entry?: ServerStatus | null): ServerStatus {
  const main = normalizeServer({ ...status, id: MAIN_SERVER, prefix: '' }) as ServerStatus;
  return { ...main, name: entry?.name ?? MAIN_SERVER_NAME, accountMissing: false };
}

/**
 * Every server, main first — or none while the main one is the only one
 * (no `servers` field, or one listing only it): the screen then looks and
 * behaves as it did before servers.
 */
export function serversOf(status: BridgeStatus | null): ServerStatus[] {
  const raw = status ? rawServers(status) : undefined;
  if (!status || !raw) return [];
  const seen = new Set<string>();
  const others: ServerStatus[] = [];
  let main: ServerStatus | null = null;
  for (const item of raw) {
    const s = normalizeServer(item);
    if (!s || seen.has(s.id)) continue;
    seen.add(s.id);
    if (s.id === MAIN_SERVER) main = s;
    else others.push(s);
  }
  return others.length ? [mainServerOf(status, main), ...others] : [];
}

/** The payload the API screen's panel shows for `server`: the main payload
 * itself for the main server, else the main payload (the shared host, the
 * sign-in, the configured policy) with the server's own fields. */
export function serverPanel(status: BridgeStatus, server: ServerStatus | null): BridgeStatus {
  return !server || server.id === MAIN_SERVER ? status : withServerFields(status, server);
}

function withServerFields(status: BridgeStatus, s: ServerStatus): BridgeStatus {
  const url = s.url ?? (s.port ? `http://127.0.0.1:${s.port}` : undefined);
  return {
    ...status,
    running: s.running,
    port: s.port,
    url,
    startedAt: s.startedAt,
    requireToken: s.requireToken,
    tokenPolicy: s.tokenPolicy,
    exposedByTunnel: s.exposedByTunnel,
    tunnelUrl: s.tunnelUrl,
    defaults: s.defaults,
    activeThread: s.activeThread,
    threads: s.threads,
    docsUrl: s.docsUrl ?? (url ? `${url}/docs/` : undefined),
    consoleUrl: s.consoleUrl ?? (url ? `${url}/example/` : undefined),
  };
}

/** A whole `GET /api/bridge` payload, not one server's status. */
function isFullStatus(r: Record<string, unknown>): boolean {
  return typeof r['id'] !== 'string' && (typeof r['enabled'] === 'boolean' || 'hostBuilt' in r);
}

/**
 * The status after an action's answer: a whole payload (the main server's
 * routes) replaces it, keeping the servers when it lists none; one server's
 * status (the server routes) replaces that server's entry, or is added.
 */
export function absorbReply(status: BridgeStatus | null, reply: unknown): BridgeStatus | null {
  if (!reply || typeof reply !== 'object') return status;
  const r = reply as Record<string, unknown>;
  if (isFullStatus(r)) {
    const full = reply as BridgeStatus;
    const kept = status ? rawServers(status) : undefined;
    return rawServers(full) || !kept ? full : ({ ...full, servers: kept } as BridgeStatus);
  }
  const server = normalizeServer(reply);
  if (!server || !status) return status;
  if (server.id === MAIN_SERVER) return withServerFields(status, server);
  const list = rawServers(status) ?? [{ id: MAIN_SERVER, name: MAIN_SERVER_NAME }];
  const at = list.findIndex((x) => idOf(x) === server.id);
  const next = at < 0 ? [...list, reply] : list.map((x, i) => (i === at ? reply : x));
  return { ...status, servers: next } as BridgeStatus;
}

/** The status without a removed server (no `servers` once only the main
 * one is left). */
export function withoutServer(status: BridgeStatus | null, id: string): BridgeStatus | null {
  const list = status ? rawServers(status) : undefined;
  if (!status || !list) return status;
  const next = list.filter((x) => idOf(x) !== id);
  const copy = { ...status } as BridgeStatus & { servers?: unknown[] };
  if (next.some((x) => idOf(x) !== MAIN_SERVER)) copy.servers = next;
  else delete copy.servers;
  return copy;
}

// ─────────────────────────────────────────────────────────── The account

/** The account a server runs under, in words. */
export interface ServerAccount {
  /** The account it names; absent: automatic (the project's, else the
   * accounts' API default). */
  id?: string;
  /** "Automatic", the account's name, or "A removed account". */
  name: string;
  /** It runs only as that account. */
  only: boolean;
  /** Its account was removed. */
  missing: boolean;
}

export function serverAccount(
  s: Pick<ServerStatus, 'defaults' | 'accountMissing'>,
  nameOf: (id: string) => string | undefined,
): ServerAccount {
  const id = s.defaults.account;
  if (!id) return { name: 'Automatic', only: false, missing: false };
  const only = s.defaults.accountOnly === true;
  if (s.accountMissing) return { id, name: 'A removed account', only, missing: true };
  return { id, name: nameOf(id) ?? id, only, missing: false };
}

// ─────────────────────────────────────────────────────── The Add form

function hasControl(s: string): boolean {
  for (const c of s) {
    const code = c.codePointAt(0) ?? 0;
    if (code < 0x20 || (code >= 0x7f && code <= 0x9f)) return true;
  }
  return false;
}

/** What is wrong with a server name ('' when nothing): 1–40 characters, no
 * control characters, not another server's (case aside; "Main" is taken). */
export function serverNameProblem(
  name: string,
  servers: readonly Pick<ServerStatus, 'id' | 'name'>[],
  exceptId?: string,
): string {
  const n = name.trim();
  if (!n) return 'Give it a name.';
  if ([...n].length > SERVER_NAME_MAX) return `At most ${SERVER_NAME_MAX} characters.`;
  if (hasControl(n)) return 'Leave out control characters.';
  const lower = n.toLowerCase();
  const taken = [
    ...servers.filter((s) => s.id !== MAIN_SERVER),
    { id: MAIN_SERVER, name: MAIN_SERVER_NAME },
  ].some((s) => s.id !== exceptId && s.name.trim().toLowerCase() === lower);
  return taken ? 'Another API server is called that.' : '';
}

/** The port typed: undefined when empty (the server picks), NaN when it
 * isn't a whole number. */
export function parsePort(text: string): number | undefined {
  const t = text.trim();
  if (!t) return undefined;
  return /^\d{1,5}$/.test(t) ? Number(t) : NaN;
}

/** What is wrong with a port typed ('' when nothing): 1024–65535, not one
 * another server (or the main one) has. The server checks the rest. */
export function portProblem(
  text: string,
  servers: readonly Pick<ServerStatus, 'id' | 'name' | 'port'>[],
  exceptId?: string,
  mainPort?: number,
): string {
  const p = parsePort(text);
  if (p === undefined) return '';
  if (!Number.isInteger(p) || p < PORT_MIN || p > PORT_MAX) {
    return `Enter a port from ${PORT_MIN} to ${PORT_MAX}, or leave it empty.`;
  }
  const holder = servers.find((s) => s.id !== exceptId && s.port === p);
  if (holder?.id === MAIN_SERVER || (!holder && mainPort === p && exceptId !== MAIN_SERVER)) {
    return 'The main server uses that port.';
  }
  return holder ? `${holder.name} uses that port.` : '';
}

// ─────────────────────────────────────────────────────────── Addresses

/** Where a server can be reached from, best first for the one asking. */
export interface ServerAddress {
  key: 'own' | 'viaMain' | 'lan' | 'tunnel' | 'mainTunnel';
  label: string;
  /** Null when not known here: `pattern` shows its shape instead. */
  url: string | null;
  pattern?: string;
  hint: string;
}

export interface AddressContext {
  /** The desktop app, on the Mac: its loopback addresses lead. A phone
   * leads with what it can reach (the network, a tunnel). */
  desktop: boolean;
  /** The main server: its port (and a tunnel to it) reaches every server. */
  main: Pick<ServerStatus, 'port' | 'running' | 'tunnelUrl'>;
  /** Mother Claude on the LAN, `https://<mac>:6725`, when known here. */
  lanBase: string | null;
  /** The dashboard's port, for the shape of the LAN address. */
  dashboardPort?: number;
}

/** The addresses `s` answers on (docs/ACCOUNTS.md §13.5 "Reaching them"):
 * its own port and, for a server besides the main one, its prefix on the
 * main port — both on the Mac only —, the LAN mount (a paired device's
 * token, always) and any tunnel that points at it or at the main port.
 * With no API port of its own (`MOTHER_CLAUDE_BRIDGE_PORT=0`) the main
 * server answers on Mother Claude's own port, and every server under its
 * prefix there: a token always. */
export function serverAddresses(s: ServerStatus, ctx: AddressContext): ServerAddress[] {
  const main = s.id === MAIN_SERVER;
  const where = ctx.desktop ? 'On this Mac' : 'On the Mac';
  // A phone can't reach the Mac's loopback: say so rather than the token.
  const macOnly = 'For programs on the Mac itself: this device cannot reach it.';
  // Mother Claude's own port, when the main server has none of its own.
  const appPort = !ctx.main.port && ctx.dashboardPort ? ctx.dashboardPort : 0;
  const ownPort: ServerAddress[] = s.port
    ? [
        {
          key: 'own',
          label: where,
          url: `http://127.0.0.1:${s.port}/v1`,
          hint: ctx.desktop ? tokenText(s) : macOnly,
        },
      ]
    : [];
  const own: ServerAddress[] =
    main && !s.port && appPort
      ? [
          {
            key: 'own',
            label: `${where}, on Mother Claude's port`,
            url: `http://127.0.0.1:${appPort}/v1`,
            hint: ctx.desktop
              ? 'Always with a token: the main server has no port of its own.'
              : macOnly,
          },
        ]
      : ownPort;
  const viaMain: ServerAddress[] = [];
  if (!main && ctx.main.port) {
    viaMain.push({
      key: 'viaMain',
      label: `${where}, through the main port`,
      url: `http://127.0.0.1:${ctx.main.port}${s.prefix}/v1`,
      hint:
        (ctx.desktop
          ? `One port reaches every server; ${s.name} keeps its own account and token rules.`
          : macOnly) + (ctx.main.running ? '' : ' Only while the main server runs.'),
    });
  } else if (!main && appPort) {
    viaMain.push({
      key: 'viaMain',
      label: `${where}, through Mother Claude's port`,
      url: `http://127.0.0.1:${appPort}${s.prefix}/v1`,
      hint: ctx.desktop
        ? `Mother Claude's own port reaches every server, always with a token; ${s.name} keeps its own account.`
        : macOnly,
    });
  }
  const lan: ServerAddress = ctx.lanBase
    ? {
        key: 'lan',
        label: 'On your network',
        url: `${ctx.lanBase}${s.prefix}/v1`,
        hint: "Over TLS on Mother Claude's own port, always with a paired device's token.",
      }
    : {
        key: 'lan',
        label: 'On your network',
        url: null,
        pattern: `https://<the Mac's address>:${ctx.dashboardPort || 6725}${s.prefix}/v1`,
        hint: "Over TLS on Mother Claude's own port, always with a paired device's token. The Mac's Remote page shows its address.",
      };
  const tunnels: ServerAddress[] = [];
  if (s.tunnelUrl) {
    tunnels.push({
      key: 'tunnel',
      label: 'Through its tunnel',
      url: `${s.tunnelUrl}/v1`,
      hint:
        s.tokenPolicy === 'never'
          ? 'Open to tunnels: no token needed.'
          : "With a paired device's token, while the tunnel points at its port.",
    });
  }
  if (!main && ctx.main.tunnelUrl) {
    tunnels.push({
      key: 'mainTunnel',
      label: "Through the main port's tunnel",
      url: `${ctx.main.tunnelUrl}${s.prefix}/v1`,
      hint:
        s.tokenPolicy === 'never'
          ? `${s.name} is open to tunnels: no token needed.`
          : "With a paired device's token.",
    });
  }
  if (ctx.desktop) return [...own, ...viaMain, lan, ...tunnels];
  // A phone: what it can reach first — the network when it is on it, else
  // a tunnel — and the Mac's own loopback last.
  return ctx.lanBase
    ? [lan, ...tunnels, ...own, ...viaMain]
    : [...tunnels, lan, ...own, ...viaMain];
}

/** Who may call a server's own port without a token. */
function tokenText(s: Pick<ServerStatus, 'tokenPolicy' | 'exposedByTunnel'>): string {
  switch (s.tokenPolicy) {
    case 'required':
      return 'A token is required.';
    case 'never':
      return 'No token, from anywhere that reaches the port.';
    default:
      return s.exposedByTunnel
        ? 'A token is required while a tunnel points at the port.'
        : 'Programs on this Mac call it without a token.';
  }
}

/** A private-network host (RFC 1918, link-local, Tailscale's 100.64/10,
 * `*.local`), as the Pair page tells a LAN origin. */
function isPrivateHost(host: string): boolean {
  return (
    /^10\./.test(host) ||
    /^192\.168\./.test(host) ||
    /^172\.(1[6-9]|2\d|3[01])\./.test(host) ||
    /^169\.254\./.test(host) ||
    /^100\.(6[4-9]|[7-9]\d|1[01]\d|12[0-7])\./.test(host) ||
    host.endsWith('.local')
  );
}

/** Mother Claude's address on the LAN (`https://<mac>:6725`): the desktop's
 * first LAN address (`GET /api/remote`), else this page's own origin when it
 * was opened over the LAN. Null when it isn't known here. */
export function lanBaseOf(remote: RemoteInfo | null, origin: string): string | null {
  if (remote) {
    if (remote.lan?.enabled === false) return null;
    const first = webUrl(remote.lan?.addresses?.[0]?.url);
    if (first) return first;
  }
  try {
    const u = new URL(origin);
    return u.protocol === 'https:' && isPrivateHost(u.hostname) ? u.origin : null;
  } catch {
    return null;
  }
}

// ─────────────────────────────────────────────────── Tunnels (Remote page)

/** An API server as `GET /api/remote`'s `bridge.servers` lists it. */
export interface RemoteServer {
  id: string;
  name: string;
  port?: number;
  running: boolean;
  /** A tunnel points at its port. */
  exposed: boolean;
}

/** The API servers the Remote page knows (main first), or none while the
 * main one is the only one. */
export function remoteServers(remote: RemoteInfo | null): RemoteServer[] {
  const raw = (remote?.bridge as { servers?: unknown } | undefined)?.servers;
  if (!Array.isArray(raw)) return [];
  const out: RemoteServer[] = [];
  for (const item of raw) {
    if (!item || typeof item !== 'object') continue;
    const r = item as Record<string, unknown>;
    const id = str(r['id'])?.trim();
    if (!id || out.some((s) => s.id === id)) continue;
    const port = num(r['port']);
    out.push({
      id,
      name: str(r['name'])?.trim() ?? (id === MAIN_SERVER ? MAIN_SERVER_NAME : id),
      ...(port !== undefined ? { port } : {}),
      running: r['running'] === true,
      exposed: r['exposed'] === true,
    });
  }
  return out.some((s) => s.id !== MAIN_SERVER) ? out : [];
}

/** The port a tunnel forwards to (`upstreamPort`, else read from `upstream`). */
export function upstreamPort(t: Pick<DetectedTunnel, 'upstream' | 'upstreamPort'>): number | null {
  if (t.upstreamPort) return t.upstreamPort;
  const s = (t.upstream ?? '').trim();
  const rest = s.includes('://') ? s.slice(s.indexOf('://') + 3) : s;
  const hostPort = rest.split('/')[0];
  const m = /^(?:.*:)?(\d{1,5})$/.exec(hostPort);
  return m ? Number(m[1]) : null;
}

/** The API server a detected tunnel points at, while servers besides the
 * main one exist: its `server` (absent: the main one, for target `bridge`),
 * else the server whose port it forwards to. */
export function tunnelServer(t: DetectedTunnel, servers: RemoteServer[]): RemoteServer | null {
  if (!servers.length) return null;
  const tag = (t as DetectedTunnel & { server?: unknown }).server;
  const id = str(tag)?.trim();
  // A server the list doesn't have (yet): still name it.
  if (id)
    return servers.find((s) => s.id === id) ?? { id, name: id, running: false, exposed: true };
  const port = upstreamPort(t);
  const byPort = port ? servers.find((s) => s.port === port) : undefined;
  if (byPort) return byPort;
  return t.target === 'bridge' ? (servers.find((s) => s.id === MAIN_SERVER) ?? null) : null;
}

// ─────────────────────────────────────────────────────────── Refusals

/** The server's own sentence, ending in a full stop ('' when it sent none:
 * ApiService then puts the bare status there). */
function serverSaid(e: ApiError): string {
  const m = e.message.trim();
  if (!m || /^\d{3}\b/.test(m)) return '';
  return /[.!?]$/.test(m) ? m : `${m}.`;
}

function portIn(e: ApiError): string {
  const v = e.extra['port'];
  return typeof v === 'number' || (typeof v === 'string' && v) ? String(v) : '';
}

/** The server a refusal is about, for {@link serverErrorText}. */
export interface RefusalContext {
  /** Its name ("the API" for the main one while it is the only one). */
  server?: string;
  /** It is the main server (its port is the environment's to change). */
  main?: boolean;
  /** The account it runs only as, by name. */
  account?: string;
  /** Every server, to name the one holding a port. */
  servers?: readonly Pick<ServerStatus, 'id' | 'name'>[];
  /** The Add or Manage form, where a port can be left empty. */
  saving?: boolean;
}

/** A refusal of something done on the API screen, in words (the server's
 * own words for the rest, as before). */
export function serverErrorText(e: unknown, ctx: RefusalContext = {}): string {
  if (!(e instanceof ApiError)) return errorMessage(e);
  if (e.kind !== 'http') return describeError(e).text;
  const text = refusalText(e, ctx);
  return text.charAt(0).toUpperCase() + text.slice(1);
}

function refusalText(e: ApiError, ctx: RefusalContext): string {
  const name = ctx.server || 'this server';
  switch (e.code) {
    case 'port_unavailable': {
      const port = portIn(e);
      const which = port ? `Port ${port}` : 'That port';
      const holderId = e.detail('server');
      const holder = holderId
        ? (ctx.servers?.find((s) => s.id === holderId)?.name ?? holderId)
        : '';
      if (holder) {
        return `${which} is ${holder}'s. Pick another, or leave the port empty for the first free one.`;
      }
      if (ctx.saving) {
        // The server knows whose it is ("… the Mother Claude app's own port.").
        const said = serverSaid(e);
        return `${said || `${which} is taken: the main server, Mother Claude's own ports or another program has it.`} Pick another, or leave it empty for the first free one.`;
      }
      return ctx.main
        ? `${which} is in use by another program, so ${name} cannot listen on it. Stop that program, or set MOTHER_CLAUDE_BRIDGE_PORT to another port.`
        : `${which} is in use by another program, so ${name} cannot listen on it. Stop that program, or give ${name} another port.`;
    }
    case 'account_not_found':
      return `${name}'s account was removed. Choose another account (or Automatic) for it, then start it.`;
    case 'account_not_served':
      return `${name} runs only as ${ctx.account || 'its own account'}: another account's conversations can't be used through it.`;
    case 'account_used_by_bridge':
      return accountInUseText(e);
    case 'server_not_found':
      return 'That API server no longer exists. Reload the page to see the ones there are.';
    case 'main_server_fixed':
      return 'The main server keeps its name and port, and cannot be removed.';
    case 'server_name_taken':
      return 'Another API server is called that. Pick another name.';
    case 'thread_not_found':
      return 'That conversation no longer exists. Pick another one.';
    case 'dangerous_blocked':
      return describeError(e).text;
    case 'desktop_only':
      return e.message || 'Only the desktop app can do this.';
    default:
      return errorMessage(e);
  }
}

/**
 * `409 account_used_by_bridge` (removing an account a running server uses),
 * in words: it names the server when it is not the main one. `then`: what
 * to do after stopping it.
 */
export function accountInUseText(e: ApiError, account = 'this account', then = 'first'): string {
  const server = e.detail('serverName') || e.detail('server');
  return server
    ? `The API server ${server} is running under ${account}. Stop it, or start it with another account, ${then}.`
    : `The API is running with ${account} as the account its new conversations start under. Stop the API, or start it again with another account, ${then}.`;
}
