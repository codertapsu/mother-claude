import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { MachinesView, defaultGrants } from '../../core/machines';
import { MachinesService } from '../../core/machines.service';
import { FakeBackend, asDesktop, flush, useFakeServer } from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { JoinSheetComponent } from './join-sheet.component';
import { OTHER, machinesFrames, machinesView, pairing } from './machines.testing';

describe('JoinSheetComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<JoinSheetComponent>;
  /** What `GET /machines` answers now. */
  let current: MachinesView;

  const joining = () =>
    machinesView({ pairing: pairing({ role: 'joiner', state: 'joining', code: null }) });

  beforeEach(async () => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    asDesktop();
    current = machinesView();
    backend.routes.set('/machines', () => ({ status: 200, body: current }));
    backend.routes.set('/machines/nearby', () => ({
      status: 200,
      body: { machines: [{ id: OTHER.id, name: 'DESKTOP-PC', os: 'windows' }] },
    }));
    backend.routes.set('/machines/join', () => {
      current = joining();
      return { status: 200, body: current };
    });
    backend.routes.set('/machines/confirm', () => {
      current = machinesView({
        pairing: { ...current.pairing!, state: 'waitingOther', confirmed: true },
      });
      return { status: 200, body: current };
    });
    // This computer's name is known before the sheet opens.
    await TestBed.inject(MachinesService).load();
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
    fixture = TestBed.createComponent(JoinSheetComponent);
    fixture.componentRef.setInput('open', true);
    fixture.detectChanges();
    await settle();
    return fixture.nativeElement as HTMLElement;
  };
  const type = async (input: HTMLInputElement, value: string) => {
    input.value = value;
    input.dispatchEvent(new Event('input'));
    await settle();
  };

  it('lists the computers nearby and joins with the code and this computer’s name', async () => {
    const el = await open();
    const nearby = el.querySelector('.nearby li');
    expect(text(nearby?.querySelector('strong'))).toBe('DESKTOP-PC');
    expect(text(nearby?.querySelector('.muted'))).toBe('Windows · showing a code');
    // Listed to check which one shows a code: joining takes its code.
    expect(text(el.querySelector('.nearby .hint'))).toBe(
      'They are listed so you can check which computer shows a code: type its code below.',
    );
    const name = el.querySelector<HTMLInputElement>('input[name="name"]')!;
    expect(name.value).toBe('MacBook');
    const join = button(el, 'Join')!;
    expect(join.disabled).toBe(true);
    await type(el.querySelector<HTMLInputElement>('input[name="code"]')!, 'K7QM-4R2X');
    join.click();
    await settle();
    expect(backend.bodyOf('POST /machines/join')).toEqual({ code: 'K7QM-4R2X', name: 'MacBook' });
    expect(text(el.querySelector('[role="status"]'))).toBe('Joining…');

    // The other computer answered: both compare the number, then confirm.
    const frame = await machinesFrames(backend);
    current = machinesView({
      pairing: pairing({
        role: 'joiner',
        state: 'confirm',
        code: null,
        other: OTHER,
        digits: '481270',
      }),
    });
    frame();
    await settle();
    expect(text(el.querySelector('.digits [aria-hidden="true"]'))).toBe('481 270');
    button(el, 'The numbers match — Pair')!.click();
    await settle();
    expect(backend.bodyOf('POST /machines/confirm')).toEqual({ grants: defaultGrants() });
    expect(text(el)).toContain('Waiting for DESKTOP-PC to confirm…');
  });

  it('says when no computer shows the code', async () => {
    backend.routes.set('/machines/nearby', () => ({ status: 200, body: { machines: [] } }));
    const el = await open();
    expect(text(el.querySelector('.nearby'))).toContain(
      'None nearby. Type the code: it works across the internet too.',
    );
    await type(el.querySelector<HTMLInputElement>('input[name="code"]')!, 'ZZZZ-9999');
    button(el, 'Join')!.click();
    await settle();
    const frame = await machinesFrames(backend);
    current = machinesView({
      pairing: pairing({
        role: 'joiner',
        state: 'failed',
        code: null,
        error: { code: 'pair_not_found', message: 'No computer shows this code.' },
      }),
    });
    frame();
    await settle();
    expect(text(el.querySelector('[role="alert"]'))).toBe(
      'No computer showing this code was found. Check the code, and that both computers have the internet or are on the same network.',
    );
    // Try again goes back to the code, kept.
    button(el, 'Try again')!.click();
    await settle();
    expect(el.querySelector<HTMLInputElement>('input[name="code"]')!.value).toBe('ZZZZ-9999');
  });

  it('shows a code the server refuses in the form', async () => {
    backend.routes.set('/machines/join', () => ({
      status: 422,
      body: {
        error: {
          code: 'pair_code_malformed',
          message: 'A code has 8 letters and digits, like K7QM-4R2X.',
        },
      },
    }));
    const el = await open();
    await type(el.querySelector<HTMLInputElement>('input[name="code"]')!, 'K7Q');
    button(el, 'Join')!.click();
    await settle();
    expect(text(el.querySelector('form [role="alert"]'))).toContain(
      'A code has 8 letters and digits, like K7QM-4R2X.',
    );
  });
});
