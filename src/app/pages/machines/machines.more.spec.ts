import { Component } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';

import { Introduction, MachinesView, Revocation } from '../../core/machines';
import {
  FakeBackend,
  asDesktop,
  asPhoneOf,
  flush,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { MachinesComponent } from './machines.component';
import { PEER_ID, machinesView, peer, setupView } from './machines.testing';

@Component({ template: '' })
class BlankComponent {}

/** A second paired computer. */
const OTHER_ID = 'e'.repeat(32);

/** Three computers or more (docs/PLAN-MACHINES.md Phase 5): versions and
 * what an older one lacks, *Revoke everywhere*, introductions. */
describe('MachinesComponent: more than two computers', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<MachinesComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([{ path: 'settings', component: BlankComponent }])]);
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
  const render = async (view: MachinesView) => {
    backend.routes.set('/machines', () => ({ status: 200, body: view }));
    fixture = TestBed.createComponent(MachinesComponent);
    fixture.detectChanges();
    await settle();
    return fixture.nativeElement as HTMLElement;
  };
  const two = () => [
    peer(),
    peer({ id: OTHER_ID, name: 'Laptop', os: 'macos', osVersion: 'macOS 27.2' }),
  ];

  it('names an older computer, what it lacks, and asks it to update', async () => {
    asDesktop();
    backend.routes.set('/machines/setup/ask', () => ({ status: 200, body: { asked: true } }));
    const el = await render(
      machinesView({
        enabled: true,
        peers: [
          peer({
            versions: { app: '0.10.9', protocol: 1, claude: '2.1.300' },
            skew: { app: 'older', claude: 'older', lacks: ['tasks', 'sync'] },
          }),
        ],
      }),
    );
    const card = el.querySelector('.card.peer')!;
    expect(text(card)).toContain(
      'Windows 11 Pro · Mother Claude 0.10.9 · Machines protocol 1 · Claude Code 2.1.300',
    );
    expect(text(card)).toContain(
      'DESKTOP-PC runs an older Mother Claude (0.10.9; this computer: 0.11.0).',
    );
    expect(text(card)).toContain('DESKTOP-PC runs an older Claude Code (2.1.300; here: 2.1.322).');
    expect(text(card.querySelector('.lacks'))).toBe(
      'DESKTOP-PC can’t take tasks and keep code in sync until it updates.',
    );
    const asks = [...card.querySelectorAll<HTMLButtonElement>('.skew button')];
    expect(asks.map((b) => text(b))).toEqual([
      'Ask DESKTOP-PC to update',
      'Ask DESKTOP-PC to update',
    ]);
    asks[0].click();
    await settle();
    expect(backend.bodyOf('POST /machines/setup/ask')).toEqual({
      machine: PEER_ID,
      check: 'app',
    });
    expect(text(el.querySelector('[role="status"]'))).toBe(
      'Asked DESKTOP-PC: it shows the update to whoever is at it.',
    );
    asks[1].click();
    await settle();
    expect(backend.bodyOf('POST /machines/setup/ask')).toEqual({
      machine: PEER_ID,
      check: 'claudeCode',
    });
  });

  it('sends this computer to Software updates when it is the older one', async () => {
    asDesktop();
    const el = await render(
      machinesView({
        enabled: true,
        peers: [
          peer({
            versions: { app: '0.12.0', protocol: 1, claude: '2.1.322' },
            skew: { app: 'newer' },
          }),
        ],
      }),
    );
    const card = el.querySelector('.card.peer')!;
    expect(text(card)).toContain(
      'This computer runs an older Mother Claude than DESKTOP-PC (0.12.0).',
    );
    button(card, 'Update it')!.click();
    await settle();
    expect(TestBed.inject(Router).url).toBe('/settings#updates');
  });

  it('revokes a computer everywhere in two taps, and says who is still to be told', async () => {
    asDesktop();
    const revoked: Revocation = {
      machine: PEER_ID,
      name: 'DESKTOP-PC',
      key: '8C21 0E5F A9B3 4D67',
      at: Date.now(),
      pending: [{ id: OTHER_ID, name: 'Laptop', online: false, tooOld: false }],
      done: [],
    };
    backend.routes.set(`/machines/${PEER_ID}/revoke-everywhere`, () => ({
      status: 200,
      body: machinesView({
        enabled: true,
        peers: [two()[1]],
        revocations: [revoked],
      }),
    }));
    const el = await render(machinesView({ enabled: true, peers: two() }));
    const card = el.querySelector('.card.peer')!;
    button(card, 'Revoke everywhere')!.click();
    await settle();
    expect(backend.calls).not.toContain(`POST /machines/${PEER_ID}/revoke-everywhere`);
    button(card, 'Tap again to revoke everywhere')!.click();
    await settle();
    expect(backend.calls).toContain(`POST /machines/${PEER_ID}/revoke-everywhere`);
    expect(text(el.querySelector('.notice.ok'))).toBe(
      'Revoked DESKTOP-PC here. Laptop will revoke it when they next connect.',
    );
    const rev = el.querySelector('.card.revocations')!;
    expect(text(rev)).toContain('DESKTOP-PC');
    expect(text(rev)).toContain('Revoked here; still to be told: Laptop (offline).');
  });

  it('offers no Revoke everywhere with a single paired computer', async () => {
    asDesktop();
    const el = await render(machinesView({ enabled: true, peers: [peer()] }));
    expect(button(el, 'Revoke everywhere')).toBeUndefined();
  });

  it('offers to introduce two computers that are not paired with each other', async () => {
    asDesktop();
    backend.routes.set('/machines/introduce', () => ({
      status: 200,
      body: {
        id: 'in_0123456789abcdef',
        host: { machine: PEER_ID, name: 'DESKTOP-PC', state: 'offered', detail: null },
        joiner: { machine: OTHER_ID, name: 'Laptop', state: 'offered', detail: null },
      },
    }));
    const el = await render(machinesView({ enabled: true, peers: two() }));
    const offer = el.querySelector('mc-introduce-offer')!;
    expect(text(offer)).toContain('Also pair Laptop with DESKTOP-PC?');
    button(offer, 'Introduce them')!.click();
    await settle();
    expect(backend.bodyOf('POST /machines/introduce')).toEqual({
      machine: OTHER_ID,
      with: PEER_ID,
    });
    expect(text(offer)).toContain(
      'Introduced: confirm the six digits on Laptop and on DESKTOP-PC.',
    );
  });

  it('offers nothing for two computers already paired with each other', async () => {
    asDesktop();
    const [a, b] = two();
    const el = await render(
      machinesView({
        enabled: true,
        peers: [{ ...a, pairedWith: [b.id] }, b],
      }),
    );
    expect(el.querySelector('mc-introduce-offer')?.textContent?.trim() ?? '').toBe('');
  });

  it('shows an introduction, and pairs only when this desktop accepts it', async () => {
    asDesktop();
    const intro: Introduction = {
      id: 'in_0123456789abcdef',
      from: OTHER_ID,
      fromName: 'MacBook',
      role: 'host',
      other: { id: 'c'.repeat(32), name: 'Build box', os: 'linux', key: '1A2B 3C4D 5E6F 7A8B' },
      at: Date.now(),
      expiresAt: Date.now() + 600_000,
      state: 'offered',
      detail: null,
      pairing: false,
    };
    backend.routes.set('/machines/introductions/in_0123456789abcdef', (_m, body) => ({
      status: 200,
      body: machinesView({
        enabled: true,
        peers: [peer()],
        introductions: [
          {
            ...intro,
            state: (body as { accept: boolean }).accept ? 'accepted' : 'declined',
            detail: (body as { accept: boolean }).accept ? null : 'You declined.',
          },
        ],
      }),
    }));
    const el = await render(
      machinesView({ enabled: true, peers: [peer()], introductions: [intro] }),
    );
    const card = el.querySelector('.card.intros')!;
    expect(text(card)).toContain('MacBook introduces Build box: pair this computer with it?');
    expect(text(card)).toContain('Linux · 1A2B 3C4D 5E6F 7A8B');
    button(card, 'Not now')!.click();
    await settle();
    expect(backend.bodyOf('POST /machines/introductions/in_0123456789abcdef')).toEqual({
      accept: false,
    });
    expect(text(el.querySelector('.card.intros'))).toContain('You declined.');
  });

  it('only shows introductions on a phone', async () => {
    asPhoneOf('macos');
    const el = await render(
      machinesView({
        enabled: true,
        peers: two(),
        introductions: [
          {
            id: 'in_0123456789abcdef',
            from: OTHER_ID,
            fromName: 'MacBook',
            role: 'joiner',
            other: { id: 'c'.repeat(32), name: 'Build box', os: 'linux', key: '1A2B' },
            at: Date.now(),
            expiresAt: Date.now() + 600_000,
            state: 'offered',
            detail: null,
            pairing: false,
          },
        ],
      }),
    );
    expect(text(el.querySelector('.card.intros'))).toContain('MacBook introduces Build box');
    for (const label of ['Pair with', 'Not now', 'Introduce them', 'Revoke everywhere']) {
      expect(button(el, label), label).toBeUndefined();
    }
  });
});
