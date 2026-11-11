import { Component } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';
import { RouterTestingHarness } from '@angular/router/testing';

import { FakeBackend, flush, useFakeServer } from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import {
  PEER2_ID,
  PEER_ID,
  ask,
  goLive,
  pairedMachines,
  remote,
  remoteList,
  remoteSession,
} from '../../core/mock/remote.testing';
import { storageRemove } from '../../core/platform';
import { RealtimeService } from '../../core/realtime.service';
import { pastClock } from '../../shared/util';
import { SessionsComponent } from './sessions.component';

@Component({ template: '' })
class BlankComponent {}

describe("SessionsComponent with other computers' sessions", () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<SessionsComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [
      provideRouter([
        { path: '', component: SessionsComponent },
        { path: 'session/:id/on/:machine', component: BlankComponent },
        { path: 'session/:id', component: BlankComponent },
        { path: 'new', component: BlankComponent },
      ]),
    ]);
  });

  afterEach(() => {
    fixture?.destroy();
    TestBed.inject(RealtimeService).stop();
    setActiveMock(null);
    storageRemove('mc_sessions_view');
    storageRemove('mc_sessions_filter');
  });

  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const render = async () => {
    fixture = TestBed.createComponent(SessionsComponent);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    return fixture.nativeElement as HTMLElement;
  };
  const cards = (el: HTMLElement) => [...el.querySelectorAll('mc-session-card')];
  const names = (el: HTMLElement) => cards(el).map((c) => text(c.querySelector('.name')));
  const card = (el: HTMLElement, name: string) =>
    cards(el).find((c) => text(c.querySelector('.name')) === name);
  const pick = (el: HTMLElement, value: string) => {
    const sel = el.querySelector('select[aria-label="Machine"]') as HTMLSelectElement;
    sel.value = value;
    sel.dispatchEvent(new Event('change'));
    fixture.detectChanges();
  };

  /** This computer: one idle session. DESKTOP-PC: one at work. BUILD-BOX:
   * offline since `since`, with one session as last heard. */
  const threeComputers = async (since: number) => {
    const s = await goLive(backend);
    s.frame({
      t: 'sessions',
      full: true,
      sessions: [remoteSession('here-1', { title: 'Release notes', status: 'idle' })],
      seq: 1,
    });
    pairedMachines(true);
    remoteList(s, PEER_ID, [
      remoteSession('r1', { title: 'Fix tray icon', status: 'waiting', pending: [ask('q1')] }),
    ]);
    remoteList(s, PEER2_ID, [remoteSession('r2', { title: 'Build installer' })]);
    remote(s, PEER2_ID, { t: 'offline', since });
    return s;
  };

  it("shows another computer's sessions with its chip, and none on this one's", async () => {
    const since = new Date().setHours(14, 32, 0, 0);
    await threeComputers(since);
    const el = await render();

    expect(card(el, 'Release notes')?.querySelector('.mchip')).toBeNull();
    const there = card(el, 'Fix tray icon');
    expect(text(there?.querySelector('.mchip'))).toBe('DESKTOP-PC');
    expect(there?.querySelector('a.name')?.getAttribute('href')).toBe(`/session/r1/on/${PEER_ID}`);
    // Its request counts in "Needs you", and is answered through it.
    expect(text(el.querySelector('.n.warn'))).toBe('1');
    expect(there?.querySelector('mc-pending-card')).not.toBeNull();
  });

  it("greys an offline computer's sessions, saying since when, in a group of their own", async () => {
    const since = new Date().setHours(14, 32, 0, 0);
    await threeComputers(since);
    const el = await render();

    const off = card(el, 'Build installer');
    expect(off?.classList.contains('offline')).toBe(true);
    expect(text(off?.querySelector('.mchip'))).toBe('BUILD-BOX');
    expect(text(off?.querySelector('.offline-line'))).toBe(
      `BUILD-BOX is offline since ${pastClock(since)}`,
    );
    // Shown as last heard: no activity, no quick answers.
    expect(off?.querySelector('mc-activity-line')).toBeNull();
    const group = off?.closest('section.group');
    expect(text(group?.querySelector('h2'))).toBe('On computers that are offline 1');
    expect(card(el, 'Fix tray icon')?.classList.contains('offline')).toBe(false);
  });

  it('filters by computer', async () => {
    await threeComputers(Date.now());
    const el = await render();
    const sel = el.querySelector('select[aria-label="Machine"]') as HTMLSelectElement;
    expect([...sel.options].map((o) => text(o))).toEqual([
      'All machines',
      'This computer',
      'BUILD-BOX (offline)',
      'DESKTOP-PC',
    ]);
    expect(names(el).sort()).toEqual(['Build installer', 'Fix tray icon', 'Release notes']);

    pick(el, PEER_ID);
    expect(names(el)).toEqual(['Fix tray icon']);
    // Earlier sessions are this computer's: not offered under another one.
    expect(el.querySelector('section.history')).toBeNull();

    pick(el, 'here');
    expect(names(el)).toEqual(['Release notes']);
    expect(el.querySelector('section.history')).not.toBeNull();

    pick(el, '');
    expect(names(el).length).toBe(3);
  });

  it('says a computer is still sending its sessions', async () => {
    const s = await goLive(backend);
    s.frame({ t: 'sessions', full: true, sessions: [], seq: 1 });
    pairedMachines();
    remote(s, PEER_ID, { t: 'online' });
    const el = await render();
    expect(text(el.querySelector('.machine-loading'))).toBe("Loading DESKTOP-PC's sessions…");
    remoteList(s, PEER_ID, [remoteSession('r1', { title: 'Fix tray icon' })]);
    fixture.detectChanges();
    expect(el.querySelector('.machine-loading')).toBeNull();
    expect(names(el)).toEqual(['Fix tray icon']);
  });

  it('is only this computer with no other computer paired', async () => {
    const s = await goLive(backend);
    s.frame({
      t: 'sessions',
      full: true,
      sessions: [remoteSession('here-1', { title: 'Release notes' })],
      seq: 1,
    });
    const el = await render();
    expect(el.querySelector('select[aria-label="Machine"]')).toBeNull();
    expect(el.querySelector('.mchip')).toBeNull();
    expect(el.querySelector('.machine-loading')).toBeNull();
    expect(backend.calls.filter((c) => c.includes('/machines'))).toEqual([]);
  });

  it("opens on one computer's sessions from its Machines card (?machine=)", async () => {
    await threeComputers(Date.now());
    const harness = await RouterTestingHarness.create();
    await harness.navigateByUrl(`/?machine=${PEER_ID}`);
    await flush();
    harness.detectChanges();
    const el = harness.routeNativeElement as HTMLElement;
    expect([...el.querySelectorAll('mc-session-card .name')].map((n) => text(n))).toEqual([
      'Fix tray icon',
    ]);
    expect(TestBed.inject(Router).url).toBe(`/?machine=${PEER_ID}`);
  });
});
