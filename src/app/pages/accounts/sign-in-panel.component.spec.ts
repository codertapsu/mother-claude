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

  it('says the Mac opened a login in the default browser, and offers only a private window and the link', async () => {
    const el = await render(work, {
      kind: 'login',
      state: 'open',
      opened: 'default',
      startedAt: 1,
    });
    expect(text(el)).toContain('The sign-in opened in your default browser.');
    expect(text(el)).toContain('Sign in there with the Claude account for Work.');
    // Never the default browser again from here.
    expect(buttons(el)).toEqual([
      'Open in a private Chrome window',
      'Copy link',
      'Send the code',
      'Cancel',
    ]);
    expect(text(el)).toContain(
      'Is that browser signed in to another Claude account? Use a private window: it starts signed out.',
    );
    // The link itself, selectable, to paste anywhere.
    expect(text(el.querySelector('code.url'))).toBe(URL);
    backend.routes.set(`/accounts/${MOCK_ACCOUNT_WORK}/sign-in/open`, () => ({
      status: 200,
      body: {
        signIn: {
          kind: 'login',
          state: 'open',
          url: URL,
          opened: 'chrome-private',
          startedAt: 1,
        },
      },
    }));
    button(el, 'Open in a private Chrome window')?.click();
    await flush();
    fixture.detectChanges();
    expect(backend.bodyOf(`POST /accounts/${MOCK_ACCOUNT_WORK}/sign-in/open`)).toEqual({
      browser: 'chrome-private',
    });
    // The answer says where it opened.
    expect(text(el)).toContain('The sign-in opened in a private Chrome window.');
    expect(text(el)).not.toContain('Use a private window');
    expect(buttons(el)).toEqual(['Copy link', 'Send the code', 'Cancel']);
  });

  it('offers an Edge InPrivate window on a PC that has Edge and no Chrome', async () => {
    let el = await render(work, { kind: 'login', state: 'open', opened: 'default', startedAt: 1 }, [
      'default',
      'edge-private',
    ]);
    expect(buttons(el)).toEqual([
      'Open in an Edge InPrivate window',
      'Copy link',
      'Send the code',
      'Cancel',
    ]);
    fixture.destroy();
    // Both: both private windows, a token sign-in recommending the first.
    el = await render(pro, { kind: 'token', state: 'open', startedAt: 1 }, [
      'default',
      'chrome',
      'chrome-private',
      'edge-private',
    ]);
    expect([...el.querySelectorAll('.opens button')].map((b) => text(b))).toEqual([
      'Open in a private Chrome window (recommended)',
      'Open in an Edge InPrivate window',
      'Open in browser',
      'Open in Chrome',
      'Copy link',
    ]);
    expect(text(el.querySelector('code.url'))).toBe(URL);
  });

  it('says a login is about to open, before the Mac opened it', async () => {
    const el = await render(work, { kind: 'login', state: 'open', startedAt: 1 }, ['default']);
    expect(text(el)).toContain('The sign-in opens in your default browser…');
    expect(text(el)).toContain('Copy the link into a private (incognito) window.');
    expect(buttons(el)).toEqual(['Copy link', 'Send the code', 'Cancel']);
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

  it('offers the code field as soon as the link shows, and says what happens', async () => {
    for (const a of [work, pro]) {
      const el = await render(a, {
        kind: a === work ? 'login' : 'token',
        state: 'open',
        startedAt: 1,
      });
      expect(el.querySelector('input[name="code"]')).withContext(a.name).not.toBeNull();
      expect(text(el)).toContain(
        'After you approve in the browser, Claude shows a code: copy it and paste it here.',
      );
      expect(el.querySelector('form.code.asked')).toBeNull();
      fixture.destroy();
    }
    // Once the CLI asks for it, it stands out and takes the focus.
    const el = await render(pro, { kind: 'token', state: 'code', startedAt: 2 });
    expect(el.querySelector('form.code.asked')).not.toBeNull();
    expect(document.activeElement).toBe(el.querySelector('input[name="code"]'));
  });

  it('says a code went in, then why Claude Code turned it down, and takes another', async () => {
    const sent: SignInView = { kind: 'token', state: 'open', codeSent: true, startedAt: 1, rev: 4 };
    let el = await render(pro, sent);
    expect(el.querySelector('input[name="code"]')).toBeNull();
    expect(text(el)).toContain('Code sent: Claude Code is finishing the sign-in…');
    expect(buttons(el)).toContain('Cancel');

    // Turned down: the field is back, with why.
    const why = 'Invalid code. Please make sure the full code was copied';
    const refused: SignInView = {
      kind: 'token',
      state: 'code',
      codeError: why,
      startedAt: 1,
      rev: 5,
    };
    backend.routes.set(`/accounts/${pro.id}`, () => ({
      status: 200,
      body: { ...pro, signIn: { ...refused, url: URL } },
    }));
    fixture.componentRef.setInput('account', { ...pro, signIn: refused });
    for (let i = 0; i < 3; i++) {
      await flush();
      fixture.detectChanges();
    }
    el = fixture.nativeElement as HTMLElement;
    expect(text(el.querySelector('.notice.warn'))).toBe(
      `Claude Code didn't take that code: ${why}`,
    );
    expect(text(el)).toContain('Paste the whole code the page shows.');
    expect(el.querySelector('input[name="code"]')).not.toBeNull();
    expect(text(el)).not.toContain('Code sent');
  });

  it('keeps a token pasted in a token sign-in as the token, never sending it as a code', async () => {
    const token = 'sk-ant-oat01-abcdefghijklmnop_-0123456789';
    const el = await render(pro, { kind: 'token', state: 'code', startedAt: 1, rev: 2 });
    backend.routes.set(`/accounts/${pro.id}/token`, () => ({
      status: 200,
      body: {
        ...pro,
        tokenStored: true,
        signIn: { kind: 'token', state: 'done', startedAt: 1, rev: 3 },
      },
    }));
    const input = el.querySelector<HTMLInputElement>('input[name="code"]')!;
    input.value = ` ${token} `;
    input.dispatchEvent(new Event('input'));
    fixture.detectChanges();
    button(el, 'Send the code')?.click();
    await flush();
    fixture.detectChanges();
    expect(backend.bodyOf(`POST /accounts/${pro.id}/token`)).toEqual({ token });
    expect(backend.calls).not.toContain(`POST /accounts/${pro.id}/sign-in/code`);
    // It went only there, and the field is empty.
    expect(
      JSON.stringify(
        backend.bodies.filter((b) => !!b && (b as { token?: string }).token !== token),
      ),
    ).not.toContain(token);
    expect(fixture.componentInstance['code']()).toBe('');
  });

  it('says a token is not the code in a login, and sends nothing', async () => {
    const el = await render(work, { kind: 'login', state: 'code', startedAt: 1 });
    const before = backend.calls.filter((c) => c.startsWith('POST')).length;
    const input = el.querySelector<HTMLInputElement>('input[name="code"]')!;
    input.value = 'sk-ant-oat01-abcdefghijklmnop_-0123456789';
    input.dispatchEvent(new Event('input'));
    fixture.detectChanges();
    button(el, 'Send the code')?.click();
    await flush();
    fixture.detectChanges();
    expect(backend.calls.filter((c) => c.startsWith('POST')).length).toBe(before);
    expect(text(el.querySelector('[role="alert"]'))).toContain("That's a token, not the code");
    expect(fixture.componentInstance['code']()).toBe('');
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
