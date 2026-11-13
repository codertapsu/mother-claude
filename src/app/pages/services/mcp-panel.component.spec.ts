import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { FakeBackend, asDesktop, flush, useFakeServer } from '../../core/mock/fake-server.testing';
import { ownedCaps } from '../../core/mock/fixtures';
import { setActiveMock } from '../../core/mock/mock-api';
import { McpPluginInfo, McpToolsStatus, SessionSummary } from '../../core/models';
import { RealtimeService } from '../../core/realtime.service';
import { McpPanelComponent } from './mcp-panel.component';

const plugin = (over: Partial<McpPluginInfo> = {}): McpPluginInfo => ({
  installed: true,
  enabled: true,
  version: '0.7.0+mcp.1',
  expectedVersion: '0.7.0+mcp.1',
  outdated: false,
  cliAvailable: true,
  ...over,
});

const status = (over: Partial<McpToolsStatus> = {}): McpToolsStatus => ({
  enabled: true,
  url: 'http://127.0.0.1:6725/mcp',
  toolPrefix: 'mcp__plugin_mother-claude_mc__',
  tools: [{ name: 'mc_ping' }, { name: 'mc_session' }],
  toolCount: 2,
  entryWritten: true,
  ownedSessions: true,
  clients: 1,
  callsTotal: 3,
  recentCalls: [
    {
      at: Date.now() - 5000,
      session: 's1',
      owned: true,
      tool: 'mc_session',
      ok: true,
      durationMs: 2,
    },
    { at: Date.now() - 9000, owned: false, tool: 'mc_ping', ok: false, durationMs: 1 },
  ],
  ...over,
});

const session = (id: string, title: string): SessionSummary =>
  ({
    id,
    name: id,
    title,
    projectName: 'api',
    cwd: '/Users/me/dev/api',
    surface: 'mother-claude',
    owned: true,
    live: true,
    status: 'idle',
    activity: { kind: 'idle', label: 'Idle' },
    usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
    messageCount: 1,
    agents: { running: 0, total: 0, workflowsRunning: 0, backgroundRunning: 0 },
    queued: 0,
    pending: [],
    control: ownedCaps(),
  }) as SessionSummary;

describe('McpPanelComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<McpPanelComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    (
      TestBed.inject(RealtimeService) as unknown as { replaceStore(l: SessionSummary[]): void }
    ).replaceStore([session('s1', 'Fix the login flow')]);
  });

  afterEach(() => {
    fixture?.destroy();
    setActiveMock(null);
  });

  const text = (el: Element) => (el.textContent ?? '').replace(/\s+/g, ' ').trim();
  const render = async (body: McpToolsStatus) => {
    backend.routes.set('/mcp', () => ({ status: 200, body }));
    fixture = TestBed.createComponent(McpPanelComponent);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    return fixture.nativeElement as HTMLElement;
  };

  it('shows the tools and the last calls, read-only on a phone', async () => {
    const el = await render(status());
    expect(text(el)).toContain('Claude Code tools (MCP)');
    expect(text(el)).toContain('mcp__plugin_mother-claude_mc__');
    // A phone sees the switch's value and where it changes, never a
    // disabled switch.
    expect(el.querySelector('input[type=checkbox]')).toBeNull();
    const row = el.querySelector('.elsewhere') as HTMLElement;
    expect(text(row)).toContain("Give sessions Mother Claude's tools");
    expect(text(row.querySelector('mc-desktop-only .value')!)).toBe('On');
    expect(text(row)).toContain('Changed on the desktop app.');
    const calls = el.querySelectorAll('.call');
    expect(calls.length).toBe(2);
    expect(text(calls[0])).toContain('mc_session');
    expect(text(calls[0])).toContain('Fix the login flow · owned');
    expect(text(calls[0])).toContain('ok');
    expect(text(calls[1])).toContain('unknown session');
    expect(text(calls[1])).toContain('error');
    expect(el.querySelector('.hint')).toBeNull();
    // The first read asks for everything; polls leave the plugin out.
    expect(backend.urls[0]).toBe('/mcp');
  });

  it('asks to update a plugin older than the one this app writes', async () => {
    asDesktop();
    const el = await render(status({ plugin: plugin({ version: '0.6.2', outdated: true }) }));
    const hint = el.querySelector('.hint') as HTMLElement;
    expect(text(hint)).toContain("Update the plugin to give sessions Mother Claude's tools");
    expect(text(hint)).toContain('installed: v0.6.2, this app: v0.7.0+mcp.1');
    expect(hint.querySelector('a')?.getAttribute('href')).toBe('/settings/elsewhere');
  });

  it('asks to install a missing plugin, and says nothing when it is current', async () => {
    asDesktop();
    let el = await render(status({ plugin: plugin({ installed: false, version: undefined }) }));
    expect(text(el.querySelector('.hint') as HTMLElement)).toContain(
      'once the Mother Claude plugin is installed',
    );
    fixture.destroy();
    el = await render(status({ plugin: plugin() }));
    expect(el.querySelector('.hint')).toBeNull();
    expect(text(el)).toContain('installed v0.7.0+mcp.1');
  });

  it('turns the tools off from the desktop', async () => {
    asDesktop();
    backend.routes.set('/mcp/settings', (_method, body) => ({
      status: 200,
      body: status({ enabled: (body as { enabled: boolean }).enabled, ownedSessions: false }),
    }));
    const el = await render(status({ plugin: plugin() }));
    const toggle = el.querySelector('input[type=checkbox]') as HTMLInputElement;
    expect(toggle.disabled).toBe(false);
    toggle.checked = false;
    toggle.dispatchEvent(new Event('change'));
    await flush();
    fixture.detectChanges();
    expect(backend.calls).toContain('PUT /mcp/settings');
    expect(backend.bodyOf('PUT /mcp/settings')).toEqual({ enabled: false });
    expect(text(el)).toContain('Off: sessions get no tools');
    expect(text(el)).toContain('get no tools');
  });
});
