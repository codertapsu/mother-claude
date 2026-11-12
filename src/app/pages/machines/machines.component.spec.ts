import { Component } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';

import { defaultGrants, setGrant } from '../../core/machines';
import { PermissionsService } from '../../core/permissions.service';
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

/** Computers › Overview (docs/UI-REDESIGN.md §6.5). */
describe('MachinesComponent: Computers › Overview', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<MachinesComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [
      provideRouter([
        { path: 'computers/setup', component: BlankComponent },
        { path: 'computers/options', component: BlankComponent },
        { path: 'inbox', component: BlankComponent },
      ]),
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
    expect(text(off?.querySelector('h2'))).toBe('Off on this computer');
    expect(text(off?.querySelector('.needs'))).toContain(
      'This version of Mother Claude, or newer, on each computer',
    );
    // Nothing to pair from or check while it was never set up.
    expect(button(el, 'Pair a computer')).toBeUndefined();
    expect(el.querySelector('mc-page-head .badge.state')).toBeNull();
    expect(button(el, 'Check again')).toBeUndefined();
    expect(backend.calls).not.toContain('GET /machines/setup');
    // The four tabs.
    const tabs = [...el.querySelectorAll('mc-hub-tabs a')];
    expect(tabs.map((a) => text(a))).toEqual(['Overview', 'Code sync', 'Activity', 'Options']);
    button(el, 'Set up your computers')!.click();
    await settle();
    expect(TestBed.inject(Router).url).toBe('/computers/setup');
  });

  it('sends what another computer asked to the Inbox, where it is answered', async () => {
    asDesktop();
    const ask = {
      id: 'ask_0000000000000001',
      from: PEER_ID,
      fromName: 'DESKTOP-PC',
      kind: 'signIn' as const,
      account: 'Work',
      at: Date.now(),
      state: 'asked' as const,
    };
    const el = await render(
      machinesView({
        enabled: true,
        peers: [peer()],
        asks: [ask, { ...ask, id: 'ask_2', state: 'done' as const }],
      }),
    );
    // The card moved to the Inbox: one line here says so.
    expect(el.querySelector('mc-machine-asks')).toBeNull();
    const line = el.querySelector('.asks-line');
    expect(text(line)).toBe(
      'One of your other computers asks something of this one. Answer in the Inbox',
    );
    expect(line?.querySelector('a')?.getAttribute('href')).toBe('/inbox');
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
      'Windows 11 Pro · Mother Claude 0.11.0 · protocol 1 · Claude Code 2.1.322',
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
    // How they stand, in the head; pairing beside it.
    expect(text(el.querySelector('mc-page-head .badge.state'))).toBe('✓ All good');
    expect(button(el, 'Pair a computer')).toBeDefined();
    // The options are a tab of their own.
    expect(el.querySelector('.card.options')).toBeNull();
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
    expect(text(el.querySelector('mc-page-head .badge.state'))).toBe('! 1 recommended');
    // This computer's card: only the failing check, with why it matters.
    const self = el.querySelector('.card.self')!;
    expect(self.querySelectorAll('li.check').length).toBe(1);
    expect(text(self.querySelector('li.check'))).toContain(
      'A computer asleep cannot answer the others.',
    );
    button(self, 'Keep awake while used')!.click();
    await settle();
    expect(backend.bodyOf('POST /machines/setup/fix')).toEqual({ check: 'keepAwake' });
  });

  it("lists another computer's failing checks on its card, asked there", async () => {
    asDesktop();
    backend.routes.set('/machines/setup', () => ({
      status: 200,
      body: setupView({
        blockers: 1,
        peers: [
          {
            machine: PEER_ID,
            name: 'DESKTOP-PC',
            at: Date.now(),
            error: null,
            checks: [
              check({
                id: 'firewall',
                area: 'network',
                title: 'The firewall',
                state: 'blocker',
                detail: 'The firewall blocks Mother Claude.',
                fix: { kind: 'auto', label: 'Allow through the firewall' },
              }),
            ],
          },
        ],
      }),
    }));
    backend.routes.set('/machines/setup/ask', () => ({ status: 200, body: { asked: true } }));
    const el = await render(
      machinesView({
        enabled: true,
        peers: [peer()],
        health: { state: 'blocker', blockers: 1, warnings: 0, at: Date.now() },
      }),
    );
    expect(text(el.querySelector('mc-page-head .badge.state'))).toBe('✕ 1 required');
    const card = el.querySelector('.card.peer')!;
    expect(text(card.querySelector('li.check'))).toContain('The firewall blocks Mother Claude.');
    button(card, 'Ask DESKTOP-PC to fix')!.click();
    await settle();
    expect(backend.bodyOf('POST /machines/setup/ask')).toEqual({
      machine: PEER_ID,
      check: 'firewall',
    });
  });

  it("shows this computer's permissions, Claude Code tools and background service", async () => {
    asDesktop();
    TestBed.inject(PermissionsService).permissions.set([
      {
        id: 'full-disk-access',
        label: 'Full Disk Access',
        description: 'Reads the sessions.',
        granted: true,
        required: true,
        steps: [],
      },
      {
        id: 'local-network',
        label: 'Local Network',
        description: 'Reaches the others.',
        granted: false,
        required: true,
        steps: ['Open System Settings'],
      },
    ]);
    backend.routes.set('/mcp', () => ({
      status: 200,
      body: { enabled: true, clients: 2, recentCalls: [], tools: [], toolCount: 9 },
    }));
    backend.routes.set('/services', () => ({
      status: 200,
      body: {
        daemon: { reachable: true, raw: 'pid 4242 · up 3 h' },
        mcpServers: { github: {} },
        backgroundJobs: [
          { id: 's1', cwd: '/Users/me/app', state: 'running' },
          { id: 's2', cwd: '/Users/me/web', state: 'done' },
        ],
      },
    }));
    const el = await render(machinesView({ enabled: true, peers: [peer()] }));
    const self = el.querySelector('.card.self')!;
    expect([...self.querySelectorAll('.fact')].map((f) => text(f))).toEqual([
      '✓ Full Disk Access : granted',
      '✕ Local Network : not granted',
      '✓ Claude Code tools on',
    ]);
    // A required permission is missing: the list with its steps shows.
    expect(self.querySelector('details.perms')?.hasAttribute('open')).toBe(true);
    expect(text(self.querySelector('.service'))).toBe(
      'Claude Code’s background service · running · 2 background jobs Details',
    );
    button(self, 'Details')!.click();
    await settle();
    const sheet = el.querySelector('mc-services-details')!;
    expect(text(sheet)).toContain('pid 4242 · up 3 h');
    expect(text(sheet)).toContain('github');
    expect(text(sheet)).toContain('s2 — done — /Users/me/web');
  });

  it('is read-only on a phone', async () => {
    asPhoneOf('macos');
    const el = await render(machinesView({ enabled: true, peers: [peer()] }));
    expect(text(el)).toContain('Your computers are set up from the desktop app.');
    expect(text(el.querySelector('.card.peer h2'))).toBe('DESKTOP-PC');
    for (const label of ['Pair a computer', 'Join…', 'Revoke', 'Change', 'Rename', 'Check again']) {
      expect(button(el, label), label).toBeUndefined();
    }
  });
});
