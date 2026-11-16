import { ComponentFixture, TestBed } from '@angular/core/testing';

import { SessionChoice, SessionsChoiceView } from '../../core/machines';
import { FakeBackend, asDesktop, flush, useFakeServer } from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { PEER_ID } from './machines.testing';
import { SessionsShownSheetComponent } from './sessions-shown-sheet.component';

const SYNC = `/machines/${PEER_ID}/sessions/sync`;
const A = 'a0ee0000-1111-4222-8333-444455556661';
const B = 'a0ee0000-1111-4222-8333-444455556662';

const session = (id: string, over: Partial<SessionChoice> = {}): SessionChoice => ({
  id,
  name: 'mc-1',
  title: null,
  projectName: 'mc',
  cwd: 'D:\\dev\\mc',
  live: false,
  status: 'ended',
  lastActivityAt: Date.now() - 60_000,
  synced: true,
  ...over,
});

describe('SessionsShownSheetComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<SessionsShownSheetComponent>;
  /** What the server keeps: the mode and each session's box. */
  let saved: SessionsChoiceView;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend);
    asDesktop();
    saved = {
      mode: 'all',
      loaded: true,
      sessions: [
        session(A, { title: 'Fix the tray icon', live: true }),
        session(B, { title: 'Release notes', synced: false }),
      ],
    };
    backend.routes.set(SYNC, (method, body) => {
      if (method === 'PUT') {
        const mode = (body as { mode: 'all' | 'chosen' }).mode;
        saved = {
          ...saved,
          mode,
          sessions: saved.sessions.map((s) => ({ ...s, synced: mode === 'all' && s.id === A })),
        };
      }
      return { status: 200, body: saved };
    });
    for (const id of [A, B]) {
      backend.routes.set(`${SYNC}/${id}`, (_m, body) => {
        const synced = (body as { synced: boolean }).synced;
        saved = {
          ...saved,
          sessions: saved.sessions.map((s) => (s.id === id ? { ...s, synced } : s)),
        };
        return { status: 200, body: saved };
      });
    }
  });

  afterEach(() => {
    fixture?.destroy();
    setActiveMock(null);
  });

  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const dialog = () => document.querySelector('dialog[open]') as HTMLElement;
  const settle = async () => {
    for (let i = 0; i < 3; i++) {
      await flush();
      fixture.detectChanges();
    }
  };
  const open = async () => {
    fixture = TestBed.createComponent(SessionsShownSheetComponent);
    fixture.componentRef.setInput('machine', PEER_ID);
    fixture.componentRef.setInput('name', 'DESKTOP-PC');
    fixture.componentRef.setInput('open', true);
    fixture.detectChanges();
    await settle();
    return dialog();
  };
  const boxes = () => [
    ...dialog().querySelectorAll<HTMLInputElement>('.list input[type="checkbox"]'),
  ];

  it('lists every session of that computer, hidden ones too, and says nothing changes there', async () => {
    const el = await open();
    expect(text(el.querySelector('.title'))).toBe('Sessions of DESKTOP-PC shown here');
    expect(text(el)).toContain(
      'This only changes what this computer shows: nothing is deleted or changed on DESKTOP-PC.',
    );
    const items = [...el.querySelectorAll('.list > li')];
    expect(items.map((li) => text(li.querySelector('.name')))).toEqual([
      'Fix the tray icon',
      'Release notes',
    ]);
    expect(text(items[0])).toContain('Running');
    expect(text(items[1])).toContain('Not synced');
    expect(boxes().map((b) => b.checked)).toEqual([true, false]);
    expect(boxes()[0].getAttribute('aria-label')).toBe('Sync Fix the tray icon to this computer');
    expect(text(el)).toContain('1 of 2 show here.');
    expect(el.querySelector<HTMLInputElement>('input[value="all"]')?.checked).toBe(true);
  });

  it('stops syncing a session, and syncs it again, at once', async () => {
    await open();
    boxes()[0].click();
    await settle();
    expect(backend.bodyOf(`POST ${SYNC}/${A}`)).toEqual({ synced: false });
    expect(boxes().map((b) => b.checked)).toEqual([false, false]);
    boxes()[1].click();
    await settle();
    expect(backend.bodyOf(`POST ${SYNC}/${B}`)).toEqual({ synced: true });
    expect(text(dialog())).toContain('1 of 2 show here.');
  });

  it('shows only the chosen ones once asked, and says when none is chosen yet', async () => {
    const el = await open();
    el.querySelector<HTMLInputElement>('input[value="chosen"]')!.click();
    await settle();
    expect(backend.bodyOf(`PUT ${SYNC}`)).toEqual({ mode: 'chosen' });
    expect(text(dialog())).toContain(
      'None chosen yet: tick the sessions to show on this computer.',
    );
    expect(boxes().every((b) => !b.checked)).toBe(true);
  });

  it('puts a box back when the change was refused, and says why', async () => {
    backend.routes.set(`${SYNC}/${A}`, () => ({
      status: 422,
      body: {
        error: { code: 'too_many_sessions', message: 'This choice already names 5000 sessions.' },
      },
    }));
    await open();
    boxes()[0].click();
    await settle();
    expect(boxes()[0].checked).toBe(true);
    expect(text(dialog().querySelector('[role="alert"]'))).toContain(
      'This choice already names 5000 sessions.',
    );
  });
});
