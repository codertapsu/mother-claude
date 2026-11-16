import { Component } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';
import { RouterTestingHarness } from '@angular/router/testing';

import { FakeBackend, asDesktop, flush, useFakeServer } from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { goLive, remoteList, remoteSession } from '../../core/mock/remote.testing';
import { RealtimeService } from '../../core/realtime.service';
import { MachinesComponent } from './machines.component';
import { PEER_ID, machinesView, peer, setupView } from './machines.testing';

@Component({ template: '' })
class BlankComponent {}

describe("MachinesComponent: a paired computer's sessions", () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<MachinesComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([{ path: '', component: BlankComponent }])]);
    backend.routes.set('/machines/setup', () => ({ status: 200, body: setupView() }));
    asDesktop();
  });

  afterEach(() => {
    fixture?.destroy();
    TestBed.inject(RealtimeService).stop();
    setActiveMock(null);
  });

  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const render = async (sessions: { total: number; running: number } | null) => {
    backend.routes.set('/machines', () => ({
      status: 200,
      body: machinesView({ enabled: true, peers: [peer({ sessions })] }),
    }));
    fixture = TestBed.createComponent(MachinesComponent);
    fixture.detectChanges();
    for (let i = 0; i < 3; i++) {
      await flush();
      fixture.detectChanges();
    }
    return fixture.nativeElement as HTMLElement;
  };

  it('counts them and opens the session list on them', async () => {
    const el = await render({ total: 3, running: 1 });
    const line = el.querySelector('section.peer .sessions');
    expect(text(line?.querySelector('span'))).toBe('3 sessions, 1 running');
    const link = line?.querySelector('a') as HTMLAnchorElement;
    expect(text(link)).toBe('Open its sessions');
    expect(link.getAttribute('href')).toBe(`/?machine=${PEER_ID}`);
    link.click();
    await flush();
    expect(TestBed.inject(Router).url).toBe(`/?machine=${PEER_ID}`);
  });

  it('follows them live once its list arrived', async () => {
    const s = await goLive(backend);
    remoteList(s, PEER_ID, [remoteSession('r1'), remoteSession('r2', { live: false })]);
    const el = await render({ total: 5, running: 5 });
    expect(text(el.querySelector('section.peer .sessions span'))).toBe('2 sessions, 1 running');
  });

  it('says nothing while none were heard', async () => {
    const el = await render(null);
    expect(el.querySelector('section.peer .sessions')).toBeNull();
  });

  /** The choice of its sessions shown here, as the server keeps it. */
  const choiceRoute = () =>
    backend.routes.set(`/machines/${PEER_ID}/sessions/sync`, () => ({
      status: 200,
      body: { mode: 'all', loaded: true, sessions: [] },
    }));

  it('says which of its sessions show here, and opens the choice', async () => {
    choiceRoute();
    backend.routes.set('/machines', () => ({
      status: 200,
      body: machinesView({
        enabled: true,
        peers: [
          peer({ sessions: { total: 3, running: 1 }, sessionsShown: { mode: 'all', hidden: 2 } }),
        ],
      }),
    }));
    fixture = TestBed.createComponent(MachinesComponent);
    fixture.detectChanges();
    for (let i = 0; i < 3; i++) {
      await flush();
      fixture.detectChanges();
    }
    const el = fixture.nativeElement as HTMLElement;
    const line = el.querySelector('section.peer .shown');
    expect(text(line?.querySelector('span'))).toBe('Shown here: all of its sessions but 2');
    (line?.querySelector('button') as HTMLButtonElement).click();
    for (let i = 0; i < 3; i++) {
      await flush();
      fixture.detectChanges();
    }
    expect(text(document.querySelector('dialog[open] .title'))).toBe(
      'Sessions of DESKTOP-PC shown here',
    );
    expect(backend.calls).toContain(`GET /machines/${PEER_ID}/sessions/sync`);
  });

  it('says when only the chosen ones show, and nothing from an older computer', async () => {
    let el = await render(null);
    expect(el.querySelector('section.peer .shown')).toBeNull();
    fixture.destroy();
    backend.routes.set('/machines', () => ({
      status: 200,
      body: machinesView({
        enabled: true,
        peers: [
          peer({
            sessions: { total: 1, running: 0 },
            sessionsShown: { mode: 'chosen', hidden: 4 },
          }),
        ],
      }),
    }));
    fixture = TestBed.createComponent(MachinesComponent);
    fixture.detectChanges();
    for (let i = 0; i < 3; i++) {
      await flush();
      fixture.detectChanges();
    }
    el = fixture.nativeElement as HTMLElement;
    expect(text(el.querySelector('section.peer .shown span'))).toBe(
      'Shown here: only the sessions you choose (1)',
    );
  });
});

describe("MachinesComponent: opened on a paired computer's sessions shown here", () => {
  afterEach(() => {
    TestBed.inject(RealtimeService).stop();
    setActiveMock(null);
  });

  it('opens the choice from the Sessions list’s link, and forgets the link once closed', async () => {
    const backend = new FakeBackend();
    useFakeServer(backend, [
      provideRouter([
        { path: 'computers', component: MachinesComponent },
        { path: '', component: BlankComponent },
      ]),
    ]);
    asDesktop();
    backend.routes.set('/machines/setup', () => ({ status: 200, body: setupView() }));
    backend.routes.set('/machines', () => ({
      status: 200,
      body: machinesView({
        enabled: true,
        peers: [peer({ sessionsShown: { mode: 'chosen', hidden: 0 } })],
      }),
    }));
    backend.routes.set(`/machines/${PEER_ID}/sessions/sync`, () => ({
      status: 200,
      body: { mode: 'chosen', loaded: true, sessions: [] },
    }));
    const harness = await RouterTestingHarness.create();
    await harness.navigateByUrl(`/computers?sessions=${PEER_ID}`);
    for (let i = 0; i < 4; i++) {
      await flush();
      harness.detectChanges();
    }
    const dialog = document.querySelector('dialog[open]');
    expect((dialog?.querySelector('.title')?.textContent ?? '').trim()).toBe(
      'Sessions of DESKTOP-PC shown here',
    );
    (
      [...(dialog?.querySelectorAll('button') ?? [])].find(
        (b) => b.textContent?.trim() === 'Close',
      ) as HTMLButtonElement
    ).click();
    for (let i = 0; i < 3; i++) {
      await flush();
      harness.detectChanges();
    }
    expect(TestBed.inject(Router).url).toBe('/computers');
  });
});
