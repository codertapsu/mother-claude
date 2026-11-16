import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { FakeBackend, flush, useFakeServer } from '../../core/mock/fake-server.testing';
import { ownedCaps } from '../../core/mock/fixtures';
import { setActiveMock } from '../../core/mock/mock-api';
import { SessionDetail, SessionSummary } from '../../core/models';
import { NoticeService } from '../../core/notice.service';
import { RealtimeService } from '../../core/realtime.service';
import { SessionContext } from './session-context';
import { SessionPluginsComponent } from './session-plugins.component';

const session = (over: Partial<SessionSummary> = {}): SessionDetail =>
  ({
    id: 's1',
    name: 'design',
    projectName: 'site',
    cwd: '/Users/me/dev/site',
    surface: 'mother-claude',
    owned: true,
    live: true,
    status: 'idle',
    activity: { kind: 'idle', label: 'Idle' },
    usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
    messageCount: 2,
    agents: { running: 0, total: 0, workflowsRunning: 0, backgroundRunning: 0 },
    queued: 0,
    pending: [],
    control: ownedCaps(),
    ...over,
  }) as SessionDetail;

const servers = (figma: string) => ({
  servers: [
    { name: 'gh', status: 'connected', tools: 12 },
    {
      name: 'figma-mcp-node',
      status: figma,
      error: figma === 'failed' ? 'Connection closed: the Figma plugin is not running' : null,
    },
  ],
});

describe('SessionPluginsComponent', () => {
  let backend: FakeBackend;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([]), SessionContext]);
    (
      TestBed.inject(RealtimeService) as unknown as { connectionS: { set(v: string): void } }
    ).connectionS.set('limited');
  });
  afterEach(() => setActiveMock(null));

  const render = async (s: SessionDetail) => {
    const ctx = TestBed.inject(SessionContext);
    ctx.id.set(s.id);
    ctx.detail.set(s);
    const fixture = TestBed.createComponent(SessionPluginsComponent);
    fixture.detectChanges();
    for (let i = 0; i < 3; i++) {
      await flush();
      fixture.detectChanges();
    }
    return { fixture, el: fixture.nativeElement as HTMLElement };
  };
  const words = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const button = (el: ParentNode, text: string) =>
    [...el.querySelectorAll('button')].find((b) => words(b).startsWith(text));

  it('reconnects a server that failed at start, and turns one off for this session', async () => {
    let figma = 'failed';
    backend.routes.set('/sessions/s1/mcp', () => ({ status: 200, body: servers(figma) }));
    backend.routes.set('/sessions/s1/mcp/reconnect', () => {
      figma = 'connected';
      return { status: 200, body: servers(figma) };
    });
    backend.routes.set('/sessions/s1/mcp/enabled', () => ({
      status: 200,
      body: {
        servers: [
          { name: 'gh', status: 'disabled' },
          { name: 'figma-mcp-node', status: 'connected' },
        ],
      },
    }));
    backend.routes.set('/sessions/s1/plugins', () => ({
      status: 200,
      body: { plugins: [], errors: [], known: true, reload: 'control' },
    }));
    const { fixture, el } = await render(session());
    const row = el.querySelector('[data-server="figma-mcp-node"]')!;
    expect(words(row)).toContain('Connection closed: the Figma plugin is not running');
    expect(words(row)).toContain('Failed');
    expect(words(el)).toContain('connects with Reconnect, without restarting the session');
    expect(words(el.querySelector('[data-server="gh"]'))).toContain('12 tools');

    const success = vi.spyOn(TestBed.inject(NoticeService), 'success');
    button(row, 'Reconnect')!.click();
    await flush();
    fixture.detectChanges();
    expect(backend.bodyOf('POST /sessions/s1/mcp/reconnect')).toEqual({ server: 'figma-mcp-node' });
    expect(words(el.querySelector('[data-server="figma-mcp-node"]'))).toContain('Connected');
    expect(success).toHaveBeenCalledWith('figma-mcp-node is connected.');
    expect(words(el)).not.toContain('connects with Reconnect');

    button(el.querySelector('[data-server="gh"]')!, 'Turn off')!.click();
    await flush();
    fixture.detectChanges();
    expect(backend.bodyOf('POST /sessions/s1/mcp/enabled')).toEqual({
      server: 'gh',
      enabled: false,
    });
    expect(words(el.querySelector('[data-server="gh"]'))).toContain('Off in this session');
    expect(button(el.querySelector('[data-server="gh"]')!, 'Turn on')).toBeTruthy();
  });

  it('reloads its plugins, and asks before a reload that would re-read the conversation', async () => {
    backend.routes.set('/sessions/s1/mcp', () => ({ status: 200, body: servers('connected') }));
    backend.routes.set('/sessions/s1/plugins', () => ({
      status: 200,
      body: {
        plugins: [{ name: 'figma', source: 'figma@claude-plugins-official', version: '2.2.157' }],
        errors: ['broken@m: manifest is not valid JSON'],
        known: true,
        reload: 'control',
      },
    }));
    let force = false;
    backend.routes.set('/sessions/s1/plugins/reload', (_m, body) => {
      force = !!(body as { force?: boolean }).force;
      return force
        ? {
            status: 200,
            body: {
              held: false,
              plugins: [
                { name: 'figma', source: 'figma@claude-plugins-official' },
                { name: 'tokens', source: 'tokens@team' },
              ],
              servers: [],
              errorCount: 0,
            },
          }
        : {
            status: 200,
            body: {
              held: true,
              cacheImpact: { serversAdded: ['tokens'], serversRemoved: [], lspToolChange: false },
              plugins: [],
              servers: [],
              errorCount: 0,
            },
          };
    });
    const { fixture, el } = await render(session());
    expect(words(el.querySelector('[data-plugin="figma"]'))).toContain(
      'From claude-plugins-official',
    );
    expect(words(el)).toContain('broken@m: manifest is not valid JSON');

    button(el, 'Reload plugins')!.click();
    await flush();
    fixture.detectChanges();
    expect(words(el.querySelector('.held'))).toContain(
      'Reloading adds tokens: Claude then re-reads the whole conversation with its next message, which costs more.',
    );
    button(el.querySelector('.held')!, 'Reload anyway')!.click();
    await flush();
    fixture.detectChanges();
    expect(force).toBe(true);
    expect(el.querySelector('.held')).toBeNull();
    expect(words(el.querySelector('[data-plugin="tokens"]'))).toContain('From team');
  });

  it('adds a plugin folder to this session only, for a desktop', async () => {
    backend.routes.set('/sessions/s1/mcp', () => ({ status: 200, body: { servers: [] } }));
    backend.routes.set('/sessions/s1/plugins', () => ({
      status: 200,
      body: { plugins: [], errors: [], known: false, reload: 'control' },
    }));
    backend.routes.set('/sessions/s1/plugins/add', () => ({
      status: 200,
      body: {
        held: false,
        plugins: [{ name: 'mine', source: 'mine@inline', path: '/p/mine' }],
        servers: [],
      },
    }));
    const ctx = TestBed.inject(SessionContext);
    vi.spyOn(ctx, 'canDangerous').mockReturnValue(true);
    const { fixture, el } = await render(session());
    expect(words(el)).toContain('Its plugins show once its first message runs');
    button(el, 'Add a plugin folder to this session')!.click();
    fixture.detectChanges();
    const input = el.querySelector('form.add input') as HTMLInputElement;
    input.value = '/p/mine';
    input.dispatchEvent(new Event('input'));
    fixture.detectChanges();
    (el.querySelector('form.add') as HTMLFormElement).dispatchEvent(new Event('submit'));
    await flush();
    fixture.detectChanges();
    expect(backend.bodyOf('POST /sessions/s1/plugins/add')).toEqual({ path: '/p/mine' });
    expect(words(el.querySelector('[data-plugin="mine"]'))).toContain('Loaded for this session');
    expect(el.querySelector('form.add')).toBeNull();
  });

  it('says to type /reload-plugins in a session it cannot type into', async () => {
    backend.routes.set('/sessions/s1/plugins', () => ({
      status: 200,
      body: { plugins: [], errors: [], known: false, reload: 'none' },
    }));
    const { el } = await render(
      session({ owned: false, surface: 'vscode', control: { ...ownedCaps(), channel: 'hooks' } }),
    );
    expect(backend.calls).not.toContain('GET /sessions/s1/mcp');
    expect(words(el)).toContain('type /reload-plugins in it');
    expect(words(el)).toContain('Type /mcp in the session itself');
    expect(button(el, 'Reload plugins')).toBeUndefined();
    expect(button(el, 'Copy the command')).toBeTruthy();
  });

  it('says a conversation of the HTTP API loads plugin changes when it starts again', async () => {
    backend.routes.set('/sessions/s1/mcp', () => ({ status: 409, body: {} }));
    backend.routes.set('/sessions/s1/plugins', () => ({
      status: 200,
      body: { plugins: [], errors: [], known: false, reload: 'start' },
    }));
    const { el } = await render(session());
    expect(words(el)).toContain('loads plugin changes when it starts again');
    expect(button(el, 'Reload plugins')).toBeUndefined();
    expect(words(el)).not.toContain('type /reload-plugins');
  });

  it('types /reload-plugins into a tmux session', async () => {
    backend.routes.set('/sessions/s1/plugins', () => ({
      status: 200,
      body: { plugins: [], errors: [], known: false, reload: 'tmux' },
    }));
    backend.routes.set('/sessions/s1/plugins/reload', () => ({
      status: 200,
      body: { typed: true },
    }));
    const success = vi.spyOn(TestBed.inject(NoticeService), 'success');
    const { fixture, el } = await render(
      session({ owned: false, surface: 'tmux', control: { ...ownedCaps(), channel: 'tmux' } }),
    );
    button(el, 'Reload plugins')!.click();
    await flush();
    fixture.detectChanges();
    expect(success).toHaveBeenCalledWith('/reload-plugins typed into the session.');
  });
});
