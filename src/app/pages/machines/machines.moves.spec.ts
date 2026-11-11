import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { MachinesService } from '../../core/machines.service';
import {
  FakeBackend,
  FakeSocket,
  asPhoneOf,
  flush,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { MOVE_ID, movedTo } from '../../core/mock/moves.testing';
import { goLive } from '../../core/mock/remote.testing';
import { RealtimeService } from '../../core/realtime.service';
import { MachinesComponent } from './machines.component';
import { machinesView, peer, setupView } from './machines.testing';

describe('MachinesComponent: Recent moves', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<MachinesComponent>;
  let socket: FakeSocket;

  beforeEach(async () => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    backend.routes.set('/machines/setup', () => ({ status: 200, body: setupView() }));
    backend.routes.set('/machines', () => ({
      status: 200,
      body: machinesView({ enabled: true, peers: [peer()] }),
    }));
    socket = await goLive(backend);
  });

  afterEach(() => {
    fixture?.destroy();
    TestBed.inject(RealtimeService).stop();
    setActiveMock(null);
  });

  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const button = (el: Element, label: string) =>
    [...el.querySelectorAll<HTMLButtonElement>('button')].find((b) => text(b).startsWith(label));
  const settle = async () => {
    for (let i = 0; i < 4; i++) {
      await flush();
      fixture.detectChanges();
    }
  };
  const render = async () => {
    fixture = TestBed.createComponent(MachinesComponent);
    fixture.detectChanges();
    await settle();
    await TestBed.inject(MachinesService).load();
    await settle();
    return fixture.nativeElement as HTMLElement;
  };
  const cards = (el: HTMLElement) => [...el.querySelectorAll('.card.moves mc-move-card')];

  it("lists this computer's moves, follows them live and undoes one with a second tap", async () => {
    const done = movedTo('resumed', { undoUntil: Date.now() + 7 * 86_400_000 });
    backend.routes.set('/moves', () => ({ status: 200, body: { moves: [done] } }));
    backend.routes.set(`/moves/${MOVE_ID}/undo`, () => ({
      status: 200,
      body: movedTo('undone', { undoUntil: null }),
    }));
    // A phone may act on a move (a device may, as the desktop does).
    asPhoneOf('macos');
    const el = await render();

    expect(text(el.querySelector('.card.moves h2'))).toBe('Recent moves');
    const [card] = cards(el);
    expect(text(card.querySelector('.mstate'))).toBe('Done');
    expect(text(card.querySelector('.route'))).toBe('This computer → DESKTOP-PC');
    expect(text(card.querySelector('.sum'))).toBe('Fix the login runs on DESKTOP-PC now.');
    expect(card.querySelector('a')?.getAttribute('href')).toBe(`/session/s1/on/${peer().id}`);

    // A new one arrives as a frame, and shows how far it got.
    socket.frame({
      t: 'move',
      move: movedTo('sending', {
        id: 'mv_00000000000000ff',
        title: 'Release notes',
        bytesSent: 10 * 1024 * 1024,
        createdAt: Date.now(),
      }),
    });
    fixture.detectChanges();
    expect(cards(el).length).toBe(2);
    const live = cards(el)[0];
    expect(text(live.querySelector('.mtitle'))).toBe('Release notes');
    expect(text(live.querySelector('.mstate'))).toBe('Moving');
    expect(live.querySelector('[role="progressbar"]')?.getAttribute('aria-valuenow')).toBe('51');
    expect(text(live.querySelector('.sum'))).toBe('Sending to DESKTOP-PC: 10 MB of 20 MB');
    expect(button(live, 'Cancel the move')).toBeTruthy();

    // Undo arms, then goes.
    const old = cards(el)[1];
    button(old, 'Undo move')!.click();
    fixture.detectChanges();
    expect(backend.calls).not.toContain(`POST /moves/${MOVE_ID}/undo`);
    button(old, 'Tap again to undo')!.click();
    await settle();
    expect(backend.calls).toContain(`POST /moves/${MOVE_ID}/undo`);
    expect(text(cards(el)[1].querySelector('.mstate'))).toBe('Undone');
    expect(button(cards(el)[1], 'Undo move')).toBeUndefined();
  });

  it('says why an undo was refused, in words', async () => {
    backend.routes.set('/moves', () => ({
      status: 200,
      body: { moves: [movedTo('resumed', { undoUntil: Date.now() + 86_400_000 })] },
    }));
    backend.routes.set(`/moves/${MOVE_ID}/undo`, () => ({
      status: 409,
      body: {
        error: {
          code: 'move_went_on',
          message:
            'It went on on DESKTOP-PC after the move: bring it back instead (an undo would drop what happened here).',
        },
      },
    }));
    const el = await render();
    const [card] = cards(el);
    button(card, 'Undo move')!.click();
    fixture.detectChanges();
    button(card, 'Tap again to undo')!.click();
    await settle();
    expect(text(card.querySelector('[role="alert"]'))).toBe(
      'It went on on DESKTOP-PC after the move: bring it back instead (an undo would drop what happened there).',
    );
  });

  it('shows nothing while there was no move', async () => {
    backend.routes.set('/moves', () => ({ status: 200, body: { moves: [] } }));
    const el = await render();
    expect(el.querySelector('.card.moves')).toBeNull();
  });
});
