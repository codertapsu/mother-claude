import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import {
  FakeBackend,
  asDesktop,
  flush,
  serveAccounts,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import {
  MOCK_ACCOUNT_PRO,
  MOCK_ACCOUNT_WORK,
  accountFixture,
  buildAccounts,
} from '../../core/mock/fixtures';
import { setActiveMock } from '../../core/mock/mock-api';
import { AccountView, AccountsView } from '../../core/models';
import { AccountSignInComponent } from './account-signin.component';

describe('AccountSignInComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<AccountSignInComponent>;
  const work = accountFixture({
    id: MOCK_ACCOUNT_WORK,
    name: 'Work',
    dir: '/u/.claude-work',
    email: 'work@example.com',
  });

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    asDesktop();
  });

  afterEach(() => setActiveMock(null));

  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const button = (el: HTMLElement, label: string) =>
    [...el.querySelectorAll<HTMLButtonElement>('button')].find((b) => text(b).startsWith(label));
  const render = async (a: AccountView, view: AccountsView = buildAccounts(Date.now(), true)) => {
    serveAccounts(backend, view);
    fixture = TestBed.createComponent(AccountSignInComponent);
    fixture.componentRef.setInput('account', a);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    return fixture.nativeElement as HTMLElement;
  };

  it('shows the sign-in panel while its sign-in runs', async () => {
    const el = await render({
      ...work,
      signIn: { kind: 'login', state: 'starting', startedAt: 1 },
    });
    expect(el.querySelector('mc-sign-in-panel')).not.toBeNull();
    expect(text(el)).toContain('Starting the sign-in…');
  });

  it("waits for another account's sign-in, naming it", async () => {
    const view = buildAccounts(Date.now(), true);
    view.accounts = view.accounts.map((a) =>
      a.id === MOCK_ACCOUNT_PRO
        ? { ...a, signIn: { kind: 'token', state: 'code', startedAt: 1 } }
        : a,
    );
    const el = await render(work, view);
    const b = button(el, 'Waiting for');
    expect(text(b)).toBe("Waiting for Marcus (Pro)'s sign-in");
    expect(b?.disabled).toBeTrue();
    // Signing in again asks for the same Claude account.
    expect(text(el)).toContain('it asks for work@example.com again');
  });

  it('says why the Mac refused, and how the last sign-in ended', async () => {
    backend.routes.set(`/accounts/${MOCK_ACCOUNT_WORK}/login`, () => ({
      status: 409,
      body: {
        error: { code: 'sign_in_running', message: 'busy', account: MOCK_ACCOUNT_PRO },
      },
    }));
    // A run that ended long ago says nothing any more…
    let el = await render({
      ...work,
      signIn: { kind: 'login', state: 'cancelled', startedAt: 1, finishedAt: 2 },
    });
    expect(el.querySelector('.notice.warn')).toBeNull();
    fixture.destroy();
    // …one that just ended does.
    el = await render({
      ...work,
      signIn: {
        kind: 'login',
        state: 'cancelled',
        startedAt: Date.now() - 5_000,
        finishedAt: Date.now(),
      },
    });
    expect(text(el.querySelector('.notice.warn'))).toBe('The sign-in was cancelled.');
    button(el, 'Sign in again')?.click();
    await flush();
    fixture.detectChanges();
    expect(text(el.querySelector('[role="alert"]'))).toBe(
      "Marcus (Pro)'s sign-in is running: one sign-in runs at a time. Finish or cancel it first.",
    );
  });
});
