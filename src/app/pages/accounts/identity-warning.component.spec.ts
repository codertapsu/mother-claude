import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { AuthService } from '../../core/auth.service';
import {
  FakeBackend,
  asDesktop,
  flush,
  me,
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
import { AccountView } from '../../core/models';
import { IdentityWarningComponent } from './identity-warning.component';

describe('IdentityWarningComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<IdentityWarningComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    serveAccounts(backend, buildAccounts(Date.now(), true));
    asDesktop();
  });

  afterEach(() => setActiveMock(null));

  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const button = (el: HTMLElement, label: string) =>
    [...el.querySelectorAll<HTMLButtonElement>('button')].find((b) => text(b) === label);
  const render = (a: AccountView) => {
    fixture = TestBed.createComponent(IdentityWarningComponent);
    fixture.componentRef.setInput('account', a);
    fixture.detectChanges();
    return fixture.nativeElement as HTMLElement;
  };

  it('says who it signed in as now, and signs in again on asking', async () => {
    const work = accountFixture({
      id: MOCK_ACCOUNT_WORK,
      name: 'Work',
      email: 'other@example.com',
      identityWarning: { kind: 'changed' },
    });
    backend.routes.set(`/accounts/${MOCK_ACCOUNT_WORK}/login`, () => ({ status: 202, body: work }));
    const el = render(work);
    expect(text(el.querySelector('p'))).toBe(
      'It signed in as a different Claude account than before. Now: other@example.com.',
    );
    const started = jasmine.createSpy('started');
    fixture.componentInstance.started.subscribe(started);
    button(el, 'Sign in again')?.click();
    await flush();
    expect(backend.calls).toContain(`POST /accounts/${MOCK_ACCOUNT_WORK}/login`);
    expect(started).toHaveBeenCalled();
  });

  it('names the other account, gets a token account a new token, and leaves the buttons to the Mac', async () => {
    const pro = accountFixture({
      id: MOCK_ACCOUNT_PRO,
      name: 'Pro',
      kind: 'token',
      identityWarning: { kind: 'same-as', account: MOCK_ACCOUNT_WORK },
    });
    backend.routes.set(`/accounts/${MOCK_ACCOUNT_PRO}/token`, () => ({ status: 202, body: {} }));
    let el = render(pro);
    expect(text(el.querySelector('p'))).toContain('the same Claude account as Work');
    button(el, 'Sign in again')?.click();
    await flush();
    expect(backend.bodyOf(`POST /accounts/${MOCK_ACCOUNT_PRO}/token`)).toEqual({});

    fixture.destroy();
    TestBed.inject(AuthService).me.set(me());
    el = render(pro);
    expect(text(el.querySelector('p'))).toContain('the same Claude account as Work');
    expect(el.querySelector('button')).toBeNull();
  });
});
