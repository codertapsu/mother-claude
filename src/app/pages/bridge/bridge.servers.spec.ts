import { ComponentFixture, TestBed } from '@angular/core/testing';
import { ActivatedRoute, convertToParamMap, provideRouter } from '@angular/router';

import {
  FakeBackend,
  asDesktop,
  flush,
  serveAccounts,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import { MOCK_ACCOUNT_PRO, MOCK_ACCOUNT_WORK, buildAccounts } from '../../core/mock/fixtures';
import { setActiveMock } from '../../core/mock/mock-api';
import { BridgeStatus, BridgeThread } from '../../core/models';
import { BridgeComponent } from './bridge.component';

const thread = (id: string, over: Partial<BridgeThread> = {}): BridgeThread => ({
  threadId: id,
  cwd: '/Users/me/dev/api',
  createdAt: 1,
  permissionMode: 'default',
  title: `Conversation ${id}`,
  persisted: true,
  ...over,
});

/** The main payload, as before servers. */
const main = (over: Partial<BridgeStatus> = {}): BridgeStatus => ({
  enabled: true,
  running: true,
  port: 5612,
  url: 'http://127.0.0.1:5612',
  startedAt: Date.now() - 60_000,
  requireToken: false,
  tokenPolicy: 'local',
  configuredTokenPolicy: 'local',
  exposedByTunnel: false,
  hostBuilt: true,
  hostRunning: true,
  messagesApi: true,
  claudeAuthenticated: true,
  claudeAccount: 'me@example.com (Max)',
  activeOperations: 0,
  defaults: { cwd: '/Users/me/dev' },
  threads: [thread('m1')],
  docsUrl: 'http://127.0.0.1:5612/docs/',
  consoleUrl: 'http://127.0.0.1:5612/example/',
  ...over,
});

const work = (over: Record<string, unknown> = {}) => ({
  id: 'work',
  name: 'Work',
  running: true,
  port: 5613,
  url: 'http://127.0.0.1:5613',
  prefix: '/servers/work',
  startedAt: Date.now() - 60_000,
  requireToken: false,
  tokenPolicy: 'local',
  exposedByTunnel: true,
  tunnelUrl: 'https://acme.ngrok.app',
  defaults: { cwd: '/Users/me/acme', account: MOCK_ACCOUNT_WORK, accountOnly: true },
  activeThread: 'w1',
  threads: [thread('w1', { title: 'Triage', cwd: '/Users/me/acme' })],
  docsUrl: 'http://127.0.0.1:5613/docs/',
  consoleUrl: 'http://127.0.0.1:5613/example/',
  ...over,
});

const personal = (over: Record<string, unknown> = {}) => ({
  id: 'personal',
  name: 'Personal',
  running: false,
  port: 5614,
  url: null,
  prefix: '/servers/personal',
  requireToken: false,
  tokenPolicy: 'local',
  exposedByTunnel: false,
  defaults: { account: MOCK_ACCOUNT_PRO, accountOnly: true },
  threads: [],
  ...over,
});

const withServers = (servers: unknown[], over: Partial<BridgeStatus> = {}) =>
  ({ ...main(over), servers: [{ id: 'main', name: 'Main' }, ...servers] }) as BridgeStatus;

const text = (n: Element | null | undefined) => (n?.textContent ?? '').replace(/\s+/g, ' ').trim();

/** An element's text nodes joined by a space, as they read on screen (the
 * template keeps no whitespace between elements). */
const words = (n: Element | null | undefined): string => {
  if (!n) return '';
  const out: string[] = [];
  const walk = document.createTreeWalker(n, NodeFilter.SHOW_TEXT);
  for (let t = walk.nextNode(); t; t = walk.nextNode()) {
    const s = (t.textContent ?? '').replace(/\s+/g, ' ').trim();
    if (s) out.push(s);
  }
  return out.join(' ');
};

describe('BridgeComponent servers', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<BridgeComponent>;
  let el: HTMLElement;

  const setup = (query: Record<string, string> = {}) => {
    backend = new FakeBackend();
    useFakeServer(backend, [
      provideRouter([]),
      {
        provide: ActivatedRoute,
        useValue: { snapshot: { queryParamMap: convertToParamMap(query) } },
      },
    ]);
    backend.routes.set('/defaults', () => ({ status: 200, body: { models: [], efforts: [] } }));
    backend.routes.set('/sessions', () => ({ status: 200, body: { sessions: [] } }));
    serveAccounts(backend, buildAccounts(Date.now(), true));
  };

  afterEach(() => setActiveMock(null));

  const render = async (s: BridgeStatus) => {
    backend.routes.set('/bridge', () => ({ status: 200, body: s }));
    fixture = TestBed.createComponent(BridgeComponent);
    fixture.detectChanges();
    await settle();
    el = fixture.nativeElement as HTMLElement;
    return el;
  };
  const settle = async () => {
    await flush();
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
  };
  const rows = () => [...el.querySelectorAll('mc-api-server-list .srv')].map(words);
  const button = (label: string, root: ParentNode = el) =>
    [...root.querySelectorAll<HTMLButtonElement>('button')].find((b) => text(b).includes(label))!;
  const click = async (label: string, root?: ParentNode) => {
    button(label, root).click();
    await settle();
  };
  const head = () => words(el.querySelector('.status .head'));
  const addresses = () =>
    [...el.querySelectorAll('mc-api-server-addresses .addr')].map((a) => ({
      label: text(a.querySelector('.alabel')),
      url: text(a.querySelector('.aurl')),
    }));

  it('with no servers field looks as it did, plus a quiet Add API server', async () => {
    setup();
    await render(main());
    expect(el.querySelector('mc-api-server-list')).toBeNull();
    expect(el.querySelector('mc-api-server-addresses')).toBeNull();
    expect(head()).toBe('running http://127.0.0.1:5612 Stop');
    expect(text(el.querySelector('.more'))).toContain('Add API server');
    // No "Only this account" for the main server while it is the only one.
    expect(el.textContent).not.toContain('Only this account');
    // The main server's routes, with the request as before.
    backend.routes.set('/bridge/start', () => ({ status: 200, body: main() }));
    await click('Restart with these settings');
    expect(backend.bodyOf('POST /bridge/start')).toEqual({
      defaults: { cwd: '/Users/me/dev', permissionMode: 'default' },
    });
    expect(backend.calls.some((c) => c.includes('/bridge/servers'))).toBeFalse();
  });

  it('lists the servers, main first, with state, port, account and tunnel', async () => {
    setup();
    await render(
      withServers([
        work(),
        personal({ accountMissing: true, defaults: { account: 'acc_gone', accountOnly: true } }),
      ]),
    );
    expect(rows()).toEqual([
      'Main running port 5612 · automatic account',
      'Work running port 5613 · Account Work · only this account A tunnel points at it: https://acme.ngrok.app',
      'Personal stopped port 5614 · its account was removed · only this account',
    ]);
    // The main server's panel, named, with its own addresses.
    expect(head()).toBe('Main running http://127.0.0.1:5612 Stop');
    expect(addresses().map((a) => a.url)).toEqual([
      "https://<the Mac's address>:6725/v1",
      'http://127.0.0.1:5612/v1',
    ]);
    // Adding is in the list now, not under the buttons.
    expect(el.querySelector('.more')).toBeNull();
    expect(text(el.querySelector('mc-api-server-list'))).toContain('Add API server');
  });

  it("shows a picked server's panel, its addresses and its routes", async () => {
    setup();
    await render(withServers([work()]));
    await click('Work');
    expect(head()).toBe('Work running http://127.0.0.1:5613 Manage Stop');
    expect(text(el.querySelector('.facts'))).toContain(
      'Work — marcus@acme.dev · Team · only this account',
    );
    // A phone leads with what it can reach (off the network here: the
    // tunnel); the Mac's loopback says so.
    expect(addresses()).toEqual([
      { label: 'Through its tunnel', url: 'https://acme.ngrok.app/v1' },
      { label: 'On your network', url: "https://<the Mac's address>:6725/servers/work/v1" },
      { label: 'On the Mac', url: 'http://127.0.0.1:5613/v1' },
      { label: 'On the Mac, through the main port', url: 'http://127.0.0.1:5612/servers/work/v1' },
    ]);
    // Its own conversation, under its account.
    const running = [...el.querySelectorAll('optgroup[label="Running now"] option')].map(text);
    expect(running).toEqual(['Triage — /Users/me/acme · Default']);
    // Restart: its route, with its account and "only this account".
    backend.routes.set('/bridge/servers/work/start', () => ({
      status: 200,
      body: work({ startedAt: Date.now() }),
    }));
    await click('Restart with these settings');
    const body = backend.bodyOf('POST /bridge/servers/work/start') as {
      defaults: Record<string, unknown>;
    };
    expect(body.defaults).toEqual(
      jasmine.objectContaining({
        account: MOCK_ACCOUNT_WORK,
        accountOnly: true,
        cwd: '/Users/me/acme',
      }),
    );
    // Stop: its route; the main server's are left alone.
    backend.routes.set('/bridge/servers/work/stop', () => ({
      status: 200,
      body: work({ running: false, startedAt: null }),
    }));
    await click('Stop');
    expect(head()).toBe('Work stopped port 5613 Manage');
    expect(rows()[1]).toContain('Work stopped');
    expect(backend.calls).not.toContain('POST /bridge/stop');
    expect(backend.calls).not.toContain('POST /bridge/start');
  });

  it('opens on the server ?server= names, and the desktop leads with this Mac', async () => {
    setup({ server: 'work' });
    asDesktop();
    backend.routes.set('/remote', () => ({
      status: 200,
      body: {
        lan: {
          enabled: true,
          port: 6725,
          addresses: [{ ip: '192.168.1.23', url: 'https://192.168.1.23:6725' }],
        },
      },
    }));
    await render(withServers([work()]));
    await settle();
    expect(head()).toContain('Work running');
    expect(addresses().map((a) => a.label)).toEqual([
      'On this Mac',
      'On this Mac, through the main port',
      'On your network',
      'Through its tunnel',
    ]);
    expect(addresses()[2].url).toBe('https://192.168.1.23:6725/servers/work/v1');
  });

  it("keeps a removed account's server from starting until another account is chosen", async () => {
    setup({ server: 'personal' });
    await render(
      withServers([
        personal({ accountMissing: true, defaults: { account: 'acc_gone', accountOnly: true } }),
      ]),
    );
    expect(text(el.querySelector('.status'))).toContain(
      'Its account was removed, so it cannot start',
    );
    const start = button('Start Personal');
    expect(start.disabled).toBeTrue();
    const picker = [...el.querySelectorAll<HTMLSelectElement>('select')].find((s) =>
      [...s.options].some((o) => text(o) === 'A removed account'),
    )!;
    picker.value = '';
    picker.dispatchEvent(new Event('change'));
    await settle();
    expect(button('Start Personal').disabled).toBeFalse();
  });

  it("saves a stopped server's defaults without starting it", async () => {
    setup({ server: 'personal' });
    await render(withServers([personal()]));
    backend.routes.set('/bridge/servers/personal', (method) => ({
      status: method === 'PATCH' ? 200 : 405,
      body: personal({ defaults: { account: MOCK_ACCOUNT_PRO, accountOnly: true, cwd: '/tmp' } }),
    }));
    await click('Save without starting');
    expect(backend.bodyOf('PATCH /bridge/servers/personal')).toEqual(
      jasmine.objectContaining({
        defaults: jasmine.objectContaining({ account: MOCK_ACCOUNT_PRO, accountOnly: true }),
      }),
    );
    expect(text(el.querySelector('.said'))).toBe('Saved: Personal starts with these settings.');
  });

  it('picking an account for a server turns "Only this account" on', async () => {
    setup({ server: 'personal' });
    await render(withServers([personal({ defaults: {} })]));
    expect(el.textContent).not.toContain('Only this account');
    const picker = [...el.querySelectorAll<HTMLSelectElement>('select')].find((s) =>
      [...s.options].some((o) => text(o).startsWith('Automatic')),
    )!;
    picker.value = MOCK_ACCOUNT_WORK;
    picker.dispatchEvent(new Event('change'));
    await settle();
    const only = [...el.querySelectorAll<HTMLLabelElement>('label.check')].find((l) =>
      text(l).startsWith('Only this account'),
    )!;
    expect(only.querySelector('input')!.checked).toBeTrue();
  });

  it('tells a phone that restarting a server open to tunnels closes it', async () => {
    setup({ server: 'work' });
    await render(withServers([work({ tokenPolicy: 'never' })]));
    const access = [...el.querySelectorAll('.hint')].map(text).find((h) => h.startsWith('On:'))!;
    expect(access).toContain('Only the desktop app can change this.');
    expect(access).toContain('Restart with these settings from here closes it to tunnels.');
    // The main server keeps its configured opening: nothing to warn about.
    await click('Main');
    expect(el.textContent).not.toContain('closes it to tunnels');
  });

  it('puts a refusal in words', async () => {
    setup();
    await render(withServers([personal({ defaults: {} })]));
    await click('Personal');
    backend.routes.set('/bridge/servers/personal/start', () => ({
      status: 409,
      body: { error: { code: 'port_unavailable', message: 'Could not bind', port: 5614 } },
    }));
    await click('Start Personal');
    expect(text(el.querySelector('.card.err'))).toBe(
      'Port 5614 is in use by another program, so Personal cannot listen on it. Stop that program, or give Personal another port.',
    );
  });

  it('adds a server and shows it, and removing it brings the screen back as it was', async () => {
    setup();
    await render(main());
    await click('Add API server');
    const dialog = el.querySelector('dialog')!;
    const name = dialog.querySelector<HTMLInputElement>('input[name=name]')!;
    name.value = 'Work';
    name.dispatchEvent(new Event('input'));
    const account = dialog.querySelector<HTMLSelectElement>('select[name=account]')!;
    account.value = MOCK_ACCOUNT_WORK;
    account.dispatchEvent(new Event('change'));
    await settle();
    backend.routes.set('/bridge/servers', () => ({
      status: 201,
      body: work({ running: false, exposedByTunnel: false, tunnelUrl: null, threads: [] }),
    }));
    await click('Add API server', dialog);
    expect(backend.bodyOf('POST /bridge/servers')).toEqual({
      name: 'Work',
      defaults: {
        cwd: '/Users/me/dev',
        permissionMode: 'default',
        account: MOCK_ACCOUNT_WORK,
        accountOnly: true,
      },
    });
    expect(rows().map((r) => r.split(' ')[0])).toEqual(['Main', 'Work']);
    expect(head()).toBe('Work stopped port 5613 Manage');
    expect(text(el.querySelector('.said-top'))).toContain('Added Work.');

    await click('Manage');
    // (The server answers 204; this fake can only send a body.)
    backend.routes.set('/bridge/servers/work', () => ({ status: 200, body: {} }));
    await click('Remove Work…', el.querySelector('dialog')!);
    await click('Remove', el.querySelector('dialog')!);
    expect(backend.calls).toContain('DELETE /bridge/servers/work');
    expect(el.querySelector('mc-api-server-list')).toBeNull();
    expect(head()).toBe('running http://127.0.0.1:5612 Stop');
    expect(text(el.querySelector('.said-top'))).toBe('Removed Work.');
  });
});
