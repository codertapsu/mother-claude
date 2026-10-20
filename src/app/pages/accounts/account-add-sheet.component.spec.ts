import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import {
  FakeBackend,
  asDesktop,
  flush,
  serveAccounts,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import { accountFixture, buildAccounts } from '../../core/mock/fixtures';
import { setActiveMock } from '../../core/mock/mock-api';
import { AccountsView } from '../../core/models';
import { AccountAddSheetComponent } from './account-add-sheet.component';

const TOKEN = 'sk-ant-oat01-abcdefghijklmnop';

describe('AccountAddSheetComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<AccountAddSheetComponent>;
  let view: AccountsView;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    asDesktop();
    view = buildAccounts(Date.now(), true);
    serveAccounts(backend, view);
    backend.routes.set('/accounts', (method, body) => {
      if (method !== 'POST') return { status: 200, body: view };
      const b = body as { name: string; kind: 'directory' | 'token'; token?: string };
      const created = accountFixture({
        id: 'acc_00000000000a',
        name: b.name,
        kind: b.kind,
        signedIn: !!b.token,
        tokenStored: !!b.token,
        ...(b.kind === 'token' && !b.token ? { attention: 'tokenMissing' as const } : {}),
      });
      view = { ...view, accounts: [...view.accounts, created] };
      return { status: 201, body: created };
    });
  });

  afterEach(() => {
    fixture?.destroy();
    setActiveMock(null);
  });

  const render = async () => {
    fixture = TestBed.createComponent(AccountAddSheetComponent);
    fixture.componentRef.setInput('open', true);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    return fixture.nativeElement as HTMLElement;
  };
  const text = (el: Element | null) => (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const button = (el: HTMLElement, label: string) =>
    [...el.querySelectorAll('button')].find((b) => text(b).includes(label)) as HTMLButtonElement;
  const type = async (el: HTMLElement, selector: string, value: string) => {
    const input = el.querySelector(selector) as HTMLInputElement;
    input.value = value;
    input.dispatchEvent(new Event('input'));
    fixture.detectChanges();
    await fixture.whenStable();
    return input;
  };
  /** Click, then let the step settle: a form's controls wire up a tick later. */
  const click = async (b: HTMLButtonElement) => {
    b.click();
    fixture.detectChanges();
    await fixture.whenStable();
    fixture.detectChanges();
  };
  const submit = async (el: HTMLElement) => {
    (el.querySelector('form button[type="submit"]') as HTMLButtonElement).click();
    await flush();
    fixture.detectChanges();
  };

  it('adds a token account with a pasted token, which travels only in that body', async () => {
    const el = await render();
    expect(text(el)).toContain('Shared data, another account');
    expect(text(el)).toContain('kept in the macOS Keychain');
    await click(button(el, 'Next'));
    await type(el, 'input[name="name"]', 'Side project');
    await click(button(el, 'I already have a token'));
    const field = await type(el, 'input[type="password"]', TOKEN);
    await submit(el);
    expect(backend.bodyOf('POST /accounts')).toEqual({
      name: 'Side project',
      kind: 'token',
      token: TOKEN,
      // The next colour no account has yet.
      color: '#10b981',
    });
    expect(field.value).toBe('');
    expect(backend.calls.some((c) => c.includes(TOKEN))).toBeFalse();
    expect(text(el)).toContain('Side project is ready');
  });

  it('adds a directory account in a folder named after it, then offers its sign-in', async () => {
    const el = await render();
    button(el, 'Separate directory').click();
    await click(button(el, 'Next'));
    await type(el, 'input[name="name"]', 'Client X');
    expect((el.querySelector('input[name="dir"]') as HTMLInputElement).value).toBe(
      '~/.claude-client-x',
    );
    await submit(el);
    expect(backend.bodyOf('POST /accounts')).toEqual({
      name: 'Client X',
      kind: 'directory',
      dir: '~/.claude-client-x',
      color: '#10b981',
    });
    expect(text(el)).toContain('Added Client X');
    expect(button(el, 'Sign in now')).toBeTruthy();
  });

  it('refuses a name another account has, without asking the server', async () => {
    const el = await render();
    await click(button(el, 'Next'));
    await type(el, 'input[name="name"]', 'work');
    expect(text(el)).toContain('Another account already has this name.');
    await submit(el);
    expect(backend.calls).not.toContain('POST /accounts');
  });
});
