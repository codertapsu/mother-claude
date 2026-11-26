import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { SessionsComponent } from '../pages/sessions/sessions.component';
import { SessionCardComponent } from '../shared/session-card.component';
import { FakeBackend, flush, me, useFakeServer } from './mock/fake-server.testing';
import { ownedCaps } from './mock/fixtures';
import { setActiveMock } from './mock/mock-api';
import { SessionSummary } from './models';
import { storageRemove } from './platform';
import { Pin, normalizePins, pinKey } from './pins';
import { PinsService } from './pins.service';
import { RealtimeService } from './realtime.service';

const words = (el: Element | null | undefined) =>
  (el?.textContent ?? '').replace(/\s+/g, ' ').trim();

const session = (id: string, over: Partial<SessionSummary> = {}): SessionSummary => ({
  id,
  name: id,
  title: `Session ${id}`,
  projectName: 'api',
  cwd: '/Users/me/dev/api',
  surface: 'terminal',
  owned: false,
  live: true,
  status: 'working',
  activity: { kind: 'thinking', label: 'Thinking' },
  usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
  messageCount: 1,
  agents: { running: 0, total: 0, workflowsRunning: 0, backgroundRunning: 0 },
  queued: 0,
  pending: [],
  control: { ...ownedCaps(), channel: 'none' },
  lastActivityAt: Date.now(),
  ...over,
});

describe('pinned sessions', () => {
  let backend: FakeBackend;
  let pins: Pin[];
  /** Sessions the server knows that the list's frames no longer carry. */
  let older: SessionSummary[];

  beforeEach(() => {
    backend = new FakeBackend();
    pins = [];
    older = [];
    const view = () => ({
      status: 200,
      body: { pins, sessions: older.filter((s) => pins.some((p) => p.id === s.id)) },
    });
    backend.routes.set('/pins', view);
    for (const id of ['a', 'b', 'c', 'old']) {
      backend.routes.set(`/pins/${id}`, (method, _body, url) => {
        const machine = url?.searchParams.get('machine') ?? undefined;
        const same = (p: Pin) => p.id === id && p.machine === machine;
        if (method === 'PUT' && !pins.some(same)) {
          pins = [...pins, { id, ...(machine ? { machine } : {}), at: Date.now() }];
        }
        if (method === 'DELETE') pins = pins.filter((p) => !same(p));
        return view();
      });
    }
    useFakeServer(backend, [provideRouter([])]);
  });

  afterEach(() => {
    TestBed.inject(RealtimeService).stop();
    setActiveMock(null);
    storageRemove('mc_sessions_filter');
  });

  const store = (list: SessionSummary[]) =>
    (
      TestBed.inject(RealtimeService) as unknown as { replaceStore(l: SessionSummary[]): void }
    ).replaceStore(list);

  it('reads the answer tolerantly; a pin is keyed by its computer too', () => {
    const v = normalizePins({
      pins: [{ id: 'a', at: 5 }, { id: 'b', machine: 'mac' }, { nope: 1 }, 7],
      sessions: [{ id: 'a' }, { title: 'no id' }],
      extra: true,
    });
    expect(v.pins).toEqual([
      { id: 'a', at: 5 },
      { id: 'b', machine: 'mac', at: 0 },
    ]);
    expect(v.sessions.map((s) => s.id)).toEqual(['a']);
    expect(normalizePins(null)).toEqual({ pins: [], sessions: [] });
    expect(pinKey('a')).toBe('a');
    expect(pinKey('a', 'mac')).not.toBe(pinKey('a'));
  });

  it('pins from the card, and the page leads with them in the order they were pinned', async () => {
    const now = Date.now();
    store([
      session('a', { lastActivityAt: now }),
      session('b', { lastActivityAt: now - 60_000 }),
      session('c', { lastActivityAt: now - 120_000, status: 'idle' }),
    ]);
    pins = [
      { id: 'c', at: 1 },
      { id: 'old', at: 2 },
    ];
    // Pinned long ago, no longer in the list's frames: the pins bring it.
    older = [session('old', { status: 'ended', live: false, lastActivityAt: now - 9e8 })];
    const fixture = TestBed.createComponent(SessionsComponent);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    const el = fixture.nativeElement as HTMLElement;
    const group = () =>
      [...el.querySelectorAll('section.group')].map((g) => ({
        title: words(g.querySelector('h2')),
        cards: [...g.querySelectorAll('mc-session-card .name')].map((n) => words(n)),
      }));
    expect(group()[0]).toEqual({ title: 'Pinned 2', cards: ['Session c', 'Session old'] });
    // Shown only there.
    expect(group().flatMap((g) => g.cards).filter((n) => n === 'Session c').length).toBe(1);

    // The pin on a card pins it: last in the group.
    const cardOf = (name: string) =>
      [...el.querySelectorAll('mc-session-card')].find(
        (c) => words(c.querySelector('.name')) === name,
      ) as HTMLElement;
    const pinA = cardOf('Session a').querySelector('button.pin') as HTMLButtonElement;
    expect(pinA.getAttribute('aria-pressed')).toBe('false');
    expect(pinA.getAttribute('aria-label')).toBe('Pin Session a to the top');
    pinA.click();
    await flush();
    fixture.detectChanges();
    expect(backend.calls).toContain('PUT /pins/a');
    expect(group()[0].cards).toEqual(['Session c', 'Session old', 'Session a']);
    const pinned = cardOf('Session a').querySelector('button.pin') as HTMLButtonElement;
    expect(pinned.getAttribute('aria-pressed')).toBe('true');
    expect(pinned.classList.contains('on')).toBe(true);

    // The search applies to the pinned ones too.
    const search = el.querySelector('.search input[type="search"]') as HTMLInputElement;
    search.value = 'Session a';
    search.dispatchEvent(new Event('input'));
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    expect(group()[0]).toEqual({ title: 'Pinned 1', cards: ['Session a'] });

    // Unpinned from its card: back in its own group.
    search.value = '';
    search.dispatchEvent(new Event('input'));
    fixture.detectChanges();
    (cardOf('Session c').querySelector('button.pin') as HTMLButtonElement).click();
    await flush();
    fixture.detectChanges();
    expect(backend.calls).toContain('DELETE /pins/c');
    expect(group()[0].cards).toEqual(['Session old', 'Session a']);
    expect(group().find((g) => g.title.startsWith('Idle'))?.cards).toEqual(['Session c']);
  });

  it("pins another computer's session as its own, and follows every client's changes", async () => {
    const svc = TestBed.inject(PinsService);
    svc.ensureLoaded();
    await flush();
    const rt = TestBed.inject(RealtimeService);
    await rt.start();
    backend.last.open();
    backend.last.frame({ t: 'hello', v: 2, me: me(), serverTime: Date.now() });
    await flush();

    const fixture = TestBed.createComponent(SessionCardComponent);
    fixture.componentRef.setInput('session', session('b', { machine: 'mac' }));
    fixture.componentRef.setInput('machine', {
      id: 'mac',
      name: 'MacBook',
      online: true,
      since: null,
    });
    fixture.detectChanges();
    (fixture.nativeElement.querySelector('button.pin') as HTMLButtonElement).click();
    await flush();
    expect(backend.urls).toContain('/pins/b?machine=mac');
    expect(svc.isPinned('b', 'mac')).toBe(true);
    expect(svc.isPinned('b')).toBe(false);

    // Another client unpinned it: the frame brings the change here.
    pins = [];
    backend.last.frame({ t: 'pins' });
    await flush();
    expect(svc.isPinned('b', 'mac')).toBe(false);
  });
});
