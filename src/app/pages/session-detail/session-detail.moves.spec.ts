import { Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { RouterTestingHarness } from '@angular/router/testing';

import { MachinesService } from '../../core/machines.service';
import {
  FakeBackend,
  FakeSocket,
  asDesktop,
  flush,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { MOVE_ID, movedTo, moveRecord, preflightOk } from '../../core/mock/moves.testing';
import {
  PEER_ID,
  goLive,
  pairedMachines,
  remote,
  remoteList,
  remoteSession,
} from '../../core/mock/remote.testing';
import { SessionSummary } from '../../core/models';
import { NoticeService } from '../../core/notice.service';
import { RealtimeService } from '../../core/realtime.service';
import { SELF_ID, machinesView } from '../machines/machines.testing';
import { RemoteSessionComponent } from './remote-session.component';
import { SessionDetailComponent } from './session-detail.component';

@Component({ template: '' })
class BlankComponent {}

const PROXY = `/machines/${PEER_ID}/proxy`;
const SINCE = new Date().setHours(9, 5, 0, 0);

/** `l1` on DESKTOP-PC, where it lives now: idle between turns. */
const there = () =>
  remoteSession('l1', {
    title: 'Fix tray icon',
    status: 'idle',
    activity: { kind: 'idle', label: 'Idle' },
  });

/** This computer's copy of `l1`, which moved to DESKTOP-PC. */
const moved = (over: Partial<SessionSummary> = {}): SessionSummary =>
  remoteSession('l1', {
    title: 'Fix tray icon',
    live: false,
    status: 'ended',
    activity: { kind: 'ended', label: 'Ended' },
    livesOn: { machine: PEER_ID, name: 'DESKTOP-PC', since: SINCE, epoch: 2 },
    ...over,
  });

describe('a session page and moves between computers', () => {
  let backend: FakeBackend;
  let harness: RouterTestingHarness;
  let socket: FakeSocket;

  beforeEach(async () => {
    backend = new FakeBackend();
    useFakeServer(backend, [
      provideRouter([
        { path: 'session/:id', component: SessionDetailComponent },
        { path: 'session/:id/on/:machine', component: RemoteSessionComponent },
        { path: '', component: BlankComponent },
      ]),
    ]);
    asDesktop();
    pairedMachines();
    backend.routes.set('/machines', () => ({
      status: 200,
      body: TestBed.inject(MachinesService).view() ?? machinesView(),
    }));
    backend.routes.set('/moves', () => ({ status: 200, body: { moves: [] } }));
    socket = await goLive(backend);
    harness = await RouterTestingHarness.create();
  });

  afterEach(() => {
    TestBed.inject(RealtimeService).stop();
    setActiveMock(null);
  });

  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const settle = async () => {
    for (let i = 0; i < 4; i++) {
      await flush();
      harness.detectChanges();
    }
  };
  /** This computer's sessions, the page's detail and transcript. */
  const here = (s: SessionSummary) => {
    socket.frame({ t: 'sessions', full: true, sessions: [s], seq: 1 });
    backend.routes.set(`/sessions/${s.id}`, () => ({
      status: 200,
      body: { ...s, pendingDetail: [] },
    }));
    backend.routes.set(`/sessions/${s.id}/transcript`, () => ({
      status: 200,
      body: { events: [], tail: '0' },
    }));
  };
  const open = async (url: string) => {
    await harness.navigateByUrl(url);
    await settle();
    return harness.routeNativeElement as HTMLElement;
  };
  const button = (el: ParentNode, label: string) =>
    [...el.querySelectorAll<HTMLButtonElement>('button')].find((b) => text(b).startsWith(label));
  const menu = (el: HTMLElement) => {
    el.querySelector<HTMLButtonElement>('[aria-label="Session menu"]')?.click();
    harness.detectChanges();
    return text(document.querySelector('mc-session-header .menu'));
  };

  it('offers Continue on another computer for a session of this computer', async () => {
    here(remoteSession('l1', { title: 'Fix tray icon', status: 'idle' }));
    const el = await open('/session/l1');
    const items = menu(el);
    expect(items).toContain('Continue on another computer…');
    expect(items).not.toContain('Bring back');
  });

  it('says where a session that moved away lives, and holds its composer', async () => {
    here(moved());
    remoteList(socket, PEER_ID, [there()]);
    const el = await open('/session/l1');

    const banner = el.querySelector('mc-lives-on-banner');
    expect(text(banner?.querySelector('.line'))).toMatch(
      /^Lives on DESKTOP-PC since .+ — moved from this computer\.$/,
    );
    expect(banner?.querySelector('a')?.getAttribute('href')).toBe(`/session/l1/on/${PEER_ID}`);
    expect(button(banner!, 'Bring it back here')).toBeTruthy();
    // Take over only while DESKTOP-PC is offline.
    expect(button(banner!.querySelector('.acts')!, 'Take over')).toBeUndefined();
    expect(text(el.querySelector('mc-composer .blocked'))).toBe(
      'This session lives on DESKTOP-PC now: open it there, or bring it back here.',
    );
    expect(el.querySelector('mc-composer textarea')).toBeNull();
    // It moves from where it lives: no move away from here.
    const items = menu(el);
    expect(items).toContain('Bring back to this computer');
    expect(items).not.toContain('Continue on another computer…');
  });

  it('brings it back through the computer it lives on, and follows the move there', async () => {
    here(moved());
    remoteList(socket, PEER_ID, [there()]);
    const back = () =>
      movedTo('sending', {
        session: 'l1',
        from: { id: PEER_ID, name: 'DESKTOP-PC', os: 'windows' },
        to: { id: SELF_ID, name: 'MacBook', os: 'macos' },
      });
    backend.routes.set(`${PROXY}/sessions/l1/move/preflight`, () => ({
      status: 200,
      body: preflightOk({ to: { id: SELF_ID, name: 'MacBook', os: 'macos', online: true } }),
    }));
    backend.routes.set(`${PROXY}/sessions/l1/move`, () => ({
      status: 202,
      body: moveRecord({
        session: 'l1',
        from: { id: PEER_ID, name: 'DESKTOP-PC', os: 'windows' },
        to: { id: SELF_ID, name: 'MacBook', os: 'macos' },
      }),
    }));
    let polled = 0;
    backend.routes.set(`${PROXY}/moves/${MOVE_ID}`, () => {
      polled += 1;
      return { status: 200, body: back() };
    });
    const el = await open('/session/l1');
    button(el.querySelector('mc-lives-on-banner')!, 'Bring it back here')?.click();
    await settle();

    const sheet = el.querySelector('mc-session-header mc-move-session-sheet')!;
    expect(text(sheet.querySelector('.title'))).toBe('Bring back to this computer');
    expect(backend.bodyOf(`POST ${PROXY}/sessions/l1/move/preflight`)).toEqual({ to: SELF_ID });
    button(sheet, 'Bring back here')?.click();
    await settle();
    expect(backend.bodyOf(`POST ${PROXY}/sessions/l1/move`)).toEqual({
      to: SELF_ID,
      code: 'bundle',
      apply: 'checkout',
      keepDriving: false,
    });
    // Nothing of it ran through this computer's own move routes.
    expect(backend.calls.filter((c) => c.startsWith('POST /sessions/l1/move'))).toEqual([]);

    // DESKTOP-PC passes no move frames: the sheet asks about it.
    await new Promise((r) => setTimeout(r, 1700));
    await settle();
    expect(polled).toBeGreaterThan(0);
    expect(text(sheet.querySelector('.outcome'))).toBe('Sending to this computer: 0 B of 20 MB');
    expect(text(el.querySelector('mc-move-line .txt'))).toBe(
      'Sending to this computer: 0 B of 20 MB',
    );
    expect(text(el.querySelector('mc-composer .blocked'))).toBe(
      'Coming back to this computer: it takes messages once it is here.',
    );
    // While it moves, the banner offers no second move.
    expect(button(el.querySelector('mc-lives-on-banner')!, 'Bring it back here')).toBeUndefined();
  });

  it('takes it over only while the computer it lives on is offline', async () => {
    here(moved());
    remote(socket, PEER_ID, { t: 'offline', since: Date.now() - 600_000 });
    backend.routes.set('/sessions/l1/move', () => ({
      status: 200,
      body: {
        takenOver: true,
        session: 'l1',
        home: SELF_ID,
        from: PEER_ID,
        epoch: 3,
        warning:
          "It goes on here from this computer's copy. If DESKTOP-PC went on with it while it was away, both versions are kept side by side when it next moves there.",
      },
    }));
    const el = await open('/session/l1');
    const banner = el.querySelector('mc-lives-on-banner')!;
    expect(text(banner.querySelector('.off'))).toMatch(/^DESKTOP-PC is offline since .+\.$/);
    expect(button(banner, 'Bring it back here')).toBeUndefined();
    expect(text(el.querySelector('mc-composer .blocked'))).toBe(
      'This session lives on DESKTOP-PC, which is offline: take it over to go on here.',
    );

    button(banner, 'Take over…')?.click();
    harness.detectChanges();
    const sheet = banner.querySelector('mc-sheet')!;
    expect(text(sheet)).toContain(
      "The session goes on here from this computer's copy: it has the conversation only up to",
    );
    expect(text(sheet)).toContain('both versions are kept side by side when it next moves there.');
    const success = vi.spyOn(TestBed.inject(NoticeService), 'success');
    button(sheet, 'Take over from')?.click();
    await settle();
    expect(backend.bodyOf('POST /sessions/l1/move')).toEqual({ to: SELF_ID, takeOver: true });
    expect(success).toHaveBeenCalledWith(
      "It goes on here from this computer's copy. If DESKTOP-PC went on with it while it was away, both versions are kept side by side when it next moves there.",
      { title: 'Fix tray icon lives here again' },
    );
  });

  it("offers Bring back on another computer's session, through that computer", async () => {
    here(remoteSession('other', { title: 'Something else' }));
    remoteList(socket, PEER_ID, [remoteSession('r1', { title: 'Release notes', status: 'idle' })]);
    backend.routes.set(`${PROXY}/sessions/r1`, () => ({
      status: 200,
      body: {
        ...remoteSession('r1', { title: 'Release notes', status: 'idle' }),
        pendingDetail: [],
      },
    }));
    backend.routes.set(`${PROXY}/sessions/r1/transcript`, () => ({
      status: 200,
      body: { events: [], tail: '0' },
    }));
    backend.routes.set(`${PROXY}/moves`, () => ({ status: 200, body: { moves: [] } }));
    const el = await open(`/session/r1/on/${PEER_ID}`);
    // Its moves were read there, so one under way would show.
    expect(backend.calls).toContain(`GET ${PROXY}/moves`);
    const items = menu(el);
    // It never lived here: it goes on here, rather than coming back.
    expect(items).toContain('Continue on this computer…');
    // The only other computer is this one: nowhere else to send it.
    expect(items).not.toContain('Continue on another computer…');
  });

  it('shows its moves on the Info tab', async () => {
    here(remoteSession('l1', { title: 'Fix tray icon', status: 'idle' }));
    backend.routes.set('/moves', () => ({
      status: 200,
      body: {
        moves: [
          movedTo('undone', {
            session: 'l1',
            title: 'Fix tray icon',
            createdAt: Date.now() - 3_600_000,
          }),
        ],
      },
    }));
    const el = await open('/session/l1?tab=info');
    const card = el.querySelector('mc-info-panel mc-move-card');
    expect(text(card?.querySelector('.mtitle'))).toBe('Fix tray icon');
    expect(text(card?.querySelector('.route'))).toBe('This computer → DESKTOP-PC');
    expect(text(card?.querySelector('.sum'))).toBe(
      'Undone: Fix tray icon lives on this computer again.',
    );
  });
});
