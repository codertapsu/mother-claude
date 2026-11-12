import { Component } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';

import { defaultGrants, setGrant } from '../../core/machines';
import {
  FakeBackend,
  asDesktop,
  asPhoneOf,
  flush,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { MachinesComponent } from './machines.component';
import { PEER_ID, check, machinesView, peer, setupView } from './machines.testing';

@Component({ template: '' })
class BlankComponent {}

describe('MachinesComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<MachinesComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [
      provideRouter([{ path: 'machines/setup', component: BlankComponent }]),
    ]);
    backend.routes.set('/machines/setup', () => ({ status: 200, body: setupView() }));
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
  const render = async (view = machinesView()) => {
    backend.routes.set('/machines', () => ({ status: 200, body: view }));
    fixture = TestBed.createComponent(MachinesComponent);
    fixture.detectChanges();
    await settle();
    return fixture.nativeElement as HTMLElement;
  };

  it('is off until it is set up, and offers the setup', async () => {
    asDesktop();
    const el = await render();
    const off = el.querySelector('.card.off');
    expect(text(off?.querySelector('h2'))).toBe('Machines is off');
    expect(text(off?.querySelector('.needs'))).toContain(
      'This version of Mother Claude, or newer, on each computer',
    );
    // Nothing to pair from, check or set while it was never set up.
    expect(button(el, 'Pair a machine')).toBeUndefined();
    expect(el.querySelector('.card.health')).toBeNull();
    expect(el.querySelector('.card.options')).toBeNull();
    expect(backend.calls).not.toContain('GET /machines/setup');
    button(el, 'Set up Machines')!.click();
    await settle();
    expect(TestBed.inject(Router).url).toBe('/machines/setup');
  });

  it('offers to go on with a setup left halfway', async () => {
    asDesktop();
    const el = await render(
      machinesView({
        setupStep: 3,
        health: { state: 'blocker', blockers: 2, warnings: 0, at: Date.now() },
      }),
    );
    expect(text(el.querySelector('.card.off'))).toContain('Setup: step 3 of 7 — 2 required left');
    expect(button(el, 'Go on with the setup')).toBeDefined();
  });

  it('shows each paired computer: online, how, and what it may do here', async () => {
    asDesktop();
    const grants = setGrant(defaultGrants(), 'dangerous', 'lan', true);
    const el = await render(
      machinesView({
        enabled: true,
        peers: [peer({ grants })],
        health: { state: 'ok', blockers: 0, warnings: 0, at: Date.now() - 60_000 },
      }),
    );
    const card = el.querySelector('.card.peer')!;
    expect(text(card.querySelector('h2'))).toBe('DESKTOP-PC');
    expect(text(card.querySelector('.status'))).toContain('Online · Direct (same network)');
    expect(text(card)).toContain(
      'Windows 11 Pro · Mother Claude 0.11.0 · Machines protocol 1 · Claude Code 2.1.322',
    );
    const [here, there] = [...card.querySelectorAll('.grants')];
    expect([...here.querySelectorAll('.gchip')].map((c) => text(c))).toEqual([
      'See ✓',
      'Drive ✓',
      'Move ✓',
      'Run –',
      'Dangerous same network only',
    ]);
    // What this computer may do there is the other computer's to say.
    expect(text(there)).toContain('Not known yet.');
    expect(text(el.querySelector('.card.health .badge'))).toBe('All good');
    expect(button(el, 'Pair a machine')).toBeDefined();
    expect(text(el.querySelector('.card.options'))).toContain('Public relays (default)');
  });

  it('changes what a computer may do through the matrix', async () => {
    asDesktop();
    backend.routes.set(`/machines/${PEER_ID}`, () => ({
      status: 200,
      body: machinesView({ enabled: true, peers: [peer()] }),
    }));
    const el = await render(machinesView({ enabled: true, peers: [peer()] }));
    button(el.querySelector('.card.peer')!, 'Change')!.click();
    await settle();
    // Seeing over the internet off: driving and moving need it, so they go too.
    const box = el.querySelector<HTMLInputElement>(
      'input[aria-label="See sessions, transcripts, requests, Internet"]',
    )!;
    box.click();
    await settle();
    button(el.querySelector('.card.peer')!, 'Save')!.click();
    await settle();
    expect(backend.bodyOf(`PATCH /machines/${PEER_ID}`)).toEqual({
      grants: setGrant(defaultGrants(), 'view', 'internet', false),
    });
  });

  it('revokes a computer in two steps', async () => {
    asDesktop();
    backend.routes.set(`/machines/${PEER_ID}`, () => ({
      status: 200,
      body: machinesView({ enabled: true }),
    }));
    const el = await render(machinesView({ enabled: true, peers: [peer()] }));
    button(el, 'Revoke')!.click();
    await settle();
    expect(backend.calls).not.toContain(`DELETE /machines/${PEER_ID}`);
    button(el, 'Tap again to revoke')!.click();
    await settle();
    expect(backend.calls).toContain(`DELETE /machines/${PEER_ID}`);
    expect(text(el.querySelector('[role="status"]'))).toBe(
      'Revoked DESKTOP-PC. Its connections were cut.',
    );
    expect(el.querySelector('.card.peer')).toBeNull();
  });

  it('lists the checks that fail, with their fix', async () => {
    asDesktop();
    backend.routes.set('/machines/setup', () => ({
      status: 200,
      body: setupView({
        warnings: 1,
        self: {
          machine: 'x',
          name: 'MacBook',
          at: Date.now(),
          error: null,
          checks: [
            check(),
            check({
              id: 'keepAwake',
              area: 'power',
              title: 'Stays awake while used',
              state: 'warning',
              detail: 'It may sleep while another computer uses it.',
              why: 'A computer asleep cannot answer the others.',
              severity: 'recommended',
              fix: { kind: 'auto', label: 'Keep awake while used' },
            }),
          ],
        },
      }),
    }));
    backend.routes.set('/machines/setup/fix', () => ({ status: 200, body: setupView() }));
    const el = await render(
      machinesView({
        enabled: true,
        peers: [peer()],
        health: { state: 'warning', blockers: 0, warnings: 1, at: Date.now() },
      }),
    );
    const health = el.querySelector('.card.health')!;
    expect(text(health.querySelector('.badge'))).toBe('1 recommended');
    // Only the failing check, with why it matters.
    expect(health.querySelectorAll('li.check').length).toBe(1);
    expect(text(health.querySelector('li.check'))).toContain(
      'A computer asleep cannot answer the others.',
    );
    button(health, 'Keep awake while used')!.click();
    await settle();
    expect(backend.bodyOf('POST /machines/setup/fix')).toEqual({ check: 'keepAwake' });
  });

  it('is read-only on a phone', async () => {
    asPhoneOf('macos');
    const el = await render(machinesView({ enabled: true, peers: [peer()] }));
    expect(text(el)).toContain('Machines are set up from the desktop app.');
    expect(text(el.querySelector('.card.peer h2'))).toBe('DESKTOP-PC');
    for (const label of ['Pair a machine', 'Join…', 'Revoke', 'Change', 'Rename', 'Re-check']) {
      expect(button(el, label), label).toBeUndefined();
    }
    expect(button(el, 'Turn Machines off')).toBeUndefined();
  });
});
