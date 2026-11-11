import { Component } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';

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
});
