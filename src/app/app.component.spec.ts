import { Component } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';
import { page } from 'vitest/browser';

import { AppComponent } from './app.component';
import { JumpService, applePlatform, jumpShortcut } from './core/jump';
import { ListDetailService } from './core/list-detail';
import { hostOsSignal } from './core/i18n/i18n';
import { MachinesService } from './core/machines.service';
import { isStillPending } from './core/pair-requests.service';
import { FakeBackend, asDesktop, flush, useFakeServer } from './core/mock/fake-server.testing';
import { setActiveMock } from './core/mock/mock-api';
import { SessionSummary } from './core/models';
import { PLACES, sidebarPlaces } from './core/nav';
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

  it('draws the sidebar in three groups, each under its heading, and Help at the foot', async () => {
    await render();
    expect(sideLinks().map((a) => text(a))).toEqual(sidebarPlaces(false).map((p) => p.label));
    expect(sideLinks().map((a) => a.getAttribute('href'))).toEqual([
      '/',
      '/inbox',
      '/teams',
      '/computers',
      '/devices',
      '/browser',
      '/http-api',
      '/accounts',
      '/backups',
      '/permissions',
      '/settings',
    ]);
    // A line between two groups; each group names itself (for screen
    // readers too) above its places.
    const nav = el().querySelector('aside .side-nav')!;
    const kids = [...nav.children].map((c) =>
      c.classList.contains('nav-sep')
        ? '—'
        : [
            text(c.querySelector('.nav-heading')),
            ...[...c.querySelectorAll('a')].map((a) => text(a)),
          ],
    );
    expect(kids).toEqual([
      ['Work', 'Sessions', 'Inbox', 'Teams'],
      '—',
      ['Connections', 'Computers', 'Phone & devices', 'Browser', 'HTTP API'],
      '—',
      ['Manage', 'Accounts', 'Backups', 'Permissions', 'Settings'],
    ]);
    for (const group of nav.querySelectorAll('.nav-group')) {
      expect(group.getAttribute('role')).toBe('group');
      const heading = group.querySelector('.nav-heading')!;
      expect(group.getAttribute('aria-labelledby')).toBe(heading.id);
    }
    expect(nav.querySelector('.nav-sep')?.getAttribute('aria-hidden')).toBe('true');
    expect(text(el().querySelector('aside .help-link'))).toBe('Help');
    // The plugin for sessions started elsewhere is the desktop app's.
    asDesktop();
    await settle();
    expect(sideLinks().map((a) => text(a))).toEqual(PLACES.map((p) => p.label));
    expect(sideLinks()[5].getAttribute('href')).toBe('/elsewhere');
    expect(text(nav.querySelectorAll('.nav-group')[1])).toContain('Sessions started elsewhere');
  });

  it('scrolls the places on a short laptop screen, never clipping them or the foot', async () => {
    const before = { width: window.innerWidth, height: window.innerHeight };
    try {
      await page.viewport(1280, 1400);
      await render();
      const side = el().querySelector<HTMLElement>('aside.sidebar')!;
      const nav = side.querySelector<HTMLElement>('.side-nav')!;
      const scrolls = (e: HTMLElement) => e.scrollHeight > e.clientHeight + 1;
      expect(getComputedStyle(side).display).toBe('flex');
      // Tall enough: nothing scrolls.
      expect(scrolls(nav)).toBe(false);
      expect(scrolls(side)).toBe(false);

      // Short: the places scroll between Jump to… and the foot, which stay.
      await page.viewport(1280, 460);
      await settle();
      expect(scrolls(nav)).toBe(true);
      expect(scrolls(side)).toBe(false);
      const jump = side.querySelector('.jump-btn')!.getBoundingClientRect();
      const foot = side.querySelector('.side-foot')!.getBoundingClientRect();
      const box = nav.getBoundingClientRect();
      expect(box.top).toBeGreaterThanOrEqual(jump.bottom - 1);
      expect(box.bottom).toBeLessThanOrEqual(foot.top + 1);
      expect(foot.bottom).toBeLessThanOrEqual(window.innerHeight + 1);
      // The last place is there, scrolled to.
      nav.scrollTop = nav.scrollHeight;
      const last = sideLinks().at(-1)!.getBoundingClientRect();
      expect(last.bottom).toBeLessThanOrEqual(box.bottom + 1);

      // Shorter than even a few places and the foot: the whole sidebar
      // scrolls instead of cutting the foot off.
      await page.viewport(1280, 260);
      await settle();
      expect(nav.clientHeight).toBeGreaterThan(100);
      expect(scrolls(side)).toBe(true);
    } finally {
      await page.viewport(before.width, before.height);
    }
  });

  it("marks the place of the page shown, its tabs' and its own pages too", async () => {
    await render('/agents');
    expect(current(sideLinks())).toEqual(['Sessions']);
    for (const [url, place] of [
      ['/?machine=abc', 'Sessions'],
      ['/session/s1', 'Sessions'],
      ['/new', 'Sessions'],
      ['/projects', 'Sessions'],
      ['/teams/templates', 'Teams'],
      ['/teams/capacity', 'Teams'],
      ['/teams/tm_0123456789ab', 'Teams'],
      ['/computers/setup', 'Computers'],
      ['/devices', 'Phone & devices'],
      ['/browser', 'Browser'],
      ['/http-api?server=work', 'HTTP API'],
      ['/accounts/changes', 'Accounts'],
      ['/backups', 'Backups'],
      ['/permissions?on=others', 'Permissions'],
      ['/settings/tools', 'Settings'],
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
    for (const url of [
      '/computers/options',
      '/devices',
      '/http-api',
      '/accounts/usage',
      '/backups',
      '/settings',
      '/settings/tools',
      '/help',
      '/more',
    ]) {
      await go(url);
      expect(current(barLinks()), url).toEqual(['More']);
    }
    await go('/teams/capacity');
    expect(current(barLinks())).toEqual(['Teams']);
  });

  it("titles the phone's top bar by its place, or the page's own name", async () => {
    await render('/agents');
    const title = () => text(el().querySelector('header.topbar .page-title'));
    expect(title()).toBe('Sessions');
    await go('/projects');
    expect(title()).toBe('Sessions');
    await go('/teams/templates');
    expect(title()).toBe('Teams');
    await go('/computers/sync');
    expect(title()).toBe('Computers');
    await go('/accounts/changes');
    expect(title()).toBe('Accounts');
    await go('/devices');
    expect(title()).toBe('Phone & devices');
    await go('/elsewhere');
    expect(title()).toBe('Sessions started elsewhere');
    await go('/http-api?server=work');
    expect(title()).toBe('HTTP API');
    await go('/backups');
    expect(title()).toBe('Backups');
    await go('/settings/tools');
    expect(title()).toBe('Settings');
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

  it('marks the Inbox while a session shows beside it (wide screens)', async () => {
    await render('/session/s1');
    const ld = TestBed.inject(ListDetailService);
    ld.wide.set(true);
    ld.showList('inbox', '/inbox');
    ld.showDetail({ id: 's1', machine: null, agent: null }, false);
    await settle();
    expect(current(sideLinks())).toEqual(['Inbox']);
    // Narrower, the session takes the page: it is one of Sessions'.
    ld.wide.set(false);
    await settle();
    expect(current(sideLinks())).toEqual(['Sessions']);
  });

  it('opens Jump to… with its key, the sidebar’s button and the top bar’s', async () => {
    await render();
    const jump = TestBed.inject(JumpService);
    const apple = applePlatform();
    const key = new KeyboardEvent('keydown', {
      key: 'k',
      bubbles: true,
      cancelable: true,
      ...(apple ? { metaKey: true } : { ctrlKey: true }),
    });
    document.body.dispatchEvent(key);
    expect(key.defaultPrevented).toBe(true);
    expect(jump.opened()).toBe(true);
    // The same key closes it.
    document.body.dispatchEvent(new KeyboardEvent('keydown', key));
    expect(jump.opened()).toBe(false);
    const side = el().querySelector<HTMLButtonElement>('aside .jump-btn')!;
    expect(text(side.querySelector('.label'))).toBe('Jump to…');
    expect(text(side.querySelector('kbd'))).toBe(jumpShortcut(apple));
    side.click();
    expect(jump.opened()).toBe(true);
    jump.close();
    el().querySelector<HTMLButtonElement>('header.topbar [aria-label="Jump to…"]')!.click();
    expect(jump.opened()).toBe(true);
    jump.close();
  });
});
