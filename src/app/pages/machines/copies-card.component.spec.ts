import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { CopiesView, HeldCopy, KeptCopy, copyStateText } from '../../core/copies';
import { hostOsSignal } from '../../core/i18n/i18n';
import { FakeBackend, asDesktop, flush, useFakeServer } from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { KeepCopySheetComponent } from '../../shared/keep-copy-sheet.component';
import { CopiesCardComponent } from './copies-card.component';
import { PEER_ID } from './machines.testing';

const SID = 'c0ee0000-1111-4222-8333-444455556666';

function held(over: Partial<HeldCopy> = {}): HeldCopy {
  return {
    id: `held.${PEER_ID}.${SID}`,
    machine: PEER_ID,
    machineName: 'DESKTOP-PC',
    online: false,
    session: SID,
    title: 'Tray icon',
    cwd: 'D:\\dev\\app',
    account: 'Default',
    bytes: 18_400_000,
    files: 12,
    copiedAt: Date.now() - 60_000,
    turnAt: Date.now() - 60_000,
    createdAt: Date.now() - 86_400_000,
    complete: true,
    livesHere: false,
    canContinue: true,
    ...over,
  };
}

function kept(over: Partial<KeptCopy> = {}): KeptCopy {
  return {
    id: `kept.${PEER_ID}.${SID}`,
    machine: PEER_ID,
    machineName: 'DESKTOP-PC',
    online: true,
    session: SID,
    title: 'Release notes',
    from: 'session',
    status: {
      state: 'paused',
      reason: 'noGrant',
      detail: 'x',
      since: Date.now(),
      bytes: 0,
      files: 0,
    },
    ...over,
  };
}

const view = (h: HeldCopy[], k: KeptCopy[]): CopiesView => ({
  enabled: true,
  held: h,
  kept: k,
  capBytes: 2 * 1024 ** 3,
  used: {},
});

describe('Copies', () => {
  let backend: FakeBackend;

  beforeEach(() => {
    backend = new FakeBackend();
    // A take over opens the session's page.
    useFakeServer(backend, [provideRouter([{ path: '**', children: [] }])]);
    asDesktop({ hostOs: 'macos' });
    hostOsSignal.set('macos');
  });

  afterEach(() => {
    setActiveMock(null);
    hostOsSignal.set(undefined);
  });

  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const button = (el: Element, label: string) =>
    [...el.querySelectorAll<HTMLButtonElement>('button')].find((b) => text(b).startsWith(label));

  it('says how a copy stands in a line', () => {
    expect(copyStateText(null)).toBe('Not copied yet');
    expect(copyStateText({ state: 'waiting', since: 0, bytes: 0, files: 0, copiedAt: null })).toBe(
      'Copies once its turn ends',
    );
    expect(
      copyStateText({ state: 'paused', reason: 'offline', since: 0, bytes: 0, files: 0 }),
    ).toBe('Paused: the other computer can’t be reached');
    expect(
      copyStateText({ state: 'copied', since: 0, bytes: 0, files: 0, copiedAt: Date.now() }),
    ).toMatch(/^Copied at /);
  });

  describe('the card', () => {
    let fixture: ComponentFixture<CopiesCardComponent>;

    afterEach(() => fixture?.destroy());

    const settle = async () => {
      for (let i = 0; i < 3; i++) {
        await flush();
        fixture.detectChanges();
      }
    };

    it('lists the copies held here and kept elsewhere, and removes one on a second tap', async () => {
      backend.routes.set('/machines/copies', () => ({
        status: 200,
        body: view([held()], [kept()]),
      }));
      fixture = TestBed.createComponent(CopiesCardComponent);
      fixture.detectChanges();
      await settle();
      const el = fixture.nativeElement as HTMLElement;
      const [here, there] = [...el.querySelectorAll('.list')];
      expect(text(here)).toContain('Tray icon');
      expect(text(here)).toContain('from DESKTOP-PC');
      expect(text(here)).toContain('18 MB');
      expect(text(here)).toContain('DESKTOP-PC is offline');
      expect(button(here, 'Continue here')).toBeDefined();
      expect(text(there)).toContain('Release notes');
      expect(text(there)).toContain(
        'Paused: the other computer has not given this one the Move permission',
      );

      backend.routes.set(`/machines/copies/held.${PEER_ID}.${SID}`, () => ({
        status: 200,
        body: view([], [kept()]),
      }));
      button(here, 'Remove')!.click();
      await settle();
      expect(backend.calls).not.toContain(`DELETE /machines/copies/held.${PEER_ID}.${SID}`);
      button(here, 'Tap again to remove')!.click();
      await settle();
      expect(backend.calls).toContain(`DELETE /machines/copies/held.${PEER_ID}.${SID}`);
      expect(text(el.querySelector('.list'))).toContain(
        'No copy of another computer’s session is here.',
      );
    });

    it('continues a session here from its copy while its computer is offline', async () => {
      backend.routes.set('/machines/copies', () => ({ status: 200, body: view([held()], []) }));
      backend.routes.set(`/sessions/${SID}/move`, () => ({
        status: 200,
        body: { takenOver: true, fromCopy: true, resumed: true, warning: 'ok' },
      }));
      fixture = TestBed.createComponent(CopiesCardComponent);
      fixture.detectChanges();
      await settle();
      const el = fixture.nativeElement as HTMLElement;
      button(el, 'Continue here')!.click();
      await settle();
      expect(backend.bodyOf(`POST /sessions/${SID}/move`)).toEqual(
        expect.objectContaining({ takeOver: true, fromCopy: true }),
      );
    });
  });

  describe('Keep a copy on…', () => {
    let fixture: ComponentFixture<KeepCopySheetComponent>;

    afterEach(() => fixture?.destroy());

    const settle = async () => {
      for (let i = 0; i < 3; i++) {
        await flush();
        fixture.detectChanges();
      }
    };

    it('lists the computers with how each copy stands, and saves the choice', async () => {
      const sessionView = (on: boolean) => ({
        session: SID,
        explicit: on,
        project: [],
        livesOn: null,
        enabled: true,
        machines: [
          {
            id: PEER_ID,
            name: 'DESKTOP-PC',
            os: 'windows',
            online: true,
            on,
            from: on ? 'session' : null,
            inProject: false,
            supported: true,
            status: on
              ? { state: 'copied', since: 0, bytes: 10, files: 1, copiedAt: Date.now() }
              : null,
          },
        ],
      });
      backend.routes.set(`/sessions/${SID}/copies`, (method) => ({
        status: 200,
        body: sessionView(method === 'PUT'),
      }));
      fixture = TestBed.createComponent(KeepCopySheetComponent);
      fixture.componentRef.setInput('session', SID);
      fixture.componentRef.setInput('open', true);
      fixture.detectChanges();
      await settle();
      const el = fixture.nativeElement as HTMLElement;
      expect(text(el)).toContain('DESKTOP-PC');
      expect(text(el)).toContain('No copy there.');
      const box = el.querySelector<HTMLInputElement>('input[type="checkbox"]')!;
      box.checked = true;
      box.dispatchEvent(new Event('change'));
      fixture.detectChanges();
      button(el, 'Save')!.click();
      await settle();
      expect(backend.bodyOf(`PUT /sessions/${SID}/copies`)).toEqual({ machines: [PEER_ID] });
      expect(text(el)).toMatch(/Copied at /);
    });
  });
});
