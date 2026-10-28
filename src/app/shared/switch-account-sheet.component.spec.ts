import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import {
  FakeBackend,
  asDesktop,
  flush,
  serveAccounts,
  useFakeServer,
} from '../core/mock/fake-server.testing';
import {
  MOCK_ACCOUNT_PRO,
  MOCK_ACCOUNT_WORK,
  buildAccounts,
  ownedCaps,
} from '../core/mock/fixtures';
import { setActiveMock } from '../core/mock/mock-api';
import { AccountRef, SessionSummary } from '../core/models';
import { NoticeService } from '../core/notice.service';
import { SwitchAccountSheetComponent } from './switch-account-sheet.component';

const DEFAULT_REF: AccountRef = {
  id: 'default',
  name: 'Default',
  color: '#8b5cf6',
  kind: 'directory',
};

const session = (over: Partial<SessionSummary> = {}): SessionSummary =>
  ({
    id: 's1',
    name: 's1',
    title: 'Fix the login',
    projectName: 'api',
    cwd: '/Users/me/dev/api',
    surface: 'mother-claude',
    owned: true,
    live: true,
    status: 'idle',
    activity: { kind: 'idle', label: 'Idle' },
    usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
    messageCount: 3,
    agents: { running: 0, total: 0, workflowsRunning: 0, backgroundRunning: 0 },
    queued: 0,
    pending: [],
    control: ownedCaps(),
    account: DEFAULT_REF,
    ...over,
  }) as SessionSummary;

describe('SwitchAccountSheetComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<SwitchAccountSheetComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    serveAccounts(backend, buildAccounts(Date.now(), true));
    asDesktop();
  });

  afterEach(() => setActiveMock(null));

  const settle = async () => {
    for (let i = 0; i < 3; i++) {
      await flush();
      fixture.detectChanges();
    }
  };
  const open = async (s: SessionSummary, extra: Record<string, unknown> = {}) => {
    fixture = TestBed.createComponent(SwitchAccountSheetComponent);
    fixture.componentRef.setInput('sessions', [s]);
    fixture.componentRef.setInput('suggested', MOCK_ACCOUNT_PRO);
    for (const [k, v] of Object.entries(extra)) fixture.componentRef.setInput(k, v);
    fixture.componentRef.setInput('open', true);
    fixture.detectChanges();
    await settle();
    return fixture.nativeElement as HTMLElement;
  };
  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const button = (el: HTMLElement, label: string) =>
    [...el.querySelectorAll<HTMLButtonElement>('button')].find((b) => text(b).startsWith(label));
  const target = (el: HTMLElement, name: string) =>
    [...el.querySelectorAll<HTMLButtonElement>('[aria-label="Account"] .opt')].find((b) =>
      text(b).startsWith(name),
    );
  const sent = () => backend.bodyOf('POST /sessions/s1/account');

  it('switches a session Mother Claude runs, the same session going on', async () => {
    backend.routes.set('/sessions/s1/account', () => ({
      status: 200,
      body: { id: 's1', account: MOCK_ACCOUNT_PRO, moved: false },
    }));
    const el = await open(session());
    expect(text(el.querySelector('.lede'))).toContain(
      'The same session goes on under the account you pick',
    );
    const success = spyOn(TestBed.inject(NoticeService), 'success').and.callThrough();
    const closed = jasmine.createSpy('closed');
    fixture.componentInstance.closed.subscribe(closed);
    button(el, 'Switch to Marcus (Pro)')?.click();
    await settle();
    expect(sent()).toEqual({ account: MOCK_ACCOUNT_PRO });
    expect(closed).toHaveBeenCalled();
    // The Mac's notice to every client says it, not a second one from here.
    expect(success).not.toHaveBeenCalled();
  });

  it('says it switched one that does not run: the Mac tells nobody', async () => {
    backend.routes.set('/sessions/s1/account', () => ({
      status: 200,
      body: { id: 's1', account: MOCK_ACCOUNT_WORK, moved: true, job: 'j1' },
    }));
    const el = await open(session({ live: false, status: 'ended' }));
    const success = spyOn(TestBed.inject(NoticeService), 'success').and.callThrough();
    target(el, 'Work')?.click();
    await settle();
    button(el, 'Switch to Work')?.click();
    await settle();
    expect(sent()).toEqual({ account: MOCK_ACCOUNT_WORK });
    expect(success).toHaveBeenCalledWith('Now on Work: the conversation moved to its folder.');
  });

  it('waits for the turn by default, shows the switch waiting, and cancels it', async () => {
    backend.routes.set('/sessions/s1/account', (method) =>
      method === 'DELETE' ? { status: 200, body: {} } : { status: 202, body: { pending: true } },
    );
    const el = await open(session({ status: 'working' }), { tasks: 2 });
    const whens = [...el.querySelectorAll<HTMLInputElement>('input[name="when"]')];
    expect(whens.map((w) => w.checked)).toEqual([true, false]);
    expect(text(el.querySelector('[aria-label="When"]'))).toContain('Now — interrupts the turn');
    expect(text(el.querySelector('.tasks'))).toBe(
      'The 2 background tasks it started stop when it switches.',
    );
    button(el, 'Switch to Marcus (Pro)')?.click();
    await settle();
    expect(sent()).toEqual({ account: MOCK_ACCOUNT_PRO, when: 'idle' });
    expect(text(el.querySelector('.pending'))).toContain(
      'Switching to Marcus (Pro) once this turn ends.',
    );
    button(el, 'Cancel the switch')?.click();
    await settle();
    expect(backend.calls).toContain('DELETE /sessions/s1/account');
    expect(el.querySelector('.pending')).toBeNull();

    // Now: the turn is interrupted.
    el.querySelectorAll<HTMLInputElement>('input[name="when"]')[1].click();
    fixture.detectChanges();
    button(el, 'Switch to Marcus (Pro)')?.click();
    await settle();
    expect(sent()).toEqual({ account: MOCK_ACCOUNT_PRO, when: 'now' });
  });

  it('asks once before moving a conversation into another directory', async () => {
    let asked = 0;
    backend.routes.set('/sessions/s1/account', () =>
      ++asked === 1
        ? {
            status: 409,
            body: {
              error: {
                code: 'copy_consent_required',
                message: 'allow it first',
                from: 'default',
                to: MOCK_ACCOUNT_WORK,
              },
            },
          }
        : { status: 200, body: { id: 's1', account: MOCK_ACCOUNT_WORK, moved: true } },
    );
    backend.routes.set('/accounts/copy-consent', () => ({
      status: 200,
      body: buildAccounts(Date.now(), true),
    }));
    const el = await open(session());
    target(el, 'Work')?.click();
    fixture.detectChanges();
    expect(text(target(el, 'Work'))).toContain(
      'Keeps its own directory: the conversation is moved there first (asked once).',
    );
    button(el, 'Switch to Work')?.click();
    await settle();
    expect(text(el.querySelector('.ask'))).toContain(
      'Move conversations from Default to Work when you switch them there?',
    );
    button(el, 'Allow and go on')?.click();
    await settle();
    expect(backend.bodyOf('PUT /accounts/copy-consent')).toEqual(
      jasmine.objectContaining({ from: 'default', to: MOCK_ACCOUNT_WORK, granted: true }),
    );
    expect(asked).toBe(2);
  });

  it('says a session another app runs keeps its account, and continues it on the other', async () => {
    backend.routes.set('/sessions/s1/continue', () => ({ status: 201, body: { id: 's9' } }));
    const el = await open(
      session({ owned: false, surface: 'vscode', control: { ...ownedCaps(), fork: true } }),
    );
    expect(text(el.querySelector('.lede'))).toContain(
      "It runs in VS Code: its account can't change while it runs there. Continue on another account instead",
    );
    button(el, 'Continue on Marcus (Pro)')?.click();
    await settle();
    expect(backend.bodyOf('POST /sessions/s1/continue')).toEqual({
      account: MOCK_ACCOUNT_PRO,
      fork: true,
    });
    expect(backend.calls).not.toContain('POST /sessions/s1/account');
  });

  it('switches an ended one for its next continue, and puts refusals in words', async () => {
    backend.routes.set('/sessions/s1/account', () => ({
      status: 409,
      body: { error: { code: 'turn_running', message: 'a turn runs' } },
    }));
    const el = await open(session({ live: false, status: 'ended' }));
    expect(text(el.querySelector('.lede'))).toContain(
      "It isn't running: its next continue runs under the account you pick.",
    );
    expect(el.querySelector('[aria-label="When"]')).toBeNull();
    button(el, 'Switch to Marcus (Pro)')?.click();
    await settle();
    expect(text(el.querySelector('.notice.error'))).toContain('its account changes between turns');
  });

  it('turns to Continue on when the Mac says another app runs it', async () => {
    backend.routes.set('/sessions/s1/account', () => ({
      status: 409,
      body: { error: { code: 'session_not_owned', message: 'runs elsewhere' } },
    }));
    const el = await open(session());
    button(el, 'Switch to Marcus (Pro)')?.click();
    await settle();
    expect(button(el, 'Continue on Marcus (Pro)')).toBeDefined();
    expect(text(el.querySelector('.notice.error'))).toContain('continue it on the other account');
  });

  it('switches a conversation of the API between turns only', async () => {
    backend.routes.set('/sessions/s1/account', () => ({
      status: 200,
      body: { id: 's1', account: MOCK_ACCOUNT_PRO, moved: false },
    }));
    const api = {
      ...ownedCaps(),
      limits: [{ code: 'bridge_thread', text: 'A conversation of the API.' }],
    };
    let el = await open(session({ status: 'working', control: api }));
    expect(el.querySelector('[aria-label="When"]')).toBeNull();
    expect(text(el.querySelector('.between'))).toContain('in the middle of a turn');
    expect(button(el, 'Switch to Marcus (Pro)')?.disabled).toBeTrue();
    fixture.destroy();

    el = await open(session({ control: api }));
    expect(text(el.querySelector('.between'))).toBe(
      'A conversation of the API changes account between turns.',
    );
    button(el, 'Switch to Marcus (Pro)')?.click();
    await settle();
    // No "when" for it.
    expect(sent()).toEqual({ account: MOCK_ACCOUNT_PRO });
  });
});
