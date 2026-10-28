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
import { AccountView, SignInView } from '../../core/models';
import { SignInPanelComponent } from './sign-in-panel.component';

const URL = 'https://claude.ai/oauth/authorize?client_id=x';

describe('SignInPanelComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<SignInPanelComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    serveAccounts(backend, buildAccounts(Date.now(), true));
    asDesktop();
  });

  afterEach(() => setActiveMock(null));

  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const buttons = (el: HTMLElement) =>
    [...el.querySelectorAll<HTMLButtonElement>('button')].map((b) => text(b));
  const button = (el: HTMLElement, label: string) =>
    [...el.querySelectorAll<HTMLButtonElement>('button')].find((b) => text(b).startsWith(label));

  /** The account as the frames say it (no link), and as the desktop's own
   * read says it (with the link). */
  const render = async (
    a: AccountView,
    run: SignInView,
    browsers: string[] = ['default', 'chrome', 'chrome-private'],
  ) => {
    backend.routes.set(`/accounts/${a.id}`, () => ({
      status: 200,
      body: { ...a, signIn: { ...run, url: URL } },
    }));
    backend.routes.set(`/accounts/${a.id}/sign-in/open`, (method) =>
      method === 'GET' ? { status: 200, body: { browsers } } : { status: 200, body: {} },
    );
    fixture = TestBed.createComponent(SignInPanelComponent);
    fixture.componentRef.setInput('account', { ...a, signIn: run });
    fixture.detectChanges();
    for (let i = 0; i < 3; i++) {
      await flush();
      fixture.detectChanges();
    }
    return fixture.nativeElement as HTMLElement;
  };

  const work = accountFixture({ id: MOCK_ACCOUNT_WORK, name: 'Work', dir: '/u/.claude-work' });
  const pro = accountFixture({ id: MOCK_ACCOUNT_PRO, name: 'Pro', kind: 'token' });

  it('says a login opened the default browser, and offers a private window and the link', async () => {
    const el = await render(work, { kind: 'login', state: 'open', startedAt: 1 });
    expect(text(el)).toContain('The sign-in opened in your default browser.');
    expect(text(el)).toContain('Sign in there with the Claude account for Work.');
    expect(buttons(el)).toEqual([
      'Open it again',
      'Open in Chrome',
      'Open in a private Chrome window',
      'Copy link',
      'Cancel',
    ]);
    expect(text(el)).toContain(
      'The wrong Claude account opened? A private window starts signed out.',
    );
    button(el, 'Open in a private Chrome window')?.click();
    await flush();
    expect(backend.bodyOf(`POST /accounts/${MOCK_ACCOUNT_WORK}/sign-in/open`)).toEqual({
      browser: 'chrome-private',
    });
  });

  it('asks before a token sign-in opens anything, recommending the private window', async () => {
    let el = await render(pro, { kind: 'token', state: 'open', startedAt: 1 });
    expect(text(el)).toContain('Open the sign-in page to get the token for Pro.');
    expect(text(el)).toContain(
      'a browser already signed in to another Claude account would give its consent to the wrong one',
    );
    const first = el.querySelector<HTMLButtonElement>('.opens button');
    expect(text(first)).toBe('Open in a private Chrome window (recommended)');
    expect(first?.classList).toContain('primary');
    // Opening is the user's choice: nothing was opened by itself.
    expect(backend.calls).not.toContain(`POST /accounts/${MOCK_ACCOUNT_PRO}/sign-in/open`);

    // Without Chrome, the default browser leads.
    fixture.destroy();
    el = await render(pro, { kind: 'token', state: 'open', startedAt: 2 }, ['default']);
    expect(text(el.querySelector('.opens button'))).toBe('Open in browser');
  });

  it('types the code the browser shows, and cancels', async () => {
    const el = await render(pro, { kind: 'token', state: 'code', startedAt: 1 });
    const input = el.querySelector<HTMLInputElement>('input[name="code"]')!;
    input.value = 'abc#123';
    input.dispatchEvent(new Event('input'));
    fixture.detectChanges();
    backend.routes.set(`/accounts/${MOCK_ACCOUNT_PRO}/sign-in/code`, () => ({
      status: 200,
      body: {},
    }));
    button(el, 'Send the code')?.click();
    await flush();
    expect(backend.bodyOf(`POST /accounts/${MOCK_ACCOUNT_PRO}/sign-in/code`)).toEqual({
      code: 'abc#123',
    });
    backend.routes.set(`/accounts/${MOCK_ACCOUNT_PRO}/sign-in`, () => ({ status: 200, body: {} }));
    button(el, 'Cancel')?.click();
    await flush();
    expect(backend.calls).toContain(`DELETE /accounts/${MOCK_ACCOUNT_PRO}/sign-in`);
  });

  it('tells its host how the sign-in ended', async () => {
    await render(work, { kind: 'login', state: 'open', startedAt: 1 });
    const ended = jasmine.createSpy('ended');
    fixture.componentInstance.finished.subscribe(ended);
    backend.routes.set(`/accounts/${MOCK_ACCOUNT_WORK}`, () => ({
      status: 200,
      body: { ...work, signIn: { kind: 'login', state: 'done', startedAt: 1 } },
    }));
    fixture.componentRef.setInput('account', {
      ...work,
      signIn: { kind: 'login', state: 'done', startedAt: 1 },
    });
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    expect(ended).toHaveBeenCalledWith('done');
    expect(text(fixture.nativeElement as HTMLElement)).toBe('');
  });
});
