// Dev-only: Claude Code's plugins for `?mock=1` (docs/PLUGINS.md): the
// Plugins page (`/api/plugins*`) and a session's MCP servers and plugins on
// its Info tab (`/api/sessions/{id}/mcp*`, `/plugins*`). Mirrors the
// server: changes are the desktop's, Mother Claude's own plugin is changed
// elsewhere, `needs-command` installs only once its command was accepted,
// and a session's `figma-mcp-node` failed at start until it is reconnected.
// Never bundled in production builds.

const D = 24 * 60 * 60 * 1000;

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function err(status: number, code: string, message: string, extra: object = {}): Response {
  return json({ error: { code, message, ...extra } }, status);
}

interface Installed {
  id: string;
  name: string;
  marketplace: string;
  version: string;
  scope: string;
  enabled: boolean;
  project?: string;
  mcpServers: { name: string; kind: string; host?: string; program?: string }[];
  ours: boolean;
  changeable: boolean;
  notes: string[];
  installedAt: string;
}

const CATALOG = [
  ['figma', 'Read and write Figma design files from Claude Code.', 18_400],
  ['postman', 'Design, test and document APIs with Postman.', 9_100],
  ['linear', 'Plan and track work in Linear.', 7_300],
  ['sentry', 'Look into errors and performance issues from Sentry.', 6_800],
  ['playwright', 'Drive a browser to test web apps.', 6_100],
  ['needs-command', 'A plugin its marketplace installs by running a command.', 120],
].map(([name, description, installCount]) => ({
  id: `${name}@claude-plugins-official`,
  name: String(name),
  description: String(description),
  marketplace: 'claude-plugins-official',
  installCount: Number(installCount),
}));

const COMMAND_SHA = 'a'.repeat(64);

/** A session's MCP servers and plugins. */
interface SessionState {
  servers: { name: string; status: string; error?: string; tools?: number }[];
  plugins: { name: string; source?: string; version?: string; path?: string }[];
  known: boolean;
}

export class MockPlugins {
  private installed: Installed[];
  private markets = [
    {
      name: 'claude-plugins-official',
      source: 'github',
      location: 'anthropics/claude-plugins-official',
      ours: false,
    },
    {
      name: 'mother-claude-local',
      source: 'directory',
      location: '/Users/marcus/Library/Application Support/Mother Claude/claude-plugin',
      ours: true,
    },
  ];
  private readonly sessions = new Map<string, SessionState>();

  constructor(
    private readonly desktop: boolean,
    now: number,
  ) {
    const at = (days: number) => new Date(now - days * D).toISOString();
    this.installed = [
      {
        id: 'figma@claude-plugins-official',
        name: 'figma',
        marketplace: 'claude-plugins-official',
        version: '2.2.157',
        scope: 'user',
        enabled: true,
        mcpServers: [{ name: 'figma', kind: 'http', host: 'mcp.figma.com' }],
        ours: false,
        changeable: true,
        notes: [],
        installedAt: at(40),
      },
      {
        id: 'mother-claude@mother-claude-local',
        name: 'mother-claude',
        marketplace: 'mother-claude-local',
        version: '0.14.0+mcp.1',
        scope: 'user',
        enabled: true,
        mcpServers: [{ name: 'mc', kind: 'http', host: '127.0.0.1:6725' }],
        ours: true,
        changeable: true,
        notes: [],
        installedAt: at(60),
      },
      {
        id: 'engineering@synced',
        name: 'engineering',
        marketplace: 'synced',
        version: '1.4.0',
        scope: 'synced',
        enabled: true,
        mcpServers: [],
        ours: false,
        changeable: false,
        notes: ['Synced from claude.ai'],
        installedAt: at(10),
      },
    ];
  }

  private view(project: string | null) {
    return {
      installed: this.installed
        .filter((p) => !p.project || p.project === project)
        .map(({ project: _p, ...p }) => ({
          ...p,
          ...(project ? { projectEnabled: !!_p } : {}),
        })),
      marketplaces: this.markets,
      project,
    };
  }

  route(path: string, method: string, body: unknown, q: URLSearchParams): Response | null {
    if (!path.startsWith('/plugins')) return null;
    const b = (body ?? {}) as Record<string, unknown>;
    if (path === '/plugins' && method === 'GET') return json(this.view(q.get('project')));
    if (path === '/plugins/available' && method === 'GET') {
      const words = (q.get('q') ?? '').toLowerCase().split(/\s+/).filter(Boolean);
      const hits = CATALOG.filter((p) =>
        words.every((w) => `${p.name} ${p.description}`.toLowerCase().includes(w)),
      );
      return json({ plugins: hits, total: hits.length, catalog: CATALOG.length });
    }
    if (method !== 'GET' && method !== 'DELETE' && !this.desktop) {
      return err(403, 'desktop_only', 'Plugins are changed in Mother Claude on the computer.');
    }
    if (path === '/plugins/reload' && method === 'POST') {
      return json({ reloaded: 2, held: 0, busy: 1, failed: 0 });
    }
    if (path === '/plugins/marketplaces' && method === 'POST') {
      const source = String(b['source'] ?? '').trim();
      if (!source || source.startsWith('-')) {
        return err(422, 'invalid_source', 'That is not a marketplace source.');
      }
      const name =
        source
          .split('/')
          .pop()
          ?.replace(/\.git$/, '') || 'market';
      this.markets = [...this.markets, { name, source: 'github', location: source, ours: false }];
      return json({ ok: true });
    }
    const market = /^\/plugins\/marketplaces\/([^/]+)$/.exec(path);
    if (market && method === 'DELETE') {
      if (!this.desktop) return err(403, 'desktop_only', 'Plugins are changed on the computer.');
      const name = decodeURIComponent(market[1]);
      if (name === 'mother-claude-local') {
        return err(409, 'plugin_is_ours', "Mother Claude's own marketplace goes with its plugin.");
      }
      this.markets = this.markets.filter((m) => m.name !== name);
      return json({ ok: true });
    }
    const verb = /^\/plugins\/(install|uninstall|enabled|update)$/.exec(path)?.[1];
    if (!verb || method !== 'POST') return null;
    const id = String(b['plugin'] ?? '');
    const scope = String(b['scope'] ?? 'user');
    const project = scope === 'user' ? undefined : String(b['project'] ?? '');
    if (id.startsWith('mother-claude@')) {
      return err(
        409,
        'plugin_is_ours',
        "Mother Claude's own plugin is turned on and off in Sessions started elsewhere.",
      );
    }
    const mine = this.installed.find(
      (p) => p.id === id && p.scope === scope && (p.project ?? undefined) === project,
    );
    if (verb === 'install') {
      if (id.startsWith('needs-command@') && b['acceptCommand'] !== COMMAND_SHA) {
        return err(
          409,
          'confirm_command',
          'The marketplace runs a command to install this plugin. Read it, and run it only if you trust it.',
          { command: 'npx @example/needs-command-installer --global', sha256: COMMAND_SHA },
        );
      }
      if (mine) return err(422, 'plugin_cli_failed', `Plugin '${id}' is already installed.`);
      const offer = CATALOG.find((p) => p.id === id);
      this.installed = [
        ...this.installed,
        {
          id,
          name: offer?.name ?? id.split('@')[0],
          marketplace: id.split('@')[1] ?? '',
          version: '1.0.0',
          scope,
          enabled: true,
          project,
          mcpServers: [],
          ours: false,
          changeable: true,
          notes: [],
          installedAt: new Date().toISOString(),
        },
      ];
      return json({ ok: true });
    }
    if (!mine) {
      return err(422, 'plugin_cli_failed', `Plugin '${id}' is not installed in scope ${scope}.`);
    }
    if (verb === 'uninstall') this.installed = this.installed.filter((p) => p !== mine);
    if (verb === 'enabled') mine.enabled = b['enabled'] === true;
    return json({ ok: true });
  }

  private session(id: string): SessionState {
    let s = this.sessions.get(id);
    if (!s) {
      s = {
        servers: [
          { name: 'github', status: 'connected', tools: 26 },
          { name: 'linear', status: 'connected', tools: 14 },
          {
            name: 'figma-mcp-node',
            status: 'failed',
            error: 'Connection closed: the Figma plugin is not running',
          },
        ],
        plugins: [
          { name: 'figma', source: 'figma@claude-plugins-official', version: '2.2.157' },
          { name: 'engineering', source: 'engineering@synced', version: '1.4.0' },
        ],
        known: true,
      };
      this.sessions.set(id, s);
    }
    return s;
  }

  /** A session's own: `sub` is what follows `/sessions/{id}/`. */
  routeSession(
    id: string,
    sub: string,
    method: string,
    body: unknown,
    how: { owned: boolean; tmux: boolean },
  ): Response | null {
    const b = (body ?? {}) as Record<string, unknown>;
    if (sub === 'mcp' && method === 'GET') {
      return how.owned ? json({ servers: this.session(id).servers }) : null;
    }
    if (sub === 'plugins' && method === 'GET') {
      const s = this.session(id);
      return json(
        how.owned
          ? { plugins: s.plugins, errors: [], known: s.known, reload: 'control' }
          : { plugins: [], errors: [], known: false, reload: how.tmux ? 'tmux' : 'none' },
      );
    }
    if (method !== 'POST') return null;
    const s = this.session(id);
    if (sub === 'mcp/reconnect' || sub === 'mcp/enabled') {
      const server = s.servers.find((x) => x.name === b['server']);
      if (!server) return err(422, 'cli_error', `Server not found: ${String(b['server'])}`);
      if (sub === 'mcp/reconnect') {
        server.status = 'connected';
        server.error = undefined;
        server.tools = server.tools ?? 31;
      } else {
        server.status = b['enabled'] ? 'connected' : 'disabled';
      }
      return json({ servers: s.servers });
    }
    if (sub === 'plugins/reload') {
      if (how.tmux && !how.owned) return json({ typed: true });
      return json({
        held: false,
        plugins: s.plugins,
        servers: s.servers,
        errorCount: 0,
        commands: 42,
      });
    }
    if (sub === 'plugins/add') {
      if (!this.desktop)
        return err(403, 'dangerous_blocked', 'Loading a plugin needs the desktop.');
      const path = String(b['path'] ?? '').trim();
      const name = path.split('/').filter(Boolean).pop() ?? 'plugin';
      s.plugins = [...s.plugins, { name, source: `${name}@inline`, path }];
      return json({
        held: false,
        plugins: s.plugins,
        servers: s.servers,
        errorCount: 0,
        commands: 42,
      });
    }
    return null;
  }
}
