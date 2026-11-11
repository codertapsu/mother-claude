import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { FakeBackend, FakeSocket, flush, useFakeServer } from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import {
  PEER_ID,
  ask,
  askFull,
  goLive,
  pairedMachines,
  remote,
  remoteList,
  remoteSession,
} from '../../core/mock/remote.testing';
import { RealtimeService } from '../../core/realtime.service';
import { InboxComponent } from './inbox.component';

const PROXY = `/machines/${PEER_ID}/proxy`;

describe("InboxComponent with another computer's requests", () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<InboxComponent>;
  let socket: FakeSocket;

  beforeEach(async () => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    backend.routes.set('/inbox', () => ({ status: 200, body: { items: [] } }));
    backend.routes.set(`${PROXY}/inbox`, () => ({
      status: 200,
      body: { items: [askFull('q1', 'r1')] },
    }));
    backend.routes.set(`${PROXY}/sessions/r1`, () => ({
      status: 200,
      body: { ...remoteSession('r1'), pendingDetail: [askFull('q1', 'r1')] },
    }));
    pairedMachines();
    socket = await goLive(backend);
    socket.frame({ t: 'sessions', full: true, sessions: [], seq: 1 });
    remoteList(socket, PEER_ID, [
      remoteSession('r1', { title: 'Fix tray icon', status: 'waiting', pending: [ask('q1')] }),
    ]);
  });

  afterEach(() => {
    fixture?.destroy();
    TestBed.inject(RealtimeService).stop();
    setActiveMock(null);
  });

  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const settle = async () => {
    for (let i = 0; i < 3; i++) {
      await flush();
      fixture.detectChanges();
    }
  };
  const render = async () => {
    fixture = TestBed.createComponent(InboxComponent);
    fixture.detectChanges();
    await settle();
    return fixture.nativeElement as HTMLElement;
  };
  const button = (el: Element | null | undefined, label: string) =>
    [...(el?.querySelectorAll<HTMLButtonElement>('button') ?? [])].find((b) => text(b) === label);

  it('names the computer it waits at, and answers it through that computer', async () => {
    backend.routes.set(`${PROXY}/sessions/r1/respond`, () => ({ status: 200, body: {} }));
    const el = await render();
    const card = el.querySelector('mc-pending-card');
    expect(text(card?.querySelector('.sess'))).toBe('Fix tray icon');
    expect(text(card?.querySelector('.mchip'))).toBe('DESKTOP-PC');
    expect(card?.querySelector('a.sess')?.getAttribute('href')).toBe(`/session/r1/on/${PEER_ID}`);
    expect(backend.calls).toContain(`GET ${PROXY}/inbox`);

    button(card, 'Allow once')?.click();
    await settle();
    expect(backend.bodyOf(`POST ${PROXY}/sessions/r1/respond`)).toEqual({
      requestId: 'q1',
      decision: 'allow',
    });
    expect(backend.calls).not.toContain('POST /sessions/r1/respond');
    expect(text(card?.querySelector('.done'))).toBe('Allowed');
  });

  it('says the computer answered first when it did', async () => {
    const el = await render();
    remote(socket, PEER_ID, {
      t: 'pendingResolved',
      id: 'q1',
      session: 'r1',
      outcome: 'withdrawn',
      by: { kind: 'host' },
      at: Date.now(),
    });
    fixture.detectChanges();
    expect(text(el.querySelector('mc-pending-card .done'))).toBe('Answered at DESKTOP-PC');
  });

  it('keeps its answer from racing another: the first one wins there', async () => {
    backend.routes.set(`${PROXY}/sessions/r1/respond`, () => ({
      status: 409,
      body: { error: { code: 'already_decided', message: 'Already answered on DESKTOP-PC.' } },
    }));
    const el = await render();
    const card = el.querySelector('mc-pending-card');
    button(card, 'Deny')?.click();
    await settle();
    expect(text(card?.querySelector('.done'))).toBe('Already answered on DESKTOP-PC.');
  });

  it('leaves out an offline computer’s requests', async () => {
    remote(socket, PEER_ID, { t: 'offline', since: Date.now() });
    const el = await render();
    expect(el.querySelector('mc-pending-card')).toBeNull();
    expect(text(el.querySelector('.empty-state strong'))).toBe('Nothing needs you');
  });
});
