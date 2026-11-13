import { Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { By } from '@angular/platform-browser';
import { provideRouter } from '@angular/router';
import { RouterTestingHarness } from '@angular/router/testing';

import { FakeBackend, FakeSocket, flush, useFakeServer } from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import {
  PEER_ID,
  goLive,
  pairedMachines,
  remote,
  remoteList,
  remoteSession,
} from '../../core/mock/remote.testing';
import { NoticeService } from '../../core/notice.service';
import { RealtimeService } from '../../core/realtime.service';
import { RemoteSessionComponent } from './remote-session.component';
import { SessionContext } from './session-context';
import { SessionDetailComponent } from './session-detail.component';

@Component({ template: '' })
class BlankComponent {}

const PROXY = `/machines/${PEER_ID}/proxy`;

describe("another computer's session page (/session/:id/on/:machine)", () => {
  let backend: FakeBackend;
  let harness: RouterTestingHarness;
  let socket: FakeSocket;

  beforeEach(async () => {
    backend = new FakeBackend();
    useFakeServer(backend, [
      provideRouter([
        { path: 'session/:id/on/:machine', component: RemoteSessionComponent },
        { path: 'session/:id', component: BlankComponent },
        { path: '', component: BlankComponent },
      ]),
    ]);
    pairedMachines();
    socket = await goLive(backend);
    socket.frame({ t: 'sessions', full: true, sessions: [], seq: 1 });
    remoteList(socket, PEER_ID, [remoteSession('r1', { title: 'Fix tray icon' })]);
    backend.routes.set(`${PROXY}/sessions/r1`, () => ({
      status: 200,
      body: { ...remoteSession('r1', { title: 'Fix tray icon' }), pendingDetail: [] },
    }));
    backend.routes.set(`${PROXY}/sessions/r1/transcript`, () => ({
      status: 200,
      body: {
        events: [{ id: 'e1:0', role: 'assistant', kind: 'text', text: 'Looking at tray.rs' }],
        tail: '7',
      },
    }));
    harness = await RouterTestingHarness.create();
  });

  afterEach(() => {
    TestBed.inject(RealtimeService).stop();
    setActiveMock(null);
  });

  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const open = async () => {
    await harness.navigateByUrl(`/session/r1/on/${PEER_ID}`);
    for (let i = 0; i < 3; i++) {
      await flush();
      harness.detectChanges();
    }
    return harness.routeNativeElement as HTMLElement;
  };
  const ctx = () =>
    harness
      .routeDebugElement!.query(By.directive(SessionDetailComponent))
      .injector.get(SessionContext);

  it('reads it through the proxy and follows it live with its computer', async () => {
    const el = await open();
    expect(backend.calls).toContain(`GET ${PROXY}/sessions/r1`);
    expect(backend.calls).toContain(`GET ${PROXY}/sessions/r1/transcript`);
    expect(backend.calls.filter((c) => c.startsWith('GET /sessions'))).toEqual([]);
    expect(socket.sent).toContainEqual({ t: 'sub', session: 'r1', machine: PEER_ID });
    expect(socket.sent.filter((f) => f.t === 'sub' && !('machine' in f))).toEqual([]);

    expect(text(el.querySelector('mc-machine-banner'))).toBe(
      'Runs on DESKTOP-PC — you drive it from here.',
    );
    expect(text(el.querySelector('mc-session-header .mchip'))).toBe('DESKTOP-PC');
    expect(text(el.querySelector('mc-transcript'))).toContain('Looking at tray.rs');

    // Its frames reach the page; this computer's own never do.
    remote(socket, PEER_ID, {
      t: 'transcript',
      session: 'r1',
      events: [{ id: 'e2:0', role: 'assistant', kind: 'text', text: 'Built the icon.' }],
      tail: '8',
    });
    socket.frame({
      t: 'transcript',
      session: 'r1',
      events: [{ id: 'x:0', role: 'assistant', kind: 'text', text: 'Not this one' }],
      tail: '9',
    });
    harness.detectChanges();
    expect(text(el.querySelector('mc-transcript'))).toContain('Built the icon.');
    expect(text(el.querySelector('mc-transcript'))).not.toContain('Not this one');
  });

  it('sends through the proxy, and says what the computer refused', async () => {
    backend.routes.set(`${PROXY}/sessions/r1/message`, () => ({
      status: 403,
      body: {
        error: {
          code: 'grant_missing',
          message: 'This computer has not given yours the "control" permission.',
          grant: 'control',
        },
      },
    }));
    await open();
    const sent = await ctx().send('Ship it');
    expect(sent).toBeNull();
    expect(backend.bodyOf(`POST ${PROXY}/sessions/r1/message`)).toEqual({ text: 'Ship it' });
    expect(
      TestBed.inject(NoticeService)
        .toasts()
        .map((t) => t.text),
    ).toContain('Not sent: DESKTOP-PC lets this computer watch its sessions, not drive them.');
  });

  it('leaves out what only that computer does, saying so in one line', async () => {
    const el = await open();
    (el.querySelector('[aria-label="Session menu"]') as HTMLButtonElement).click();
    harness.detectChanges();
    const menu = text(document.querySelector('mc-sheet .menu'));
    expect(menu).toContain(
      'Switching its account, moving, copying and deleting it are done on DESKTOP-PC.',
    );
    expect(menu).not.toContain('Delete…');
    expect(menu).not.toContain('Move or copy');
    // Its project's settings are that computer's.
    expect(menu).not.toContain('Project settings…');
    expect(menu).toContain('Rename…');
  });

  it("says when it went offline, and holds sending until it's back", async () => {
    const since = new Date().setHours(14, 32, 0, 0);
    const el = await open();
    remote(socket, PEER_ID, { t: 'offline', since });
    harness.detectChanges();
    expect(text(el.querySelector('mc-machine-banner'))).toContain('DESKTOP-PC went offline at');
    expect(text(el.querySelector('mc-composer .blocked'))).toBe(
      'Sending waits until DESKTOP-PC is back.',
    );
    expect(el.querySelector('mc-composer textarea')).toBeNull();
    expect(ctx().canAct()).toBe(false);

    remote(socket, PEER_ID, { t: 'online' });
    harness.detectChanges();
    expect(ctx().canAct()).toBe(true);
    expect(el.querySelector('mc-composer textarea')).not.toBeNull();
  });
});
