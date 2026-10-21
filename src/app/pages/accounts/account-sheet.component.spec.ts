import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import {
  FakeBackend,
  asDesktop,
  flush,
  serveAccounts,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import { MOCK_ACCOUNT_WORK, buildAccounts } from '../../core/mock/fixtures';
import { setActiveMock } from '../../core/mock/mock-api';
import { AccountSheetComponent } from './account-sheet.component';

describe('AccountSheetComponent removal', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<AccountSheetComponent>;

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
    [...el.querySelectorAll<HTMLButtonElement>('button')].find((b) => text(b).startsWith(label));

  const removeWork = async (refusal: { code: string; message: string; [k: string]: unknown }) => {
    backend.routes.set(`/accounts/${MOCK_ACCOUNT_WORK}`, (method) =>
      method === 'DELETE' ? { status: 409, body: { error: refusal } } : { status: 404, body: {} },
    );
    fixture = TestBed.createComponent(AccountSheetComponent);
    fixture.componentRef.setInput('accountId', MOCK_ACCOUNT_WORK);
    fixture.detectChanges();
    const el = fixture.nativeElement as HTMLElement;
    button(el, 'Remove Work')?.click();
    fixture.detectChanges();
    [...el.querySelectorAll<HTMLButtonElement>('button.btn.danger')]
      .find((b) => text(b) === 'Remove')
      ?.click();
    await flush();
    fixture.detectChanges();
    return el;
  };

  it('says the running API uses the account, and where to stop it', async () => {
    const el = await removeWork({
      code: 'account_used_by_bridge',
      message: 'The API runs with this account as its default for new conversations.',
    });
    expect(backend.calls).toContain(`DELETE /accounts/${MOCK_ACCOUNT_WORK}`);
    expect(text(el.querySelector('.error-text'))).toBe(
      'The API is running with Work as the account its new conversations start under. Stop the API, or start it again with another account, then remove Work.',
    );
    const link = el.querySelector('a[href="/api"]');
    expect(text(link)).toBe('Open the API page');
  });

  it('names the API server that runs under it, and links to that server', async () => {
    const el = await removeWork({
      code: 'account_used_by_bridge',
      message: 'An API server runs with this account.',
      server: 'work',
      serverName: 'Work API',
    });
    expect(text(el.querySelector('.error-text'))).toBe(
      'The API server Work API is running under Work. Stop it, or start it with another account, then remove Work.',
    );
    const link = el.querySelector('a[href="/api?server=work"]');
    expect(text(link)).toBe('Open the API page');
  });

  it('says sessions still run under it, without the API link', async () => {
    const el = await removeWork({
      code: 'account_in_use',
      message: '2 session(s) Mother Claude started still run under this account.',
    });
    expect(text(el.querySelector('.error-text'))).toBe(
      '2 session(s) Mother Claude started still run under this account. Stop them first.',
    );
    expect(el.querySelector('a[href="/api"]')).toBeNull();
  });
});
