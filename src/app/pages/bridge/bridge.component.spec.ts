import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import {
  FakeBackend,
  flush,
  serveAccounts,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import { MOCK_ACCOUNT_WORK, buildAccounts } from '../../core/mock/fixtures';
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

const status = (
  threads: BridgeThread[],
  defaults: BridgeStatus['defaults'] = {},
): BridgeStatus => ({
  enabled: true,
  running: true,
  port: 5612,
  url: 'http://127.0.0.1:5612',
  requireToken: false,
  tokenPolicy: 'local',
  configuredTokenPolicy: 'local',
  exposedByTunnel: false,
  hostBuilt: true,
  hostRunning: true,
  messagesApi: true,
  claudeAuthenticated: true,
  claudeAccount: 'me@example.com',
  activeOperations: 0,
  defaults,
  threads,
});

describe('BridgeComponent accounts', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<BridgeComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    backend.routes.set('/defaults', () => ({ status: 200, body: { models: [], efforts: [] } }));
    backend.routes.set('/sessions', () => ({ status: 200, body: { sessions: [] } }));
  });

  afterEach(() => setActiveMock(null));

  const render = async (s: BridgeStatus) => {
    backend.routes.set('/bridge', () => ({ status: 200, body: s }));
    fixture = TestBed.createComponent(BridgeComponent);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    return fixture.nativeElement as HTMLElement;
  };
  const options = (el: HTMLElement) =>
    [...el.querySelectorAll('optgroup[label="Running now"] option')].map((o) =>
      (o.textContent ?? '').replace(/\s+/g, ' ').trim(),
    );

  it("names each running conversation's account; one that names none is the Default's", async () => {
    serveAccounts(backend, buildAccounts(Date.now(), true));
    const work = {
      id: MOCK_ACCOUNT_WORK,
      name: 'Work',
      color: '#0ea5e9',
      kind: 'directory' as const,
    };
    const el = await render(status([thread('t1', { account: work }), thread('t2')]));
    expect(options(el)).toEqual([
      'Conversation t1 — /Users/me/dev/api · Work',
      'Conversation t2 — /Users/me/dev/api · Default',
    ]);
  });

  it('with only the Default names no account, and an unset start-time account is Automatic', async () => {
    serveAccounts(backend, buildAccounts(Date.now(), false));
    const el = await render(status([thread('t1')]));
    expect(options(el)).toEqual(['Conversation t1 — /Users/me/dev/api']);
    // No account picker with only the Default.
    expect(el.textContent).not.toContain('Automatic — the project');
  });

  it('shows the start-time account, and Automatic when the defaults name none', async () => {
    serveAccounts(backend, buildAccounts(Date.now(), true));
    let el = await render(status([], { account: MOCK_ACCOUNT_WORK }));
    const picker = () =>
      [...el.querySelectorAll<HTMLSelectElement>('select')].find((s) =>
        [...s.options].some((o) => o.textContent?.includes('Automatic')),
      )!;
    await flush();
    fixture.detectChanges();
    expect(picker().value).toBe(MOCK_ACCOUNT_WORK);
    fixture.destroy();
    el = await render(status([]));
    await flush();
    fixture.detectChanges();
    expect(picker().value).toBe('');
  });
});
