import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import {
  FakeBackend,
  asDesktop,
  flush,
  serveAccounts,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import { MOCK_ACCOUNT_PRO, MOCK_ACCOUNT_WORK, buildAccounts } from '../../core/mock/fixtures';
import { setActiveMock } from '../../core/mock/mock-api';
import { AccountsComponent } from './accounts.component';

describe('AccountsComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<AccountsComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
  });

  afterEach(() => setActiveMock(null));

  const render = async () => {
    fixture = TestBed.createComponent(AccountsComponent);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    return fixture.nativeElement as HTMLElement;
  };
  const text = (el: Element | null) => (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const buttons = (el: HTMLElement) => [...el.querySelectorAll('button')].map((b) => text(b));

  it('with only the Default shows that one account, and the desktop can add another', async () => {
    asDesktop();
    serveAccounts(backend, buildAccounts(Date.now(), false));
    const el = await render();
    const rows = el.querySelectorAll('.acct');
    expect(rows.length).toBe(1);
    expect(text(rows[0])).toContain('Default');
    expect(text(rows[0])).toContain('Own directory · ~/.claude');
    expect(text(rows[0])).toContain('marcus@example.com · Max');
    expect(buttons(el)).toContain('Add account');
    // Nothing to choose between yet.
    expect(text(el)).not.toContain('New sessions run under');
    expect(text(el)).toContain('Account tokens are kept in the macOS Keychain');
  });

  it('shows a phone every account read-only', async () => {
    serveAccounts(backend, buildAccounts(Date.now(), true));
    const el = await render();
    const rows = [...el.querySelectorAll('.acct')];
    expect(rows.map((r) => text(r.querySelector('.aname')))).toEqual([
      'Default',
      'Work',
      'Marcus (Pro)',
    ]);
    expect(text(rows[2])).toContain("Token, shares the Default's data");
    expect(text(el.querySelector('.phone-note'))).toContain('added and signed in on the Mac');
    // No administration at all: no add, manage, sign-in or selects.
    const labels = buttons(el);
    expect(labels).not.toContain('Add account');
    expect(labels.some((b) => b.includes('Manage'))).toBeFalse();
    expect(el.querySelectorAll('.defaults select').length).toBe(0);
    expect(el.querySelector('form.padd')).toBeNull();
    // The defaults are still there to read.
    const rowsOfDefaults = [...el.querySelectorAll('.defaults .drow')];
    expect(text(rowsOfDefaults[0].querySelector('strong'))).toBe('New sessions run under');
    expect(text(rowsOfDefaults[0].querySelector('.dval'))).toBe('Default');
    expect(text(rowsOfDefaults[1].querySelector('.dval'))).toBe('Default');
    expect(text(el.querySelector('.prow .pdir'))).toBe('~/dev/analytics');
    expect(text(el.querySelector('.prow .pname'))).toBe('Work');
  });

  it('lets the desktop choose the defaults and add a project rule', async () => {
    asDesktop();
    const view = buildAccounts(Date.now(), true);
    serveAccounts(backend, view);
    backend.routes.set('/accounts/defaults', (_m, body) => ({ status: 200, body }));
    const el = await render();

    const sel = el.querySelector('#acc-new-session') as HTMLSelectElement;
    sel.value = MOCK_ACCOUNT_PRO;
    sel.dispatchEvent(new Event('change'));
    await flush();
    expect(backend.bodyOf('PUT /accounts/defaults')).toEqual({
      newSession: MOCK_ACCOUNT_PRO,
      bridge: 'default',
      byProject: { '/Users/marcus/dev/analytics': MOCK_ACCOUNT_WORK },
    });

    const dir = el.querySelector('form.padd input[name="dir"]') as HTMLInputElement;
    dir.value = '/Users/marcus/dev/payments/';
    dir.dispatchEvent(new Event('input'));
    fixture.detectChanges();
    (el.querySelector('form.padd button[type="submit"]') as HTMLButtonElement).click();
    await flush();
    const body = backend.bodyOf('PUT /accounts/defaults') as { byProject: Record<string, string> };
    // The rule names the first account that isn't the Default, without the trailing slash.
    expect(body.byProject['/Users/marcus/dev/payments']).toBe(MOCK_ACCOUNT_WORK);
  });

  it('refuses a project folder that is not a full path', async () => {
    asDesktop();
    serveAccounts(backend, buildAccounts(Date.now(), true));
    const el = await render();
    const dir = el.querySelector('form.padd input[name="dir"]') as HTMLInputElement;
    dir.value = 'dev/payments';
    dir.dispatchEvent(new Event('input'));
    fixture.detectChanges();
    (el.querySelector('form.padd button[type="submit"]') as HTMLButtonElement).click();
    await flush();
    fixture.detectChanges();
    expect(backend.calls).not.toContain('PUT /accounts/defaults');
    expect(text(el.querySelector('form.padd + .error-text'))).toContain('starting with /');
  });
});
