import { ComponentFixture, TestBed } from '@angular/core/testing';

import { ServerStatus, serversOf } from '../../core/api-servers';
import {
  FakeBackend,
  flush,
  serveAccounts,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import { MOCK_ACCOUNT_WORK, buildAccounts } from '../../core/mock/fixtures';
import { setActiveMock } from '../../core/mock/mock-api';
import { BridgeStatus } from '../../core/models';
import { ApiServerSheetComponent } from './server-sheet.component';

const status = {
  enabled: true,
  running: true,
  port: 5612,
  requireToken: false,
  tokenPolicy: 'local',
  configuredTokenPolicy: 'local',
  exposedByTunnel: false,
  hostBuilt: true,
  hostRunning: true,
  messagesApi: true,
  claudeAuthenticated: true,
  claudeAccount: 'me',
  activeOperations: 0,
  defaults: {},
  threads: [],
  servers: [
    { id: 'main', name: 'Main' },
    { id: 'work', name: 'Work', port: 5613, running: true, defaults: {} },
  ],
} as unknown as BridgeStatus;

const text = (n: Element | null | undefined) => (n?.textContent ?? '').replace(/\s+/g, ' ').trim();

describe('ApiServerSheetComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<ApiServerSheetComponent>;
  let saved: unknown[];
  let removed: string[];
  const servers = serversOf(status);

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend);
    serveAccounts(backend, buildAccounts(Date.now(), true));
    saved = [];
    removed = [];
  });

  afterEach(() => setActiveMock(null));

  const open = async (inputs: Partial<Record<string, unknown>> = {}) => {
    fixture = TestBed.createComponent(ApiServerSheetComponent);
    fixture.componentRef.setInput('servers', servers);
    fixture.componentRef.setInput('mainPort', 5612);
    for (const [k, v] of Object.entries(inputs)) fixture.componentRef.setInput(k, v);
    fixture.componentInstance.saved.subscribe((s) => saved.push(s));
    fixture.componentInstance.removed.subscribe((id) => removed.push(id));
    fixture.detectChanges();
    fixture.componentRef.setInput('open', true);
    await settle();
    return fixture.nativeElement as HTMLElement;
  };
  const settle = async () => {
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
  };
  const field = <T extends HTMLElement>(el: HTMLElement, name: string) =>
    el.querySelector<T>(`[name=${name}]`)!;
  const type = async (el: HTMLElement, name: string, value: string) => {
    const input = field<HTMLInputElement>(el, name);
    input.value = value;
    input.dispatchEvent(new Event('input'));
    await settle();
  };
  const choose = async (el: HTMLElement, name: string, value: string) => {
    const select = field<HTMLSelectElement>(el, name);
    select.value = value;
    select.dispatchEvent(new Event('change'));
    await settle();
  };
  const submit = (el: HTMLElement) => el.querySelector<HTMLButtonElement>('button[type=submit]')!;

  it("starts from the main server's defaults, without its conversation", async () => {
    const el = await open({
      mainDefaults: {
        cwd: '/Users/me/dev',
        model: 'opus',
        permissionMode: 'bypassPermissions',
        settingSources: [],
        defaultThread: 'main-thread',
        systemPromptAppend: 'Be brief.',
      },
    });
    expect(field<HTMLInputElement>(el, 'cwd').value).toBe('/Users/me/dev');
    // A mode this form doesn't offer becomes "Ask me".
    expect(field<HTMLSelectElement>(el, 'permissionMode').value).toBe('default');
    expect(field<HTMLInputElement>(el, 'isolate').checked).toBeTrue();
    await type(el, 'name', 'Lab');
    await type(el, 'port', '5620');
    backend.routes.set('/bridge/servers', () => ({
      status: 201,
      body: { id: 'lab', name: 'Lab', port: 5620 },
    }));
    submit(el).click();
    await settle();
    expect(backend.bodyOf('POST /bridge/servers')).toEqual({
      name: 'Lab',
      port: 5620,
      defaults: {
        cwd: '/Users/me/dev',
        model: 'opus',
        permissionMode: 'default',
        settingSources: [],
        systemPromptAppend: 'Be brief.',
      },
    });
    expect(saved).toEqual([{ id: 'lab', name: 'Lab', port: 5620 }]);
  });

  it('turns "Only this account" on with an account, and off with Automatic', async () => {
    const el = await open();
    expect(field(el, 'only')).toBeNull();
    await choose(el, 'account', MOCK_ACCOUNT_WORK);
    expect(field<HTMLInputElement>(el, 'only').checked).toBeTrue();
    expect(text(el.querySelector('label.check'))).toContain('every conversation runs as Work');
    await choose(el, 'account', '');
    expect(field(el, 'only')).toBeNull();
  });

  it('checks the name and the port before asking the server', async () => {
    const el = await open();
    expect(submit(el).disabled).toBeTrue();
    await type(el, 'name', 'work');
    expect(text(el)).toContain('Another API server is called that.');
    expect(submit(el).disabled).toBeTrue();
    await type(el, 'name', 'Lab');
    await type(el, 'port', '5613');
    expect(text(el)).toContain('Work uses that port.');
    await type(el, 'port', '80');
    expect(text(el)).toContain('Enter a port from 1024 to 65535, or leave it empty.');
    await type(el, 'port', '');
    expect(text(el)).toContain('Empty: the first free port from 5613.');
    expect(submit(el).disabled).toBeFalse();
  });

  it('says whose a refused port is', async () => {
    const el = await open();
    await type(el, 'name', 'Lab');
    await type(el, 'port', '6000');
    backend.routes.set('/bridge/servers', () => ({
      status: 409,
      body: { error: { code: 'port_unavailable', message: 'taken', port: 6000, server: 'work' } },
    }));
    submit(el).click();
    await settle();
    expect(text(el.querySelector('[role=alert]'))).toBe(
      "Port 6000 is Work's. Pick another, or leave the port empty for the first free one.",
    );
    expect(saved).toEqual([]);
  });

  describe('for a server', () => {
    const work = servers.find((s) => s.id === 'work') as ServerStatus;

    it('changes only what changed', async () => {
      const el = await open({ server: work });
      expect(field<HTMLInputElement>(el, 'name').value).toBe('Work');
      expect(field<HTMLInputElement>(el, 'port').value).toBe('5613');
      // No account or defaults here: those are on its panel.
      expect(field(el, 'account')).toBeNull();
      expect(field(el, 'cwd')).toBeNull();
      expect(submit(el).disabled).toBeTrue();
      await type(el, 'name', 'Acme');
      backend.routes.set('/bridge/servers/work', () => ({
        status: 200,
        body: { id: 'work', name: 'Acme', port: 5613 },
      }));
      submit(el).click();
      await settle();
      expect(backend.bodyOf('PATCH /bridge/servers/work')).toEqual({ name: 'Acme' });
      expect(saved.length).toBe(1);
    });

    it('removes it after asking, and takes "already gone" for done', async () => {
      const el = await open({ server: work });
      const remove = () =>
        [...el.querySelectorAll<HTMLButtonElement>('button')].find((b) =>
          text(b).startsWith('Remove'),
        )!;
      remove().click();
      await settle();
      expect(text(el.querySelector('.notice.warn'))).toContain('Remove Work? It is stopped first.');
      backend.routes.set('/bridge/servers/work', () => ({
        status: 404,
        body: { error: { code: 'server_not_found', message: 'gone' } },
      }));
      remove().click();
      await settle();
      expect(backend.calls).toContain('DELETE /bridge/servers/work');
      expect(removed).toEqual(['work']);
    });
  });
});
