import { Component } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';

import { AppComponent } from './app.component';
import { hostOsSignal } from './core/i18n/i18n';
import { MachinesService } from './core/machines.service';
import { isStillPending } from './core/pair-requests.service';
import { FakeBackend, asDesktop, flush, useFakeServer } from './core/mock/fake-server.testing';
import { setActiveMock } from './core/mock/mock-api';
import { SessionSummary } from './core/models';
import { PLACES } from './core/nav';
import { RealtimeService } from './core/realtime.service';
import { UpdaterService } from './core/updater.service';
import { machinesView, peer } from './pages/machines/machines.testing';

describe('isStillPending', () => {
  it('drops expired and settled pair requests', () => {
    const now = 10_000;
    expect(isStillPending({ id: 'a', expiresAt: now + 1 }, now)).toBe(true);
    expect(isStillPending({ id: 'b' }, now)).toBe(true);
    expect(isStillPending({ id: 'c', expiresAt: now }, now)).toBe(false);
    expect(isStillPending({ id: 'd', status: 'denied', expiresAt: now + 1 }, now)).toBe(false);
    expect(isStillPending({ id: 'e', status: 'approved' }, now)).toBe(false);
  });
});

@Component({ template: '' })
class BlankComponent {}

describe('AppComponent: the navigation, from the map', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<AppComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [
      provideRouter([
        { path: '', component: BlankComponent },
        { path: '**', component: BlankComponent },
      ]),
    ]);
  });

  afterEach(() => {
    fixture?.destroy();
    TestBed.inject(RealtimeService).stop();
    setActiveMock(null);
    // The shell's I18nService names the desktop's system for every module:
    // other specs expect it unknown.
    hostOsSignal.set(undefined);
  });

  const text = (e: Element | null | undefined) =>
    (e?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const settle = async () => {
    for (let i = 0; i < 3; i++) {
      await flush();
      fixture.detectChanges();
    }
  };
  const render = async (url = '/') => {
    fixture = TestBed.createComponent(AppComponent);
    fixture.detectChanges();
    await settle();
    await go(url);
    return fixture.nativeElement as HTMLElement;
  };
  const go = async (url: string) => {
    await TestBed.inject(Router).navigateByUrl(url);
    await settle();
  };
  const el = () => fixture.nativeElement as HTMLElement;
  const sideLinks = () => [...el().querySelectorAll<HTMLAnchorElement>('aside .side-nav a')];
  const barLinks = () => [...el().querySelectorAll<HTMLAnchorElement>('nav.tabbar a')];
  const current = (links: HTMLAnchorElement[]) =>
    links.filter((a) => a.getAttribute('aria-current') === 'page').map((a) => text(a));
  const store = (list: Partial<SessionSummary>[]) =>
    (
      TestBed.inject(RealtimeService) as unknown as { replaceStore(l: unknown[]): void }
    ).replaceStore(list);

  it('draws the sidebar: the work, a line, what you look after, and Help at the foot', async () => {
    await render();
    expect(sideLinks().map((a) => text(a))).toEqual(PLACES.map((p) => p.label));
    expect(sideLinks().map((a) => a.getAttribute('href'))).toEqual([
      '/',
      '/inbox',
      '/teams',
      '/machines',
      '/accounts',
      '/settings',
    ]);
    // One line, between Teams and Computers; no group headings.
    const nav = el().querySelector('aside .side-nav')!;
    const kids = [...nav.children].map((c) => (c.classList.contains('nav-sep') ? '—' : text(c)));
    expect(kids).toEqual(['Sessions', 'Inbox', 'Teams', '—', 'Computers', 'Accounts', 'Settings']);
    expect(el().querySelector('.nav-group')).toBeNull();
    expect(text(el().querySelector('aside .help-link'))).toBe('Help');
  });

  it("marks the place of the page shown, its tabs' and connections' pages too", async () => {
    await render('/agents');
    expect(current(sideLinks())).toEqual(['Sessions']);
    for (const [url, place] of [
      ['/?machine=abc', 'Sessions'],
      ['/session/s1', 'Sessions'],
      ['/new', 'Sessions'],
      ['/capacity', 'Teams'],
      ['/teams/tm_0123456789ab', 'Teams'],
      ['/machines/setup', 'Computers'],
      ['/backup', 'Accounts'],
      ['/remote', 'Settings'],
      ['/services', 'Settings'],
    ]) {
      await go(url);
      expect(current(sideLinks()), url).toEqual([place]);
    }
    await go('/help/phone');
    expect(current(sideLinks())).toEqual([]);
    expect(el().querySelector('aside .help-link')?.getAttribute('aria-current')).toBe('page');
  });

  it("draws the phone's bar: the work, then More for the rest", async () => {
    await render('/teams');
    expect(barLinks().map((a) => text(a))).toEqual(['Sessions', 'Inbox', 'Teams', 'More']);
    expect(barLinks().map((a) => a.getAttribute('href'))).toEqual([
      '/',
      '/inbox',
      '/teams',
      '/more',
    ]);
    expect(current(barLinks())).toEqual(['Teams']);
    for (const url of ['/machines', '/backup', '/settings', '/api', '/help', '/more']) {
      await go(url);
      expect(current(barLinks()), url).toEqual(['More']);
    }
    await go('/capacity');
    expect(current(barLinks())).toEqual(['Teams']);
  });

  it("titles the phone's top bar by its place, or the page's own name", async () => {
    await render('/agents');
    const title = () => text(el().querySelector('header.topbar .page-title'));
    expect(title()).toBe('Sessions');
    await go('/machines');
    expect(title()).toBe('Computers');
    await go('/backup');
    expect(title()).toBe('Accounts');
    await go('/remote');
    expect(title()).toBe('Remote');
  });

  it("counts what waits on the Inbox, and shows Computers' health dot", async () => {
    await render();
    store([
      {
        id: 's1',
        live: true,
        pending: [
          {
            id: 'p1',
            kind: 'permission',
            title: 'Allow Bash?',
            dangerous: false,
            answerable: true,
            createdAt: Date.now(),
          },
        ],
      },
    ]);
    await settle();
    const inbox = sideLinks()[1];
    expect(text(inbox.querySelector('.count-badge'))).toBe('1');
    expect(inbox.getAttribute('aria-label')).toBe('Inbox, 1 waiting');
    expect(text(barLinks()[1].querySelector('.count-badge'))).toBe('1');

    const computers = () => sideLinks()[3];
    expect(computers().querySelector('.count-badge.dot')).toBeNull();
    asDesktop();
    TestBed.inject(MachinesService).apply(
      machinesView({
        enabled: true,
        peers: [peer()],
        health: { state: 'blocker', blockers: 1, warnings: 0, at: Date.now() },
      }),
    );
    await settle();
    expect(computers().querySelector('.count-badge.dot')).not.toBeNull();
    expect(computers().getAttribute('aria-label')).toBe('Computers, needs attention');
  });

  it('puts a dot on the Inbox when only something to fix waits', async () => {
    await render();
    const inbox = () => sideLinks()[1];
    expect(inbox().querySelector('.count-badge')).toBeNull();
    asDesktop();
    TestBed.inject(UpdaterService).status.set('available');
    TestBed.inject(UpdaterService).newVersion.set('0.12.0');
    await settle();
    expect(inbox().querySelector('.count-badge.dot')).not.toBeNull();
    expect(inbox().getAttribute('aria-label')).toBe('Inbox, 1 to fix');
    expect(barLinks()[1].querySelector('.count-badge.dot')).not.toBeNull();
    // A request waiting shows its number instead.
    store([
      {
        id: 's1',
        live: true,
        pending: [
          {
            id: 'p1',
            kind: 'permission',
            title: 'Allow Bash?',
            dangerous: false,
            answerable: true,
            createdAt: Date.now(),
          },
        ],
      },
    ]);
    await settle();
    expect(text(inbox().querySelector('.count-badge'))).toBe('1');
    expect(inbox().querySelector('.count-badge.dot')).toBeNull();
    expect(inbox().getAttribute('aria-label')).toBe('Inbox, 1 waiting, 1 to fix');
  });
});
