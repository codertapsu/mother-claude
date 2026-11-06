// Dev-only: Mother Claude's MCP server as the Services page shows it
// (`GET /api/mcp`, `PUT /api/mcp/settings`, docs/MCP.md) for `?mock=1`.
// `?mockmcp=outdated` shows an installed plugin older than the one the app
// writes, `?mockmcp=missing` no plugin at all. Never bundled in production
// builds.

import { McpCall, McpPluginInfo, McpToolsStatus } from '../models';
import { urlParam } from '../platform';

const S = 1000;

/** The browser tools (`extension::mcp`), by name and title. */
const BROWSER_TOOLS = (
  [
    ['browser_tabs', 'Browser tabs'],
    ['browser_navigate', 'Open a URL'],
    ['browser_back', 'Go back'],
    ['browser_forward', 'Go forward'],
    ['browser_reload', 'Reload'],
    ['browser_snapshot', 'Read the page (snapshot)'],
    ['browser_read', "Read the page's content"],
    ['browser_find', 'Find elements'],
    ['browser_click', 'Click'],
    ['browser_hover', 'Hover'],
    ['browser_type', 'Type text'],
    ['browser_fill_form', 'Fill a form'],
    ['browser_select', 'Choose options'],
    ['browser_check', 'Check or uncheck'],
    ['browser_press', 'Press a key'],
    ['browser_scroll', 'Scroll'],
    ['browser_wait', 'Wait'],
    ['browser_screenshot', 'Screenshot'],
    ['browser_eval', 'Run JavaScript'],
    ['browser_console', 'Console messages'],
    ['browser_dialog', 'Answer a dialog'],
    ['browser_upload', 'Upload files'],
  ] as const
).map(([name, title]) => ({ name, title }));

/** The team tools (`orchestration::mcp`), by name and title. */
const TEAM_TOOLS = (
  [
    ['task_create', 'Write a task packet'],
    ['task_list', "List the board's tasks"],
    ['task_get', 'Read a task'],
    ['task_claim', 'Claim a task'],
    ['task_update', 'Update a task'],
    ['task_done', 'Finish a task'],
    ['team_roles', 'Who is on the team'],
    ['team_status', 'The team now'],
    ['team_request_role', "Start a role's session"],
    ['session_summary', "What a role's session is doing"],
  ] as const
).map(([name, title]) => ({ name, title }));
const VERSION = '0.7.0+mcp.1';

export class MockMcp {
  private enabled = true;
  private readonly calls: McpCall[];
  private readonly plugin: McpPluginInfo;

  constructor(
    private readonly desktop: boolean,
    now: number,
  ) {
    this.calls = [
      {
        at: now - 12 * S,
        session: 's-auth',
        owned: true,
        tool: 'mc_session',
        ok: true,
        durationMs: 3,
      },
      { at: now - 40 * S, session: 's-db', owned: false, tool: 'mc_ping', ok: true, durationMs: 1 },
      { at: now - 95 * S, owned: false, tool: 'mc_session', ok: false, durationMs: 2 },
    ];
    const mode = urlParam('mockmcp');
    this.plugin = {
      installed: mode !== 'missing',
      enabled: mode !== 'missing',
      version: mode === 'missing' ? undefined : mode === 'outdated' ? '0.6.2' : VERSION,
      expectedVersion: VERSION,
      outdated: mode === 'outdated',
      cliAvailable: true,
    };
  }

  route(path: string, method: string, body: unknown): Response | null {
    if (path === '/mcp' && method === 'GET') return json(this.status());
    if (path === '/mcp/settings' && method === 'PUT') {
      if (!this.desktop) {
        return json(
          { error: { code: 'desktop_only', message: 'Only the desktop app can do this.' } },
          403,
        );
      }
      const enabled = (body as { enabled?: unknown } | null)?.enabled;
      if (typeof enabled !== 'boolean') {
        return json(
          { error: { code: 'invalid_body', message: '"enabled" must be a boolean.' } },
          422,
        );
      }
      this.enabled = enabled;
      return json(this.status());
    }
    return null;
  }

  private status(): McpToolsStatus {
    // What the app registers: core, browser, teams (`mcp::registry`).
    const tools = [
      { name: 'mc_ping', title: 'Ping Mother Claude' },
      { name: 'mc_session', title: 'This session in Mother Claude' },
      ...BROWSER_TOOLS,
      ...TEAM_TOOLS,
    ];
    return {
      enabled: this.enabled,
      url: 'http://127.0.0.1:6725/mcp',
      toolPrefix: 'mcp__plugin_mother-claude_mc__',
      tools,
      toolCount: tools.length,
      entryWritten: this.enabled,
      ownedSessions: this.enabled,
      clients: this.enabled ? 2 : 0,
      callsTotal: 17,
      recentCalls: this.calls,
      plugin: this.desktop ? this.plugin : undefined,
    };
  }
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}
