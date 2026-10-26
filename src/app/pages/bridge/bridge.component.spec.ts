import { ComponentFixture, TestBed } from '@angular/core/testing';
import { By } from '@angular/platform-browser';
import { provideRouter } from '@angular/router';

import { desktopOs, revealLabel } from '../../core/delete-data';
import {
  FakeBackend,
  asDesktop,
  flush,
  serveAccounts,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import { MOCK_ACCOUNT_WORK, buildAccounts } from '../../core/mock/fixtures';
import { setActiveMock } from '../../core/mock/mock-api';
import { BridgeStatus, BridgeThread } from '../../core/models';
import { BridgeComponent } from './bridge.component';
import { ConversationFilesComponent } from './conversation-files.component';

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

describe('BridgeComponent conversations on disk', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<BridgeComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    backend.routes.set('/defaults', () => ({ status: 200, body: { models: [], efforts: [] } }));
    backend.routes.set('/sessions', () => ({ status: 200, body: { sessions: [] } }));
    serveAccounts(backend, buildAccounts(Date.now(), false));
  });

  afterEach(() => setActiveMock(null));

  const render = async (s: BridgeStatus) => {
    backend.routes.set('/bridge', () => ({ status: 200, body: s }));
    fixture = TestBed.createComponent(BridgeComponent);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    return fixture.nativeElement as HTMLElement;
  };
  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const button = (el: Element, label: string) =>
    [...el.querySelectorAll<HTMLButtonElement>('button')].find((b) => text(b) === label);

  it("shows the picked conversation's transcript; one open in the API is skipped as running", async () => {
    asDesktop();
    backend.routes.set('/sessions/t1', () => ({
      status: 200,
      body: {
        id: 't1',
        files: {
          transcript: '/Users/me/.claude/projects/-Users-me-dev-api/t1.jsonl',
          account: 'default',
        },
      },
    }));
    backend.routes.set('/data/delete', () => ({
      status: 200,
      body: {
        plan: {
          sessions: [],
          skipped: [
            {
              id: 't1',
              code: 'session_live',
              message: 'it is open in the API: close it there first',
            },
          ],
        },
      },
    }));
    const el = await render({ ...status([thread('t1')]), activeThread: 't1' });
    const files = el.querySelector('mc-conversation-files')!;
    expect(text(files)).toContain('~/.claude/projects/-Users-me-dev-api/t1.jsonl');
    expect(button(files, 'Copy path')).toBeDefined();
    expect(button(files, revealLabel(desktopOs(navigator.userAgent)))).toBeDefined();
    button(files, 'Delete…')!.click();
    for (let i = 0; i < 3; i++) {
      fixture.detectChanges();
      await flush();
    }
    fixture.detectChanges();
    expect(backend.bodyOf('POST /data/delete')).toEqual({
      from: 'default',
      items: { sessions: ['t1'] },
      undo: true,
      dryRun: true,
    });
    expect(text(el.querySelector('.skips .why'))).toBe(
      'It is open in the API: close it there first.',
    );
    expect(button(el, 'Nothing to delete')?.disabled).toBeTrue();
  });

  it('drops a deleted conversation from the picker and asks the server again', async () => {
    backend.routes.set('/sessions', () => ({
      status: 200,
      body: {
        sessions: [
          {
            id: 'p1',
            title: 'Fix the login',
            cwd: '/Users/me/dev/api',
            projectName: 'api',
            messageCount: 4,
            lastActivityAt: Date.now() - 60_000,
          },
        ],
      },
    }));
    const el = await render({ ...status([]), activeThread: 'p1' });
    const earlier = () =>
      [...el.querySelectorAll('optgroup[label="Earlier conversations"] option')].map(text);
    expect(earlier().length).toBe(1);
    // A phone copies the path, and does not reveal it.
    expect(button(el, revealLabel(desktopOs(navigator.userAgent)))).toBeUndefined();
    const part = fixture.debugElement.query(By.directive(ConversationFilesComponent))
      .componentInstance as ConversationFilesComponent;
    part.deleted.emit('p1');
    await flush();
    fixture.detectChanges();
    expect(earlier()).toEqual([]);
    expect(el.querySelector('mc-conversation-files')).toBeNull();
    expect(backend.calls.filter((c) => c === 'GET /bridge').length).toBe(2);
  });
});
