import { Component } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';

import {
  FakeBackend,
  asDesktop,
  asPhoneOf,
  flush,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { PEER_ID, check, machinesView, peer, setupView } from './machines.testing';
import { MachinesSetupComponent } from './setup.component';

@Component({ template: '' })
class BlankComponent {}

const RELAY_DOWN =
  "The relay does not answer: on the same network your computers still meet, across networks they can't.";

describe('MachinesSetupComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<MachinesSetupComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([{ path: 'machines', component: BlankComponent }])]);
    // Left at step 7, with one computer paired.
    backend.routes.set('/machines', () => ({
      status: 200,
      body: machinesView({ setupStep: 7, peers: [peer()] }),
    }));
    backend.routes.set('/machines/setup/progress', () => ({
      status: 200,
      body: machinesView({ setupStep: 7, peers: [peer()] }),
    }));
    backend.routes.set('/machines/setup', () => ({ status: 200, body: setupView() }));
    backend.routes.set('/machines/setup/check', () => ({ status: 200, body: setupView() }));
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
    for (let i = 0; i < 4; i++) {
      await flush();
      fixture.detectChanges();
    }
  };
  const render = async () => {
    fixture = TestBed.createComponent(MachinesSetupComponent);
    fixture.detectChanges();
    await settle();
    return fixture.nativeElement as HTMLElement;
  };

  it('goes on at the step it was left at, checking every computer first', async () => {
    asDesktop();
    const el = await render();
    expect(backend.calls).toContain('POST /machines/setup/check');
    expect(el.querySelector('.rstep[aria-current="step"]')?.textContent).toContain(
      'Test, then turn on',
    );
    expect(text(el.querySelector('.step-body h2'))).toBe('Test, then turn on');
  });

  it('keeps Turn on Machines off while a required check fails, and says why', async () => {
    asDesktop();
    backend.routes.set('/machines/setup/check', () => ({
      status: 200,
      body: setupView({ blockers: 2 }),
    }));
    const el = await render();
    const turnOn = button(el, 'Turn on Machines')!;
    expect(turnOn.disabled).toBe(true);
    expect(turnOn.getAttribute('aria-describedby')).toBe('turn-on-reason');
    expect(text(el.querySelector('#turn-on-reason'))).toBe(
      '2 required checks are left: fix them first.',
    );
  });

  it('lists the required checks the server names when it refuses to turn on', async () => {
    asDesktop();
    backend.routes.set('/machines/enable', () => ({
      status: 409,
      body: {
        error: {
          code: 'machines_blocked',
          message: '1 required check is left: fix it first.',
          blockers: [{ id: 'relay', title: 'The relay', detail: RELAY_DOWN }],
        },
      },
    }));
    const el = await render();
    button(el, 'Turn on Machines')!.click();
    await settle();
    expect(backend.bodyOf('POST /machines/enable')).toEqual({ enabled: true });
    expect(text(el.querySelector('.notice.error'))).toBe('1 required check is left: fix it first.');
    const refused = [...el.querySelectorAll('.refused li')].map((li) => text(li));
    expect(refused).toEqual([`The relay — ${RELAY_DOWN}`]);
    // Still here, the checks read again.
    expect(TestBed.inject(Router).url).not.toBe('/machines');
    expect(backend.calls).toContain('GET /machines/setup');
  });

  it('runs the readiness test, then turns Machines on', async () => {
    asDesktop();
    backend.routes.set('/machines/test', () => ({
      status: 200,
      body: {
        results: [
          {
            machine: PEER_ID,
            name: 'DESKTOP-PC',
            at: Date.now(),
            tests: [
              {
                id: 'connect',
                state: 'ok',
                detail: 'Direct, on the same network.',
                ms: 6.2,
                mibps: null,
              },
              {
                id: 'transfer',
                state: 'ok',
                detail: '20 MiB at 28.4 MiB/s.',
                ms: 704,
                mibps: 28.4,
              },
            ],
          },
        ],
      },
    }));
    backend.routes.set('/machines/enable', () => ({
      status: 200,
      body: machinesView({ enabled: true, peers: [peer()] }),
    }));
    const el = await render();
    button(el, 'Run the test')!.click();
    await settle();
    const row = el.querySelector('table.tests tbody tr')!;
    expect(text(row.querySelector('th'))).toBe('DESKTOP-PC');
    const cells = [...row.querySelectorAll('td')].map((td) => text(td));
    expect(row.querySelector('td .visually-hidden')?.textContent).toBe('Passed');
    expect(cells[0]).toContain('Direct, on the same network.');
    expect(cells[0]).toContain('6.2 ms');
    expect(cells[1]).toContain('20 MiB at 28.4 MiB/s.');
    button(el, 'Turn on Machines')!.click();
    await settle();
    expect(TestBed.inject(Router).url).toBe('/machines');
  });

  it('asks another computer to fix its own checks', async () => {
    asDesktop();
    const failing = check({
      id: 'firewall',
      area: 'network',
      title: 'The firewall',
      state: 'warning',
      detail: 'No firewall rule lets your other computers reach Mother Claude.',
      why: 'Without a rule, your other computers reach this PC only through the relay.',
      severity: 'recommended',
      fix: { kind: 'admin', label: 'Allow through the firewall' },
    });
    const view = setupView({
      warnings: 1,
      peers: [
        { machine: PEER_ID, name: 'DESKTOP-PC', at: Date.now(), checks: [failing], error: null },
      ],
    });
    backend.routes.set('/machines/setup/check', () => ({ status: 200, body: view }));
    backend.routes.set('/machines/setup/ask', () => ({ status: 200, body: { asked: true } }));
    const el = await render();
    button(el, '3')!.click();
    await settle();
    const columns = [...el.querySelectorAll('.columns .column')];
    expect(columns.map((c) => text(c.querySelector('h3')))).toEqual([
      'This computer',
      'DESKTOP-PC',
    ]);
    expect(text(columns[1])).toContain('On DESKTOP-PC: Allow through the firewall');
    button(columns[1], 'Ask DESKTOP-PC to fix')!.click();
    await settle();
    expect(backend.bodyOf('POST /machines/setup/ask')).toEqual({
      machine: PEER_ID,
      check: 'firewall',
    });
    expect(text(el.querySelector('.notice.ok'))).toBe(
      'Asked DESKTOP-PC: it shows the fix to whoever is at it.',
    );
  });

  it('is the desktop’s to run', async () => {
    asPhoneOf('macos');
    const el = await render();
    expect(text(el)).toContain('Machines are set up from the desktop app.');
    expect(button(el, 'Turn on Machines')).toBeUndefined();
    expect(backend.calls).not.toContain('POST /machines/setup/check');
  });
});
