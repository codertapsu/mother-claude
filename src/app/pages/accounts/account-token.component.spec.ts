import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import {
  FakeBackend,
  asDesktop,
  flush,
  serveAccounts,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import { MOCK_ACCOUNT_PRO, accountFixture, buildAccounts } from '../../core/mock/fixtures';
import { setActiveMock } from '../../core/mock/mock-api';
import { AccountView } from '../../core/models';
import { AccountTokenComponent } from './account-token.component';

const TOKEN = 'sk-ant-oat01-abcdefghijklmnop';
const PATH = `/accounts/${MOCK_ACCOUNT_PRO}/token`;

describe('AccountTokenComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<AccountTokenComponent>;
  let account: AccountView;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    asDesktop();
    serveAccounts(backend, buildAccounts(Date.now(), true));
    account = accountFixture({
      id: MOCK_ACCOUNT_PRO,
      name: 'Marcus (Pro)',
      kind: 'token',
      signedIn: false,
      attention: 'tokenMissing',
    });
  });

  afterEach(() => {
    fixture?.destroy();
    setActiveMock(null);
  });

  const render = async () => {
    fixture = TestBed.createComponent(AccountTokenComponent);
    fixture.componentRef.setInput('account', account);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    return fixture.nativeElement as HTMLElement;
  };
  const text = (el: Element | null) => (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const button = (el: HTMLElement, label: string) =>
    [...el.querySelectorAll('button')].find((b) => text(b).includes(label)) as HTMLButtonElement;

  it('sends a pasted token once, in the body, and empties the field', async () => {
    let tokenRoute: unknown;
    backend.routes.set(PATH, (method, body) => {
      if (method === 'POST') tokenRoute = body;
      return method === 'GET'
        ? { status: 200, body: { available: true, tokenStored: false } }
        : { status: 200, body: { ...account, tokenStored: true, signedIn: true } };
    });
    const done = jasmine.createSpy('done');
    const el = await render();
    fixture.componentInstance.done.subscribe(done);
    button(el, 'I already have a token').click();
    fixture.detectChanges();
    const field = el.querySelector('input[type="password"]') as HTMLInputElement;
    field.value = TOKEN;
    field.dispatchEvent(new Event('input'));
    fixture.detectChanges();
    button(el, 'Save the token').click();
    // Out of the form before the request even returns.
    expect(field.value).toBe('');
    await flush();
    expect(tokenRoute).toEqual({ token: TOKEN });
    // Nowhere else: not in any URL, nor in any other request.
    expect(backend.calls.some((c) => c.includes(TOKEN))).toBeFalse();
    const others = backend.bodies.filter((b) => b !== tokenRoute);
    expect(JSON.stringify(others)).not.toContain(TOKEN);
    expect(done).toHaveBeenCalled();
  });

  it('runs the sign-in, offers its link and asks for the code when the terminal does', async () => {
    let polls = 0;
    backend.routes.set(PATH, (method, body) => {
      if (method === 'GET') {
        polls += 1;
        return {
          status: 200,
          body: {
            available: true,
            tokenStored: false,
            capture:
              polls > 1
                ? {
                    state: 'running',
                    url: 'https://claude.ai/oauth/authorize?code=true',
                    asksForCode: true,
                    startedAt: 1,
                  }
                : undefined,
          },
        };
      }
      const b = body as { code?: string };
      return {
        status: b.code ? 200 : 202,
        body: {
          capture: { state: 'running', asksForCode: !b.code, startedAt: 1 },
        },
      };
    });
    const el = await render();
    button(el, 'Sign in to get a token').click();
    await flush();
    fixture.detectChanges();
    expect(backend.bodyOf(`POST ${PATH}`)).toEqual({});
    expect(text(el)).toContain('Waiting for the browser…');
    // The next look (every 1.5 s while it runs) brings the link.
    await new Promise((r) => setTimeout(r, 1_600));
    await flush();
    fixture.detectChanges();
    const link = el.querySelector('a[target="_blank"]') as HTMLAnchorElement;
    expect(link.href).toBe('https://claude.ai/oauth/authorize?code=true');
    expect(text(el)).toContain("Browser didn't open? Open this link");
    const code = el.querySelector('input[name="code"]') as HTMLInputElement;
    code.value = 'ABC-123';
    code.dispatchEvent(new Event('input'));
    fixture.detectChanges();
    button(el, 'Send the code').click();
    await flush();
    expect(backend.bodyOf(`POST ${PATH}`)).toEqual({ code: 'ABC-123' });
  });

  it('offers the paste field outright when the sign-in ran out of time', async () => {
    backend.routes.set(PATH, () => ({
      status: 200,
      body: {
        available: true,
        tokenStored: false,
        capture: { state: 'timedOut', asksForCode: false, startedAt: 1, finishedAt: 2 },
      },
    }));
    const el = await render();
    expect(text(el)).toContain('No token arrived within five minutes.');
    expect(el.querySelector('input[type="password"]')).not.toBeNull();
    expect(text(button(el, 'Try again'))).toContain('Try again');
  });

  it("shows the sign-in panel while its token sign-in runs, and waits for another account's", async () => {
    backend.routes.set(PATH, () => ({
      status: 200,
      body: { available: true, tokenStored: false },
    }));
    account = { ...account, signIn: { kind: 'token', state: 'starting', startedAt: 1 } };
    let el = await render();
    expect(el.querySelector('mc-sign-in-panel')).not.toBeNull();
    expect(button(el, 'Sign in to get a token')).toBeUndefined();
    fixture.destroy();

    // Another account's sign-in runs: the button says which, and waits.
    serveAccounts(backend, {
      ...buildAccounts(Date.now(), true),
      accounts: buildAccounts(Date.now(), true).accounts.map((a) =>
        a.isDefault ? { ...a, signIn: { kind: 'login', state: 'open', startedAt: 1 } } : a,
      ),
    });
    account = { ...account, signIn: undefined };
    el = await render();
    const wait = button(el, "Waiting for Default's sign-in");
    expect(wait.disabled).toBeTrue();
    expect(el.querySelector('mc-sign-in-panel')).toBeNull();
  });

  it('says a token sign-in was cancelled or ended without a token', async () => {
    backend.routes.set(PATH, () => ({
      status: 200,
      body: { available: true, tokenStored: false },
    }));
    account = {
      ...account,
      signIn: {
        kind: 'token',
        state: 'failed',
        startedAt: Date.now() - 5_000,
        finishedAt: Date.now(),
        error: 'the browser was closed',
      },
    };
    const el = await render();
    expect(text(el.querySelector('.notice.warn'))).toBe(
      'The sign-in ended without a token: the browser was closed',
    );
  });
});
