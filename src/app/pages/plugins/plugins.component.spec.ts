import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { FakeBackend, asDesktop, flush, useFakeServer } from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { NoticeService } from '../../core/notice.service';
import { PluginsComponent, reloadAllText } from './plugins.component';

const installed = (over: Record<string, unknown> = {}) => ({
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
  ...over,
});

const view = (list: unknown[]) => ({
  installed: list,
  marketplaces: [
    {
      name: 'claude-plugins-official',
      source: 'github',
      location: 'anthropics/claude-plugins-official',
      ours: false,
    },
    { name: 'mother-claude-local', source: 'directory', location: '/x', ours: true },
  ],
  project: null,
});

const catalog = {
  plugins: [
    {
      id: 'figma@claude-plugins-official',
      name: 'figma',
      description: 'Design files',
      marketplace: 'claude-plugins-official',
      installCount: 900,
    },
    {
      id: 'postman@claude-plugins-official',
      name: 'postman',
      description: 'API testing',
      marketplace: 'claude-plugins-official',
      installCount: 400,
    },
  ],
  total: 120,
  catalog: 4045,
};

describe('PluginsComponent', () => {
  let backend: FakeBackend;
  let list: unknown[];

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    list = [
      installed(),
      installed({
        id: 'mother-claude@mother-claude-local',
        name: 'mother-claude',
        marketplace: 'mother-claude-local',
        ours: true,
        mcpServers: [],
      }),
      installed({
        id: 'engineering@synced',
        name: 'engineering',
        marketplace: 'synced',
        scope: 'synced',
        changeable: false,
        mcpServers: [],
        notes: ['Synced from claude.ai'],
      }),
    ];
    backend.routes.set('/plugins', () => ({ status: 200, body: view(list) }));
    backend.routes.set('/plugins/available', () => ({ status: 200, body: catalog }));
    backend.routes.set('/projects', () => ({
      status: 200,
      body: { projects: [{ path: '/Users/me/dev/site', name: 'site' }] },
    }));
  });
  afterEach(() => setActiveMock(null));

  const render = async () => {
    const fixture = TestBed.createComponent(PluginsComponent);
    fixture.detectChanges();
    for (let i = 0; i < 4; i++) {
      await flush();
      fixture.detectChanges();
    }
    return { fixture, el: fixture.nativeElement as HTMLElement };
  };
  const words = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const button = (el: ParentNode | null, text: string) =>
    [...(el?.querySelectorAll('button') ?? [])].find((b) => words(b).startsWith(text));
  const settle = async (fixture: { detectChanges(): void }) => {
    for (let i = 0; i < 4; i++) {
      await flush();
      fixture.detectChanges();
    }
  };

  it('shows what is installed and where, read only on a phone', async () => {
    const { el } = await render();
    const figma = el.querySelector('[data-id="figma@claude-plugins-official"]');
    expect(words(figma)).toContain('Everywhere');
    expect(words(figma)).toContain('MCP servers: figma (mcp.figma.com)');
    expect(words(el.querySelector('[data-id="mother-claude@mother-claude-local"]'))).toContain(
      'Sessions started elsewhere',
    );
    const synced = el.querySelector('[data-id="engineering@synced"]');
    expect(words(synced)).toContain('From claude.ai');
    expect(words(synced)).toContain('Changed where it comes from, not here.');
    // A phone changes nothing here.
    expect(words(el)).toContain(
      'plugins are installed and changed in Mother Claude on the computer',
    );
    expect(button(figma, 'Turn off')).toBeUndefined();
    expect(button(el, 'Install…')).toBeUndefined();
    expect(el.querySelector('form.add')).toBeNull();
    // The catalog: most installed first, and how many more there are.
    expect(words(el.querySelector('[data-id="postman@claude-plugins-official"]'))).toContain(
      '400 installs',
    );
    expect(words(el)).toContain('Showing 2 of 120: search to narrow it down.');
  });

  it('installs one for a project only for you, after showing the command its marketplace runs', async () => {
    asDesktop();
    let accepted = '';
    backend.routes.set('/plugins/install', (_m, body) => {
      const b = body as { acceptCommand?: string };
      if (!b.acceptCommand) {
        return {
          status: 409,
          body: {
            error: {
              code: 'confirm_command',
              message: 'The marketplace runs a command to install this plugin.',
              command: 'npx postman-installer',
              sha256: 'c'.repeat(64),
            },
          },
        };
      }
      accepted = b.acceptCommand;
      list = [
        ...list,
        installed({ id: 'postman@claude-plugins-official', name: 'postman', scope: 'local' }),
      ];
      return { status: 200, body: { ok: true } };
    });
    const { fixture, el } = await render();
    // Installed already: no second install offered.
    expect(
      button(el.querySelector('[data-id="figma@claude-plugins-official"].offer'), 'Install…'),
    ).toBeUndefined();
    button(
      el.querySelector('[data-id="postman@claude-plugins-official"].offer'),
      'Install…',
    )!.click();
    await settle(fixture);
    const sheet = () => document.querySelector('dialog[open]')!;
    expect(words(sheet())).toContain('Install postman');
    (sheet().querySelector('input[value="local"]') as HTMLInputElement).click();
    fixture.detectChanges();
    const select = sheet().querySelector('select') as HTMLSelectElement;
    select.value = '/Users/me/dev/site';
    select.dispatchEvent(new Event('change'));
    fixture.detectChanges();
    button(sheet(), 'Install')!.click();
    await settle(fixture);
    expect(backend.bodyOf('POST /plugins/install')).toEqual({
      plugin: 'postman@claude-plugins-official',
      scope: 'local',
      project: '/Users/me/dev/site',
    });
    expect(words(sheet())).toContain('npx postman-installer');
    button(sheet(), 'Run it and install')!.click();
    await settle(fixture);
    expect(accepted).toBe('c'.repeat(64));
    expect(words(el.querySelector('[data-id="postman@claude-plugins-official"].plugin'))).toContain(
      'This project, only you',
    );
    // Running sessions can load it now.
    backend.routes.set('/plugins/reload', () => ({
      status: 200,
      body: { reloaded: 2, held: 0, busy: 1, failed: 0 },
    }));
    const success = vi.spyOn(TestBed.inject(NoticeService), 'success');
    button(el.querySelector('.changed'), 'Reload in running sessions')!.click();
    await settle(fixture);
    expect(success).toHaveBeenCalledWith('2 reloaded · 1 working (reload them once they finish)');
  });

  it('turns one off and removes another, and adds and removes a marketplace', async () => {
    asDesktop();
    backend.routes.set('/plugins/enabled', (_m, body) => {
      list = list.map((p) =>
        (p as { id: string }).id === 'figma@claude-plugins-official'
          ? { ...(p as object), enabled: (body as { enabled: boolean }).enabled }
          : p,
      );
      return { status: 200, body: { ok: true } };
    });
    backend.routes.set('/plugins/uninstall', () => {
      list = list.filter((p) => (p as { id: string }).id !== 'figma@claude-plugins-official');
      return { status: 200, body: { ok: true } };
    });
    backend.routes.set('/plugins/marketplaces', () => ({ status: 200, body: { ok: true } }));
    backend.routes.set('/plugins/marketplaces/claude-plugins-official', () => ({
      status: 200,
      body: { ok: true },
    }));
    const { fixture, el } = await render();
    const figma = () => el.querySelector('[data-id="figma@claude-plugins-official"].plugin');
    button(figma(), 'Turn off')!.click();
    await settle(fixture);
    expect(backend.bodyOf('POST /plugins/enabled')).toEqual({
      plugin: 'figma@claude-plugins-official',
      enabled: false,
      scope: 'user',
    });
    expect(words(figma())).toContain('Off');
    // Mother Claude's own and a synced one offer nothing.
    expect(
      button(el.querySelector('[data-id="mother-claude@mother-claude-local"]'), 'Remove'),
    ).toBeUndefined();
    expect(button(el.querySelector('[data-id="engineering@synced"]'), 'Remove')).toBeUndefined();

    button(figma(), 'Remove…')!.click();
    fixture.detectChanges();
    button(figma(), 'Remove')!.click();
    await settle(fixture);
    expect(backend.bodyOf('POST /plugins/uninstall')).toEqual({
      plugin: 'figma@claude-plugins-official',
      scope: 'user',
    });
    expect(figma()).toBeNull();

    const input = el.querySelector('form.add input') as HTMLInputElement;
    input.value = 'acme/team-plugins';
    input.dispatchEvent(new Event('input'));
    fixture.detectChanges();
    (el.querySelector('form.add') as HTMLFormElement).dispatchEvent(new Event('submit'));
    await settle(fixture);
    expect(backend.bodyOf('POST /plugins/marketplaces')).toEqual({ source: 'acme/team-plugins' });
    expect((el.querySelector('form.add input') as HTMLInputElement).value).toBe('');
    // Its own marketplace can't be removed from here.
    expect(button(el.querySelector('[data-name="mother-claude-local"]'), 'Remove')).toBeUndefined();
    button(el.querySelector('[data-name="claude-plugins-official"]'), 'Remove…')!.click();
    fixture.detectChanges();
    button(el.querySelector('[data-name="claude-plugins-official"]'), 'Remove')!.click();
    await settle(fixture);
    expect(backend.calls).toContain('DELETE /plugins/marketplaces/claude-plugins-official');
  });

  it("shows a project's own plugin with its project, and switches it in that project", async () => {
    asDesktop();
    const site = '/Users/me/dev/site';
    const local = (known: boolean) =>
      installed({
        id: 'tokens@team',
        name: 'tokens',
        marketplace: 'team',
        scope: 'local',
        project: site,
        enabled: known,
        enabledKnown: known,
        mcpServers: [],
      });
    backend.routes.set('/plugins', (_m, _b, url) => ({
      status: 200,
      body: view([installed(), local(url?.searchParams.get('project') === site)]),
    }));
    backend.routes.set('/plugins/enabled', () => ({ status: 200, body: { ok: true } }));
    const { fixture, el } = await render();
    const tokens = () => el.querySelector('[data-id="tokens@team"].plugin');
    expect(words(tokens())).toContain('This project, only you');
    expect(words(tokens())).toContain('in site');
    // Whether it is on is known only in its project.
    expect(words(tokens())).not.toContain('Off');
    expect(button(tokens(), 'Turn off')).toBeUndefined();
    button(tokens(), 'Show site')!.click();
    await settle(fixture);
    expect(backend.urls).toContain(`/plugins?project=${encodeURIComponent(site)}`);
    button(tokens(), 'Turn off')!.click();
    await settle(fixture);
    expect(backend.bodyOf('POST /plugins/enabled')).toEqual({
      plugin: 'tokens@team',
      enabled: false,
      scope: 'local',
      project: site,
    });
  });

  it('says what a reload of every session did', () => {
    expect(reloadAllText({ reloaded: 0, held: 0, busy: 0, failed: 0 })).toContain(
      'No session Mother Claude runs needed it',
    );
    expect(reloadAllText({ reloaded: 1, held: 1, busy: 0, failed: 1 })).toBe(
      '1 reloaded · 1 left as they are: their next message would re-read the whole conversation · 1 could not reload',
    );
  });
});
