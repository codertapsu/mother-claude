import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { MachinesView, defaultGrants, setGrant } from '../../core/machines';
import { MachinesService } from '../../core/machines.service';
import { FakeBackend, asDesktop, flush, useFakeServer } from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { OTHER, machinesFrames, machinesView, pairing } from './machines.testing';
import { PairSheetComponent } from './pair-sheet.component';

describe('PairSheetComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<PairSheetComponent>;
  /** What `GET /machines` answers now. */
  let current: MachinesView;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    asDesktop();
    current = machinesView();
    backend.routes.set('/machines', () => ({ status: 200, body: current }));
    backend.routes.set('/machines/pair', (method) => {
      current = machinesView({ pairing: method === 'DELETE' ? null : pairing() });
      return { status: 200, body: current };
    });
    backend.routes.set('/machines/confirm', () => {
      const p = current.pairing!;
      current = machinesView({ pairing: { ...p, state: 'waitingOther', confirmed: true } });
      return { status: 200, body: current };
    });
  });

  afterEach(() => {
    fixture?.destroy();
    setActiveMock(null);
  });

  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const button = (el: Element, label: string) =>
    [...el.querySelectorAll<HTMLButtonElement>('button')].find((b) => text(b).startsWith(label));
  const settle = async () => {
    for (let i = 0; i < 3; i++) {
      await flush();
      fixture.detectChanges();
    }
  };
  const open = async () => {
    fixture = TestBed.createComponent(PairSheetComponent);
    fixture.componentRef.setInput('open', true);
    fixture.detectChanges();
    await settle();
    return fixture.nativeElement as HTMLElement;
  };

  it('shows a new code, then the number to compare once the other computer joined', async () => {
    const el = await open();
    expect(backend.calls).toContain('POST /machines/pair');
    const code = el.querySelector('.code');
    expect(text(code)).toBe('K7QM-4R2X');
    expect(code?.getAttribute('aria-label')).toBe('Pairing code K 7 Q M 4 R 2 X');
    expect(text(el.querySelector('[role="timer"]'))).toMatch(/^Works once · expires in [45]:\d\d$/);
    // Nearby only shows which computer shows a code: the code is typed.
    expect(text(el)).toContain('Join and type this code.');
    expect(text(el)).toContain(
      'it lists “MacBook” among the computers nearby, so you can check which computer shows the code',
    );

    // The other computer typed the code: the server says so in a frame.
    const frame = await machinesFrames(backend);
    current = machinesView({
      pairing: pairing({ state: 'confirm', code: null, other: OTHER, digits: '481270' }),
    });
    frame();
    await settle();
    expect(el.querySelector('dialog')?.getAttribute('aria-label')).toBe('Pair with DESKTOP-PC?');
    expect(text(el.querySelector('.who'))).toBe('DESKTOP-PC · Windows · Direct (same network)');
    expect(text(el.querySelector('.digits [aria-hidden="true"]'))).toBe('481 270');
    expect(text(el.querySelector('.digits .visually-hidden'))).toBe('The number: 481270');

    // Moving sessions only on the same network.
    el.querySelector<HTMLInputElement>(
      'input[aria-label="Move sessions here and away, Internet"]',
    )!.click();
    await settle();
    button(el, 'The numbers match — Pair')!.click();
    await settle();
    expect(backend.bodyOf('POST /machines/confirm')).toEqual({
      grants: setGrant(defaultGrants(), 'move', 'internet', false),
    });
    expect(text(el)).toContain('Waiting for DESKTOP-PC to confirm…');
  });

  it('has the six digits typed when the other computer joined over the internet', async () => {
    current = machinesView({
      pairing: pairing({
        state: 'confirm',
        code: null,
        other: { ...OTHER, path: 'relay' },
        digits: '481270',
        mustType: true,
      }),
    });
    await TestBed.inject(MachinesService).load();
    const el = await open();
    // The pairing in progress shows: no new code.
    expect(backend.calls).not.toContain('POST /machines/pair');
    expect(text(el)).toContain('DESKTOP-PC joined over the internet. Type the number it shows:');
    // Never shown here: they are read on the other computer.
    expect(text(el)).not.toContain('481');
    const pair = button(el, 'Pair')!;
    expect(pair.disabled).toBe(true);
    const input = el.querySelector<HTMLInputElement>('input.typed')!;
    input.value = '481 27';
    input.dispatchEvent(new Event('input'));
    await settle();
    expect(pair.disabled).toBe(true);
    input.value = '481 270';
    input.dispatchEvent(new Event('input'));
    await settle();
    expect(pair.disabled).toBe(false);
    pair.click();
    await settle();
    expect(backend.bodyOf('POST /machines/confirm')).toEqual({
      grants: defaultGrants(),
      digits: '481270',
    });
  });

  it('cancels the pairing when closed while it runs', async () => {
    const el = await open();
    button(el, 'Cancel')!.click();
    await settle();
    expect(backend.calls).toContain('DELETE /machines/pair');
    expect(TestBed.inject(MachinesService).pairing()).toBeNull();
  });

  it('says why a pairing failed', async () => {
    const el = await open();
    const frame = await machinesFrames(backend);
    current = machinesView({
      pairing: pairing({
        state: 'failed',
        code: null,
        other: OTHER,
        error: { code: 'pair_digits_wrong', message: 'The digits did not match.' },
      }),
    });
    frame();
    await settle();
    expect(text(el.querySelector('[role="alert"]'))).toBe(
      "That's not the number on DESKTOP-PC. Nothing was paired.",
    );
    expect(button(el, 'New code')).toBeDefined();
  });
});
