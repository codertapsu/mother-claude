import { tr } from './i18n/i18n';
import { McpServerStatus } from './models';

/**
 * Claude Code's plugins, as Mother Claude shows and changes them
 * (docs/PLUGINS.md): what is installed and where it is on, the
 * marketplaces, the catalog, and a session's own plugins and MCP servers.
 * Every answer is read field by field: what the CLI adds is left out.
 */

/** Where a plugin is on: for you everywhere (`user`), for a project and
 * everyone who works on it (`project`), or for a project and only you
 * (`local`). */
export type PluginScope = 'user' | 'project' | 'local';

export const PLUGIN_SCOPES: readonly PluginScope[] = ['user', 'project', 'local'];

/** An MCP server a plugin brings: its kind and host, never its secrets. */
export interface PluginServer {
  name: string;
  kind: string;
  host?: string;
  program?: string;
}

export interface InstalledPlugin {
  /** `name@marketplace`. */
  id: string;
  name: string;
  marketplace?: string;
  version?: string;
  /** `user`, `project`, `local`, `managed`, `synced`, … */
  scope: string;
  enabled: boolean;
  /** On in the project the list was read in. */
  projectEnabled?: boolean;
  installedAt?: string;
  updatedAt?: string;
  mcpServers: PluginServer[];
  /** Mother Claude's own: changed in Sessions started elsewhere. */
  ours: boolean;
  /** Its scope is one this page changes. */
  changeable: boolean;
  notes: string[];
}

export interface Marketplace {
  name: string;
  /** `github`, `git`, `url`, `directory`, … */
  source: string;
  location?: string;
  ours: boolean;
}

export interface PluginsView {
  installed: InstalledPlugin[];
  marketplaces: Marketplace[];
  /** The project folder the list was read in. */
  project: string | null;
}

export interface AvailablePlugin {
  id: string;
  name: string;
  description: string;
  marketplace: string;
  installCount?: number;
}

export interface CatalogView {
  plugins: AvailablePlugin[];
  /** How many match (more than shown when the search is wide). */
  total: number;
  /** How many the marketplaces offer. */
  catalog: number;
}

/** A change on the Plugins page. */
export interface PluginChange {
  plugin: string;
  scope?: PluginScope;
  project?: string;
  account?: string;
  /** The SHA-256 of the marketplace's command the person accepted. */
  acceptCommand?: string;
  enabled?: boolean;
}

/** `POST /plugins/reload`: what every running session did. */
export interface ReloadAllResult {
  reloaded: number;
  held: number;
  busy: number;
  failed: number;
}

/** A plugin a session loaded. */
export interface SessionPlugin {
  name: string;
  source?: string;
  version?: string;
  path?: string;
}

/** How a session's plugins are read again from here: asked of it, typed
 * into its tmux pane, or by typing /reload-plugins in it. */
export type ReloadVia = 'control' | 'tmux' | 'none';

export interface SessionPluginsView {
  plugins: SessionPlugin[];
  errors: string[];
  /** The plugins are known (a turn started, or a reload answered). */
  known: boolean;
  reload: ReloadVia;
}

/** What a session's reload did. */
export interface PluginReload {
  /** Not done: the next message would re-read the whole conversation. */
  held: boolean;
  cacheImpact: { serversAdded: string[]; serversRemoved: string[]; lspToolChange: boolean } | null;
  plugins: SessionPlugin[];
  servers: McpServerStatus[];
  errorCount: number;
  commands: number;
  /** `/reload-plugins` was typed into its tmux pane. */
  typed: boolean;
}

// ─────────────────────────────── reading ───────────────────────────────

type Obj = Record<string, unknown>;

const obj = (v: unknown): Obj =>
  v && typeof v === 'object' && !Array.isArray(v) ? (v as Obj) : {};
const list = (v: unknown): unknown[] => (Array.isArray(v) ? v : []);
const str = (v: unknown): string | undefined => (typeof v === 'string' ? v : undefined);
const num = (v: unknown): number | undefined =>
  typeof v === 'number' && Number.isFinite(v) ? v : undefined;
const strings = (v: unknown): string[] => list(v).filter((x): x is string => typeof x === 'string');

function server(v: unknown): PluginServer | null {
  const o = obj(v);
  const name = str(o['name']);
  if (!name) return null;
  return { name, kind: str(o['kind']) ?? '', host: str(o['host']), program: str(o['program']) };
}

function installed(v: unknown): InstalledPlugin | null {
  const o = obj(v);
  const id = str(o['id']);
  if (!id) return null;
  return {
    id,
    name: str(o['name']) ?? id.split('@')[0],
    marketplace: str(o['marketplace']),
    version: str(o['version']),
    scope: str(o['scope']) ?? 'user',
    enabled: o['enabled'] !== false,
    projectEnabled: typeof o['projectEnabled'] === 'boolean' ? o['projectEnabled'] : undefined,
    installedAt: str(o['installedAt']),
    updatedAt: str(o['updatedAt']),
    mcpServers: list(o['mcpServers'])
      .map(server)
      .filter((s): s is PluginServer => !!s),
    ours: o['ours'] === true,
    changeable: o['changeable'] === true,
    notes: strings(o['notes']),
  };
}

function marketplace(v: unknown): Marketplace | null {
  const o = obj(v);
  const name = str(o['name']);
  if (!name) return null;
  return {
    name,
    source: str(o['source']) ?? '',
    location: str(o['location']),
    ours: o['ours'] === true,
  };
}

export function normalizePlugins(v: unknown): PluginsView {
  const o = obj(v);
  return {
    installed: list(o['installed'])
      .map(installed)
      .filter((p): p is InstalledPlugin => !!p),
    marketplaces: list(o['marketplaces'])
      .map(marketplace)
      .filter((m): m is Marketplace => !!m),
    project: str(o['project']) ?? null,
  };
}

export function normalizeCatalog(v: unknown): CatalogView {
  const o = obj(v);
  const plugins = list(o['plugins'])
    .map((p): AvailablePlugin | null => {
      const x = obj(p);
      const id = str(x['id']);
      if (!id) return null;
      return {
        id,
        name: str(x['name']) ?? id.split('@')[0],
        description: str(x['description']) ?? '',
        marketplace: str(x['marketplace']) ?? '',
        installCount: num(x['installCount']),
      };
    })
    .filter((p): p is AvailablePlugin => !!p);
  return { plugins, total: num(o['total']) ?? plugins.length, catalog: num(o['catalog']) ?? 0 };
}

function sessionPlugin(v: unknown): SessionPlugin | null {
  const o = obj(v);
  const name = str(o['name']);
  if (!name) return null;
  return { name, source: str(o['source']), version: str(o['version']), path: str(o['path']) };
}

function mcpServer(v: unknown): McpServerStatus | null {
  const o = obj(v);
  const name = str(o['name']);
  if (!name) return null;
  return {
    name,
    status: str(o['status']) ?? '',
    scope: str(o['scope']),
    source: str(o['source']),
    error: str(o['error']),
    tools: num(o['tools']),
  };
}

export function normalizeMcpServers(v: unknown): McpServerStatus[] {
  return list(obj(v)['servers'])
    .map(mcpServer)
    .filter((s): s is McpServerStatus => !!s);
}

export function normalizeSessionPlugins(v: unknown): SessionPluginsView {
  const o = obj(v);
  const via = str(o['reload']);
  return {
    plugins: list(o['plugins'])
      .map(sessionPlugin)
      .filter((p): p is SessionPlugin => !!p),
    errors: strings(o['errors']),
    known: o['known'] === true,
    reload: via === 'control' || via === 'tmux' ? via : 'none',
  };
}

export function normalizeReload(v: unknown): PluginReload {
  const o = obj(v);
  const impact = o['cacheImpact'] ? obj(o['cacheImpact']) : null;
  return {
    held: o['held'] === true,
    cacheImpact: impact
      ? {
          serversAdded: strings(impact['serversAdded']),
          serversRemoved: strings(impact['serversRemoved']),
          lspToolChange: impact['lspToolChange'] === true,
        }
      : null,
    plugins: list(o['plugins'])
      .map(sessionPlugin)
      .filter((p): p is SessionPlugin => !!p),
    servers: list(o['servers'])
      .map(mcpServer)
      .filter((s): s is McpServerStatus => !!s),
    errorCount: num(o['errorCount']) ?? 0,
    commands: num(o['commands']) ?? 0,
    typed: o['typed'] === true,
  };
}

export function normalizeReloadAll(v: unknown): ReloadAllResult {
  const o = obj(v);
  return {
    reloaded: num(o['reloaded']) ?? 0,
    held: num(o['held']) ?? 0,
    busy: num(o['busy']) ?? 0,
    failed: num(o['failed']) ?? 0,
  };
}

// ─────────────────────────────── words ───────────────────────────────

/** Where a plugin is on, in words. */
export function scopeText(scope: string): string {
  switch (scope) {
    case 'user':
      return tr('Everywhere');
    case 'project':
      return tr('This project, for everyone');
    case 'local':
      return tr('This project, only you');
    case 'managed':
      return tr('Set by your organization');
    case 'synced':
      return tr('From claude.ai');
    default:
      return scope;
  }
}

/** An MCP server's status, in words. */
export function mcpStatusText(status: string): string {
  switch (status) {
    case 'connected':
      return tr('Connected');
    case 'failed':
      return tr('Failed');
    case 'pending':
      return tr('Connecting…');
    case 'needs-auth':
      return tr('Needs sign-in');
    case 'disabled':
      return tr('Off in this session');
    default:
      return status;
  }
}

/** A server that a reconnect may bring back. */
export function mcpReconnectable(status: string): boolean {
  return status === 'failed' || status === 'needs-auth' || status === 'pending';
}
