import { Component } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import {
  FakeBackend,
  flush,
  serveAccounts,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import { MOCK_ACCOUNT_PRO, MOCK_ACCOUNT_WORK, buildAccounts } from '../../core/mock/fixtures';
import { setActiveMock } from '../../core/mock/mock-api';
import { NewSessionComponent } from './new-session.component';

@Component({ template: '' })
class BlankComponent {}

describe('NewSessionComponent account picker', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<NewSessionComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [
      provideRouter([
        { path: 'session/:id', component: BlankComponent },
        { path: '', component: BlankComponent },
      ]),
    ]);
    backend.routes.set('/projects', () => ({
      status: 200,
      body: {
        projects: [
          { name: 'analytics', path: '/Users/marcus/dev/analytics', lastActivityAt: 2 },
          { name: 'api-server', path: '/Users/marcus/dev/api-server', lastActivityAt: 1 },
        ],
      },
    }));
    backend.routes.set('/sessions', () => ({ status: 201, body: { id: 's-new' } }));
  });

  afterEach(() => setActiveMock(null));

  const render = async () => {
    fixture = TestBed.createComponent(NewSessionComponent);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    return fixture.nativeElement as HTMLElement;
  };
  const text = (el: Element | null) => (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const pickProject = async (el: HTMLElement, name: string) => {
    const btn = [...el.querySelectorAll<HTMLButtonElement>('.project')].find((b) =>
      text(b).startsWith(name),
    );
    btn?.click();
    fixture.detectChanges();
    await fixture.whenStable();
    fixture.detectChanges();
  };
  const start = async (el: HTMLElement) => {
    (el.querySelector('.actions .btn.primary') as HTMLButtonElement).click();
    await flush();
    return backend.bodyOf('POST /sessions') as Record<string, unknown>;
  };
  const accountSelect = (el: HTMLElement) =>
    [...el.querySelectorAll('section.card')]
      .find((s) => text(s.querySelector('h2')) === 'Account')
      ?.querySelector('select') as HTMLSelectElement | undefined;

  it('is not there with only the Default, and the launch names no account', async () => {
    serveAccounts(backend, buildAccounts(Date.now(), false));
    const el = await render();
    expect(accountSelect(el)).toBeUndefined();
    await pickProject(el, 'api-server');
    const body = await start(el);
    expect(body['cwd']).toBe('/Users/marcus/dev/api-server');
    expect('account' in body).toBeFalse();
  });

  it("preselects the project's account, else the default for new sessions", async () => {
    serveAccounts(backend, buildAccounts(Date.now(), true));
    const el = await render();
    await pickProject(el, 'analytics');
    expect(accountSelect(el)?.value).toBe(MOCK_ACCOUNT_WORK);
    expect(text(el)).toContain('The account of ~/dev/analytics, set on the Accounts page.');
    await pickProject(el, 'api-server');
    expect(accountSelect(el)?.value).toBe('default');
    expect(text(el)).toContain('The default for new sessions.');
    const body = await start(el);
    expect(body['account']).toBe('default');
  });

  it('runs under the account picked here', async () => {
    serveAccounts(backend, buildAccounts(Date.now(), true));
    const el = await render();
    await pickProject(el, 'api-server');
    const sel = accountSelect(el) as HTMLSelectElement;
    sel.value = MOCK_ACCOUNT_PRO;
    sel.dispatchEvent(new Event('change'));
    fixture.detectChanges();
    expect(text(el)).toContain("Chosen here, for this session. It shares this Mac's projects");
    const body = await start(el);
    expect(body['account']).toBe(MOCK_ACCOUNT_PRO);
  });

  it('explains an account the server refuses', async () => {
    serveAccounts(backend, buildAccounts(Date.now(), true));
    backend.routes.set('/sessions', () => ({
      status: 409,
      body: { error: { code: 'account_token_missing', message: 'token not stored' } },
    }));
    const el = await render();
    await pickProject(el, 'api-server');
    const sel = accountSelect(el) as HTMLSelectElement;
    sel.value = MOCK_ACCOUNT_PRO;
    sel.dispatchEvent(new Event('change'));
    fixture.detectChanges();
    await start(el);
    fixture.detectChanges();
    expect(text(el.querySelector('.notice.error'))).toContain(
      'Could not start the session: Marcus (Pro) has no token on this Mac yet',
    );
  });
});
