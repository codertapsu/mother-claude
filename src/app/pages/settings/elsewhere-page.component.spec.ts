import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import {
  FakeBackend,
  asDesktop,
  asPhoneOf,
  flush,
  serveAccounts,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import { MOCK_ACCOUNT_WORK, buildAccounts } from '../../core/mock/fixtures';
import { setActiveMock } from '../../core/mock/mock-api';
import { ElsewherePageComponent } from './elsewhere-page.component';

/** Sessions started elsewhere (docs/UI-REDESIGN.md §6.7, §11): the plugin in
 * every config directory, in one table. */
describe('ElsewherePageComponent: Sessions started elsewhere', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<ElsewherePageComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    serveAccounts(backend, buildAccounts(Date.now(), true));
    backend.routes.set('/control/foreign', () => ({
      status: 200,
      body: {
        installed: true,
        pluginVersion: '1.4.0',
        cliAvailable: true,
        sessionsWithHooks: 3,
        lastEventAt: Date.now() - 60_000,
        notes: [],
      },
    }));
  });

  afterEach(() => {
    fixture?.destroy();
    setActiveMock(null);
  });

  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const settle = async () => {
    for (let i = 0; i < 4; i++) {
      await flush();
      fixture.detectChanges();
    }
  };
  const render = async () => {
    fixture = TestBed.createComponent(ElsewherePageComponent);
    fixture.detectChanges();
    await settle();
    return fixture.nativeElement as HTMLElement;
  };
  const rows = (el: HTMLElement) => [...el.querySelectorAll('tbody tr')];
  const cells = (row: Element) => [...row.querySelectorAll('th, td')].map((c) => text(c));

  it("lists the Default's plugin and each directory account's, in one table", async () => {
    asDesktop();
    const el = await render();
    // A token account uses the Default's directory: no row of its own.
    expect(rows(el).map((r) => text(r.querySelector('.name')))).toEqual(['Default', 'Work']);
    const [def, work] = rows(el);
    expect(cells(def).slice(1, 3)).toEqual(['On v1.4.0', '3']);
    expect(text(def.querySelectorAll('td')[2])).toContain('ago');
    expect(cells(work).slice(1, 4)).toEqual(['Not installed', '0', 'none yet']);
    expect(text(work.querySelector('.act'))).toBe('Install');
    expect(text(def.querySelector('.act'))).toBe('Uninstall');
  });

  it("installs in a directory account's own directory, and uninstalls the Default's in two taps", async () => {
    asDesktop();
    backend.routes.set(`/accounts/${MOCK_ACCOUNT_WORK}/hooks/install`, () => {
      const view = buildAccounts(Date.now(), true);
      view.accounts = view.accounts.map((a) =>
        a.id === MOCK_ACCOUNT_WORK ? { ...a, hooksPlugin: 'installed' } : a,
      );
      serveAccounts(backend, view);
      return { status: 200, body: view.accounts[1] };
    });
    backend.routes.set('/control/foreign/uninstall', () => ({
      status: 200,
      body: { installed: false, cliAvailable: true, sessionsWithHooks: 0, notes: [] },
    }));
    const el = await render();
    (rows(el)[1].querySelector('.act button') as HTMLButtonElement).click();
    await settle();
    expect(backend.calls).toContain(`POST /accounts/${MOCK_ACCOUNT_WORK}/hooks/install`);
    expect(text(rows(el)[1].querySelectorAll('td')[0])).toBe('On');
    expect(text(el.querySelector('.notice.ok'))).toContain('/reload-plugins');

    const uninstall = () => rows(el)[0].querySelector('.act button') as HTMLButtonElement;
    uninstall().click();
    await settle();
    expect(backend.calls).not.toContain('POST /control/foreign/uninstall');
    expect(text(uninstall())).toBe('Tap again to uninstall');
    uninstall().click();
    await settle();
    expect(backend.calls).toContain('POST /control/foreign/uninstall');
    expect(text(rows(el)[0].querySelectorAll('td')[0])).toBe('Not installed');
  });

  it('reads on a phone, installs on the desktop', async () => {
    asPhoneOf('macos');
    const el = await render();
    expect(rows(el).length).toBe(2);
    expect(el.querySelector('.act')).toBeNull();
    expect(text(el)).toContain('Installed and removed on the desktop app.');
    // The Default's own status is the desktop's to read.
    expect(backend.calls).not.toContain('GET /control/foreign');
  });
});
