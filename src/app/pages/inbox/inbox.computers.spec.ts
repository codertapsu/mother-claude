import { Component } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { AttentionService } from '../../core/attention.service';
import { Introduction, Revocation } from '../../core/machines';
import { MachinesService } from '../../core/machines.service';
import { machinesView, peer } from '../../core/mock/attention.testing';
import {
  FakeBackend,
  asDesktop,
  asPhoneOf,
  flush,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { InboxComponent } from './inbox.component';

@Component({ template: '' })
class BlankComponent {}

const OTHER_ID = 'e'.repeat(32);

function intro(over: Partial<Introduction> = {}): Introduction {
  return {
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
    ...over,
  };
}

function revocation(over: Partial<Revocation> = {}): Revocation {
  return {
    machine: 'd'.repeat(32),
    name: 'OLD-LAPTOP',
    key: '9F9F 0000 1111 2222',
    at: Date.now() - 60_000,
    pending: [{ id: OTHER_ID, name: 'Laptop', online: false, tooOld: false }],
    done: [],
    ...over,
  };
}

/**
 * What your other computers leave waiting, in the Inbox (docs/UI-REDESIGN.md
 * §6.4, stage 3): an introduction (*Pair with …?*), a setting another
 * computer suggests, a computer revoked everywhere that some still trust.
 */
describe('InboxComponent: what your computers leave waiting', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<InboxComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([{ path: '**', component: BlankComponent }])]);
    backend.routes.set('/inbox', () => ({ status: 200, body: { items: [] } }));
    backend.routes.set('/data/jobs', () => ({ status: 200, body: { jobs: [] } }));
    backend.routes.set('/devices', () => ({ status: 200, body: { devices: [], pending: [] } }));
  });

  afterEach(() => {
    fixture?.destroy();
    setActiveMock(null);
    localStorage.removeItem('mc_inbox_dismissed');
  });

  const text = (e: Element | null | undefined) =>
    (e?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const settle = async () => {
    for (let i = 0; i < 4; i++) {
      await flush();
      fixture.detectChanges();
    }
  };
  const render = async () => {
    fixture = TestBed.createComponent(InboxComponent);
    fixture.detectChanges();
    await settle();
    return fixture.nativeElement as HTMLElement;
  };
  const items = (el: Element | null, kind: string) => [
    ...(el?.querySelectorAll<HTMLElement>(`mc-attention-card[data-kind="${kind}"]`) ?? []),
  ];
  const title = (card: Element | undefined) => text(card?.querySelector('.title'));
  const why = (card: Element | undefined) => text(card?.querySelector('.why'));
  const button = (el: Element | undefined, label: string) =>
    [...(el?.querySelectorAll<HTMLElement>('button, a') ?? [])].find((b) => text(b) === label);

  it('asks the desktop to pair a computer another one introduces, and counts it', async () => {
    asDesktop();
    backend.routes.set('/machines/introductions/in_0123456789abcdef', (_m, body) => ({
      status: 200,
      body: machinesView({
        enabled: true,
        peers: [peer()],
        introductions: [
          {
            ...intro(),
            state: (body as { accept: boolean }).accept ? 'accepted' : 'declined',
            detail: (body as { accept: boolean }).accept ? null : 'You declined.',
          },
        ],
      }),
    }));
    TestBed.inject(MachinesService).apply(
      machinesView({ enabled: true, peers: [peer()], introductions: [intro()] }),
    );
    const el = await render();
    const [card] = items(el, 'intro');
    expect(title(card)).toBe('MacBook introduces Build box');
    expect(text(card.querySelector('.meta'))).toBe('Linux · 1A2B 3C4D 5E6F 7A8B');
    expect(why(card)).toContain('Pair this computer with it?');
    expect(card.closest('section')?.getAttribute('aria-labelledby')).toBe('inbox-needs');
    expect(TestBed.inject(AttentionService).pendingCount()).toBe(1);

    button(card, 'Not now')!.click();
    await settle();
    expect(backend.bodyOf('POST /machines/introductions/in_0123456789abcdef')).toEqual({
      accept: false,
    });
    // Declined: nothing waits any more.
    expect(items(el, 'intro')).toEqual([]);
    expect(TestBed.inject(AttentionService).pendingCount()).toBe(0);
  });

  it('shows a pairing under way, to open again', async () => {
    asDesktop();
    TestBed.inject(MachinesService).apply(
      machinesView({
        enabled: true,
        peers: [peer()],
        introductions: [intro({ state: 'accepted', pairing: true })],
      }),
    );
    const el = await render();
    const [card] = items(el, 'intro');
    expect(text(card)).toContain('Pairing…');
    expect(button(card, 'Show the pairing')).toBeDefined();
  });

  it('leaves an introduction to the desktop on a phone, outside the badge', async () => {
    asPhoneOf('macos');
    TestBed.inject(MachinesService).apply(
      machinesView({ enabled: true, peers: [peer()], introductions: [intro()] }),
    );
    const el = await render();
    const [card] = items(el, 'intro');
    expect(title(card)).toBe('MacBook introduces Build box');
    expect(text(card)).toContain('Answer on the desktop app.');
    expect(button(card, 'Not now')).toBeUndefined();
    expect(TestBed.inject(AttentionService).pendingCount()).toBe(0);
  });

  it('offers the setting another computer uses, answered on the desktop', async () => {
    asDesktop();
    backend.routes.set('/machines/settings/offer', () => ({
      status: 200,
      body: machinesView({ enabled: true, peers: [peer()] }),
    }));
    TestBed.inject(MachinesService).apply(
      machinesView({
        enabled: true,
        peers: [peer()],
        offer: {
          from: peer().id,
          fromName: 'DESKTOP-PC',
          relay: { mode: 'none', url: null },
          at: Date.now(),
        },
      }),
    );
    const el = await render();
    const [card] = items(el, 'offer');
    expect(title(card)).toBe('DESKTOP-PC suggests its settings');
    expect(why(card)).toBe(
      'DESKTOP-PC uses another relay setting: No relay. Two computers on different relays may never meet.',
    );
    expect(TestBed.inject(AttentionService).pendingCount()).toBe(1);
    button(card, 'Use it here too')!.click();
    await settle();
    expect(backend.bodyOf('POST /machines/settings/offer')).toEqual({ accept: true });
    expect(items(el, 'offer')).toEqual([]);
  });

  it('says a computer revoked everywhere is still trusted by one away, until put away', async () => {
    TestBed.inject(MachinesService).apply(
      machinesView({ enabled: true, peers: [peer()], revocations: [revocation()] }),
    );
    const el = await render();
    const [card] = items(el, 'revoked');
    expect(card.closest('section')?.getAttribute('aria-labelledby')).toBe('inbox-fix');
    expect(title(card)).toBe('OLD-LAPTOP is not revoked everywhere yet');
    expect(why(card)).toBe(
      'Revoked here; still to be told: Laptop (offline). Each one revokes it when it next connects: until then it lets it in.',
    );
    button(card, 'Dismiss')!.click();
    await settle();
    expect(items(el, 'revoked')).toEqual([]);

    // Once every computer took it, nothing is left to fix.
    TestBed.inject(MachinesService).apply(
      machinesView({
        enabled: true,
        peers: [peer()],
        revocations: [
          revocation({
            machine: 'b'.repeat(32),
            pending: [],
            done: [{ id: OTHER_ID, name: 'Laptop' }],
          }),
        ],
      }),
    );
    await settle();
    expect(items(el, 'revoked')).toEqual([]);
  });
});
