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
    // Nothing to choose between yet, and nowhere to move data to.
    expect(text(el)).not.toContain('New sessions run under');
    expect(buttons(el).some((b) => b.startsWith('Move or copy'))).toBeFalse();
    expect(text(el)).toContain('Account tokens are kept in the macOS Keychain');
  });

  it('lets a phone rename accounts and choose the defaults; adding and signing in stay on the Mac', async () => {
    serveAccounts(backend, buildAccounts(Date.now(), true));
    const el = await render();
    const rows = [...el.querySelectorAll('.acct')];
    expect(rows.map((r) => text(r.querySelector('.aname')))).toEqual([
      'Default',
      'Work',
      'Marcus (Pro)',
    ]);
    expect(text(rows[2])).toContain("Token, shares the Default's data");
    expect(text(el.querySelector('.phone-note'))).toContain(
      'added, signed in and removed on the Mac',
    );
    const labels = buttons(el);
    expect(labels).not.toContain('Add account');
    expect(labels.some((b) => b.startsWith('Sign in'))).toBeFalse();
    // Manage: the name and colour (docs/ACCOUNTS.md §13.9).
    expect(labels.filter((b) => b.includes('Manage')).length).toBe(3);
    expect((el.querySelector('#acc-new-session') as HTMLSelectElement).value).toBe('default');
    expect((el.querySelector('#acc-bridge') as HTMLSelectElement).value).toBe('default');
    expect(text(el.querySelector('.prow .pdir'))).toBe('~/dev/analytics');
    expect((el.querySelector('.prow .psel') as HTMLSelectElement).value).toBe(MOCK_ACCOUNT_WORK);
    expect(el.querySelector('form.padd')).not.toBeNull();
  });

  it("lets a phone rename an account, and says the rest is the Mac's", async () => {
    const view = buildAccounts(Date.now(), true);
    serveAccounts(backend, view);
    backend.routes.set(`/accounts/${MOCK_ACCOUNT_WORK}`, (_m, body) => ({
      status: 200,
      body: { ...view.accounts[1], ...(body as object) },
    }));
    const el = await render();
    el.querySelector<HTMLButtonElement>('button[aria-label="Manage Work"]')!.click();
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    const sheet = el.querySelector('mc-account-sheet') as HTMLElement;
    expect(sheet.querySelector('mc-account-signin')).toBeNull();
    expect(text(sheet.querySelector('.onmac'))).toContain('changed on the Mac');
    expect(buttons(sheet).some((b) => b.startsWith('Remove'))).toBeFalse();
    const name = sheet.querySelector<HTMLInputElement>('input[name="name"]')!;
    name.value = 'Acme';
    name.dispatchEvent(new Event('input'));
    fixture.detectChanges();
    (sheet.querySelector('form.look button[type="submit"]') as HTMLButtonElement).click();
    await flush();
    expect(backend.bodyOf(`PATCH /accounts/${MOCK_ACCOUNT_WORK}`)).toEqual({ name: 'Acme' });
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

  it("installs and removes the hooks plugin in a directory account's own directory", async () => {
    asDesktop();
    serveAccounts(backend, buildAccounts(Date.now(), true));
    backend.routes.set(`/accounts/${MOCK_ACCOUNT_WORK}/hooks/install`, () => {
      const view = buildAccounts(Date.now(), true);
      view.accounts = view.accounts.map((a) =>
        a.id === MOCK_ACCOUNT_WORK ? { ...a, hooksPlugin: 'installed' } : a,
      );
      serveAccounts(backend, view);
      return { status: 200, body: view.accounts[1] };
    });
    const el = await render();
    const rows = [...el.querySelectorAll('.acct')];
    // The Default's plugin is foreign control's; a token account has none.
    expect(text(rows[0])).toContain('Hooks plugin installed');
    expect(rows[0].querySelector('.hooks')).toBeNull();
    expect(text(rows[2])).not.toContain('Hooks plugin');
    expect(text(rows[1])).toContain('Hooks plugin not installed');
    const install = rows[1].querySelector('.hooks .btn') as HTMLButtonElement;
    expect(text(install)).toBe('Install hooks plugin');
    install.click();
    await flush();
    fixture.detectChanges();
    expect(backend.calls).toContain(`POST /accounts/${MOCK_ACCOUNT_WORK}/hooks/install`);
    const after = el.querySelectorAll('.acct')[1];
    expect(text(after)).toContain('Hooks plugin installed');
    expect(text(after.querySelector('.hooks'))).toBe('Uninstall');
    expect(text(el.querySelector('.notice.ok'))).toContain('/reload-plugins');
  });

  it('lets the desktop allow copies between two accounts ahead of time', async () => {
    asDesktop();
    serveAccounts(backend, buildAccounts(Date.now(), true));
    backend.routes.set('/accounts/copy-consent', (_m, body) => ({
      status: 200,
      body: { ...buildAccounts(Date.now(), true), copyConsent: [body] },
    }));
    const el = await render();
    const sel = el.querySelector('select[name="pair"]') as HTMLSelectElement;
    // Only pairs in different directories: the token account shares the Default's.
    expect([...sel.options].map((o) => o.text.trim())).toEqual([
      'Default → Work',
      'Work → Default',
      'Work → Marcus (Pro)',
      'Marcus (Pro) → Work',
    ]);
    sel.value = sel.options[1].value;
    sel.dispatchEvent(new Event('change'));
    fixture.detectChanges();
    const forms = [...el.querySelectorAll('form.padd')];
    const consentForm = forms.find((f) =>
      f.querySelector('select[name="pair"]'),
    ) as HTMLFormElement;
    (consentForm.querySelector('button[type="submit"]') as HTMLButtonElement).click();
    await flush();
    expect(backend.bodyOf('PUT /accounts/copy-consent')).toEqual({
      from: MOCK_ACCOUNT_WORK,
      to: 'default',
      granted: true,
    });
  });

  it('opens Move data with everything of an account', async () => {
    asDesktop();
    serveAccounts(backend, buildAccounts(Date.now(), true));
    backend.routes.set('/backup', () => ({ status: 200, body: { settings: { folder: null } } }));
    const el = await render();
    el.querySelector<HTMLButtonElement>(
      'button[aria-label="Move or copy the data of Work"]',
    )!.click();
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    const sheet = el.querySelector('mc-move-data-sheet') as HTMLElement;
    expect(sheet.querySelector<HTMLSelectElement>('select[name="from"]')!.value).toBe(
      `a:${MOCK_ACCOUNT_WORK}`,
    );
    expect(text(sheet.querySelector('.whatseg [aria-pressed="true"]'))).toBe('Everything');
    expect(text(sheet.querySelector('.allline'))).toBe(
      'Every conversation of Work, and its prompt history.',
    );
  });

  it('shows a phone no hooks plugin buttons, and lets it allow copies', async () => {
    serveAccounts(backend, buildAccounts(Date.now(), true));
    const el = await render();
    expect(el.querySelector('.hooks')).toBeNull();
    expect(el.querySelector('select[name="pair"]')).not.toBeNull();
  });

  it('keeps the Undo of recent moves and copies at hand', async () => {
    asDesktop();
    serveAccounts(backend, buildAccounts(Date.now(), true));
    const job = (id: string, kind: string, over: Record<string, unknown> = {}) => ({
      id,
      kind,
      state: 'done',
      startedAt: Date.now() - 60_000,
      finishedAt: Date.now() - 50_000,
      filesDone: 3,
      filesTotal: 3,
      bytesDone: 10,
      bytesTotal: 10,
      sessions: ['s1'],
      undoUntil: Date.now() + 86_400_000,
      ...over,
    });
    backend.routes.set('/data/jobs', () => ({
      status: 200,
      body: {
        jobs: [
          job('j_backup', 'backup'),
          job('j_copy', 'copy'),
          job('j_move', 'move', {
            startedAt: Date.now() - 10_000,
            state: 'undone',
            undoUntil: undefined,
          }),
        ],
      },
    }));
    const el = await render();
    const rows = [...el.querySelectorAll('.moves mc-data-job')];
    // Backups live on the Backup & sync page; the newest first.
    expect(rows.map((r) => text(r.querySelector('.jtitle')))).toEqual(['Move', 'Copy']);
    expect(text(rows[0])).toContain('Undone');
    expect(rows[0].querySelector('.undo')).toBeNull();
    expect(text(rows[1])).toContain('Copied 1 session.');
    expect(text(rows[1].querySelector('.undo .btn'))).toBe('Undo');
  });

  it('shows a running sign-in on its row, and the others wait for it', async () => {
    asDesktop();
    const view = buildAccounts(Date.now(), true);
    view.accounts = view.accounts.map((a) =>
      a.id === MOCK_ACCOUNT_WORK
        ? {
            ...a,
            signedIn: false,
            attention: 'signedOut' as const,
            signIn: { kind: 'login' as const, state: 'open' as const, startedAt: 1 },
          }
        : a.id === MOCK_ACCOUNT_PRO
          ? { ...a, tokenStored: false, attention: 'tokenMissing' as const }
          : a,
    );
    serveAccounts(backend, view);
    const el = await render();
    const rows = [...el.querySelectorAll('.acct')];
    const showSignIn = rows[1].querySelector<HTMLButtonElement>(
      'button[aria-label="Show the sign-in of Work"]',
    );
    expect(text(showSignIn)).toBe('Signing in…');
    // One sign-in at a time: Pro's token waits for Work's.
    const proButton = [...rows[2].querySelectorAll<HTMLButtonElement>('button.primary')][0];
    expect(text(proButton)).toBe("Waiting for Work's sign-in");
    expect(proButton.disabled).toBeTrue();
    showSignIn?.click();
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    expect(el.querySelector('mc-account-sheet mc-sign-in-panel')).not.toBeNull();
  });

  it('warns about a changed identity on its row, and keeps it on asking', async () => {
    asDesktop();
    const view = buildAccounts(Date.now(), true);
    view.accounts = view.accounts.map((a) =>
      a.id === MOCK_ACCOUNT_WORK ? { ...a, identityWarning: { kind: 'changed' as const } } : a,
    );
    serveAccounts(backend, view);
    backend.routes.set(`/accounts/${MOCK_ACCOUNT_WORK}/identity/accept`, () => ({
      status: 200,
      body: {},
    }));
    const el = await render();
    const row = [...el.querySelectorAll('.acct')][1];
    expect(text(row.querySelector('mc-identity-warning'))).toContain(
      'It signed in as a different Claude account than before.',
    );
    [...row.querySelectorAll<HTMLButtonElement>('mc-identity-warning button')]
      .find((b) => text(b) === 'Keep')
      ?.click();
    await flush();
    expect(backend.calls).toContain(`POST /accounts/${MOCK_ACCOUNT_WORK}/identity/accept`);
  });
});
