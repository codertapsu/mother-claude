import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import {
  FakeBackend,
  asDesktop,
  flush,
  serveAccounts,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import { buildAccounts, buildFixtures } from '../../core/mock/fixtures';
import { setActiveMock } from '../../core/mock/mock-api';
import { ConnectionsComponent } from './connections.component';

describe('Settings › Connections', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<ConnectionsComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    backend.routes.set('/browser', () => ({
      status: 200,
      body: {
        browsers: [
          {
            id: 'b1',
            name: 'Work laptop',
            browser: { name: 'Chrome', version: '131' },
            extensionVersion: '0.4.0',
            origin: 'chrome-extension://x',
            createdAt: 1,
            revoked: false,
            connected: true,
          },
        ],
        sites: [
          { origin: 'https://github.com', mode: 'act' },
          { origin: 'https://example.com', mode: 'read' },
        ],
        settings: {},
        sensitive: [],
        grants: [],
        leases: [],
        stops: [],
        approvals: [],
        pairing: { liveCodes: 0 },
        endpoint: 'ws://127.0.0.1:6725/ws/browser',
      },
    }));
    backend.routes.set('/bridge', () => ({
      status: 200,
      body: { running: true, url: 'http://127.0.0.1:5612', threads: [], defaults: {} },
    }));
    backend.routes.set('/mcp', () => ({
      status: 200,
      body: { enabled: true, clients: 4, tools: [], toolCount: 0, recentCalls: [] },
    }));
  });

  afterEach(() => {
    fixture?.destroy();
    setActiveMock(null);
  });

  const text = (e: Element | null | undefined) =>
    (e?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const render = async () => {
    fixture = TestBed.createComponent(ConnectionsComponent);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    return fixture.nativeElement as HTMLElement;
  };
  const rows = (el: HTMLElement) => [...el.querySelectorAll<HTMLAnchorElement>('a.list-row')];
  const state = (el: HTMLElement, id: string) =>
    text(el.querySelector(`a[data-id="${id}"] .state`));

  it('gives the desktop every connection, each with how it stands, opening its page', async () => {
    asDesktop();
    serveAccounts(backend, buildAccounts(Date.now(), true));
    backend.routes.set('/devices', () => ({
      status: 200,
      body: {
        devices: [
          { id: 'd1', name: 'iPhone' },
          { id: 'd2', name: 'iPad' },
        ],
        pending: [],
      },
    }));
    backend.routes.set('/remote', () => ({ status: 200, body: buildFixtures(Date.now()).remote }));
    backend.routes.set('/control/foreign', () => ({
      status: 200,
      body: { installed: true, cliAvailable: true, sessionsWithHooks: 1, notes: [] },
    }));
    const el = await render();
    expect(rows(el).map((a) => text(a.querySelector('.grow > span')))).toEqual([
      'Phone & devices',
      'Sessions started elsewhere',
      'Browser',
      'HTTP API',
      'Claude Code tools',
    ]);
    expect(rows(el).map((a) => a.getAttribute('href'))).toEqual([
      '/remote',
      '/remote#foreign',
      '/browser',
      '/api',
      '/services',
    ]);
    const internet = buildFixtures(Date.now()).remote.internet.enabled ? 'on' : 'off';
    expect(state(el, 'devices')).toBe(`2 devices paired · internet access ${internet}`);
    // Work keeps its own directory without the plugin: only the Default reports.
    expect(state(el, 'elsewhere')).toBe('VS Code and terminal sessions: on for Default');
    expect(state(el, 'browser')).toBe('Chrome connected · 1 site may act');
    expect(state(el, 'api')).toBe('Running on 127.0.0.1:5612');
    expect(state(el, 'tools')).toBe('On · used by 4 sessions in the last 15 min');
  });

  it("gives a phone the three it may open, and asks nothing of the desktop's own", async () => {
    const el = await render();
    expect(rows(el).map((a) => a.getAttribute('data-id'))).toEqual(['browser', 'api', 'tools']);
    for (const call of ['GET /devices', 'GET /remote', 'GET /control/foreign']) {
      expect(backend.calls).not.toContain(call);
    }
  });

  it('says what a connection is while its state cannot be read', async () => {
    backend.routes.delete('/bridge');
    backend.routes.set('/mcp', () => ({
      status: 200,
      body: { enabled: false, clients: 0, tools: [], toolCount: 0, recentCalls: [] },
    }));
    const el = await render();
    expect(state(el, 'api')).toBe('Drive Claude from scripts and tools');
    expect(state(el, 'tools')).toBe('Off: the browser extension and the team tools need them');
  });
});
