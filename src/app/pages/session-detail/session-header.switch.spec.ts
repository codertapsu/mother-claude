import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import {
  FakeBackend,
  asDesktop,
  flush,
  serveAccounts,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import { MOCK_ACCOUNT_PRO, buildAccounts, ownedCaps } from '../../core/mock/fixtures';
import { setActiveMock } from '../../core/mock/mock-api';
import { SessionDetail } from '../../core/models';
import { SessionContext } from './session-context';
import { SessionHeaderComponent } from './session-header.component';

const detail = (over: Partial<SessionDetail> = {}): SessionDetail =>
  ({
    id: 's1',
    name: 'auth',
    title: 'Fix the login',
    projectName: 'api',
    cwd: '/Users/me/dev/api',
    surface: 'mother-claude',
    owned: true,
    live: true,
    status: 'working',
    activity: { kind: 'tool', label: 'Running Bash' },
    usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
    messageCount: 3,
    agents: { running: 0, total: 0, workflowsRunning: 0, backgroundRunning: 0 },
    queued: 0,
    pending: [],
    control: ownedCaps(),
    account: { id: 'default', name: 'Default', color: '#8b5cf6', kind: 'directory' },
    tasks: [
      { id: 't1', kind: 'bash', label: 'npm test', status: 'running', outputAvailable: true },
    ],
    ...over,
  }) as SessionDetail;

describe('SessionHeaderComponent switching account', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<SessionHeaderComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([]), SessionContext]);
    serveAccounts(backend, buildAccounts(Date.now(), true));
    asDesktop();
  });

  afterEach(() => setActiveMock(null));

  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const render = async (d: SessionDetail) => {
    const ctx = TestBed.inject(SessionContext);
    ctx.id.set(d.id);
    ctx.detail.set(d);
    fixture = TestBed.createComponent(SessionHeaderComponent);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    return fixture.nativeElement as HTMLElement;
  };

  it('offers Switch account in its menu, saying the background task stops', async () => {
    const el = await render(detail());
    el.querySelector<HTMLButtonElement>('[aria-label="Session menu"]')?.click();
    fixture.detectChanges();
    [...el.querySelectorAll<HTMLButtonElement>('.menu .item')]
      .find((b) => text(b) === 'Switch account…')
      ?.click();
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    const sheet = el.querySelector('mc-switch-account-sheet') as HTMLElement;
    expect(text(sheet)).toContain('After this turn');
    expect(text(sheet.querySelector('.tasks'))).toBe(
      'The background task it started stops when it switches.',
    );
  });

  it('shows a switch waiting for the turn, and cancels it', async () => {
    backend.routes.set('/sessions/s1/account', () => ({ status: 200, body: {} }));
    const el = await render(detail({ switchingTo: MOCK_ACCOUNT_PRO }));
    expect(text(el.querySelector('mc-switch-pending'))).toBe(
      'Switching to Marcus (Pro) once this turn ends Cancel',
    );
    el.querySelector<HTMLButtonElement>('mc-switch-pending button')?.click();
    await flush();
    expect(backend.calls).toContain('DELETE /sessions/s1/account');
  });
});
