import { ComponentFixture, TestBed } from '@angular/core/testing';

import { hostOsSignal } from '../../core/i18n/i18n';
import { SyncPair, SyncView } from '../../core/machines';
import {
  FakeBackend,
  asDesktop,
  asPhoneOf,
  flush,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { SyncCardComponent } from './sync-card.component';
import { PEER_ID, machinesFrames } from './machines.testing';

const ID = 'sy_0123456789abcdef';

function pair(over: Partial<SyncPair> = {}): SyncPair {
  return {
    id: ID,
    machine: PEER_ID,
    machineName: 'DESKTOP-PC',
    machineOs: 'windows',
    online: true,
    folder: '/Users/you/dev/app',
    name: 'app',
    there: 'D:\\dev\\app',
    here: { on: true },
    peer: { on: true, asked: false },
    status: { state: 'inSync', at: Date.now() - 60_000 },
    last: {
      at: Date.now() - 60_000,
      step: 4,
      branch: 'main',
      head: 'a1b2c3d4e5',
      direction: 'in',
      files: 2,
    },
    conflict: null,
    kept: [],
    createdAt: Date.now() - 86_400_000,
    updatedAt: Date.now() - 60_000,
    ...over,
  };
}

const conflicted = (): SyncPair =>
  pair({
    status: {
      state: 'paused',
      reason: 'conflict',
      detail: 'app changed on both computers: choose which one’s to keep.',
      since: Date.now(),
    },
    conflict: {
      at: Date.now(),
      first: false,
      here: {
        files: 3,
        headMoved: false,
        branchSwitched: false,
        branch: 'main',
        head: 'a1b2c3d4e5',
      },
      there: {
        files: 1,
        headMoved: true,
        branchSwitched: false,
        branch: 'main',
        head: 'f6e5d4c3b2',
      },
    },
  });

describe('SyncCardComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<SyncCardComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend);
  });

  afterEach(() => {
    fixture?.destroy();
    setActiveMock(null);
    hostOsSignal.set(undefined);
  });

  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const buttons = (el: Element) => [...el.querySelectorAll<HTMLButtonElement>('button')];
  const button = (el: Element, label: string) => buttons(el).find((b) => text(b).startsWith(label));
  const settle = async () => {
    for (let i = 0; i < 3; i++) {
      await flush();
      fixture.detectChanges();
    }
  };
  const render = async (pairs: SyncPair[], readonly = false) => {
    const view: SyncView = { pairs, watching: true };
    backend.routes.set('/machines/sync', () => ({ status: 200, body: view }));
    fixture = TestBed.createComponent(SyncCardComponent);
    fixture.componentRef.setInput('readonly', readonly);
    fixture.detectChanges();
    await settle();
    return fixture.nativeElement as HTMLElement;
  };

  it('lists each synced project with how it stands, Sync now and Turn off', async () => {
    asDesktop({ hostOs: 'macos' });
    hostOsSignal.set('macos');
    const waiting = pair({
      id: 'sy_1111111111111111',
      name: 'website',
      folder: '/Users/you/dev/website',
      peer: { on: false, asked: true },
      status: { state: 'waiting' },
      last: null,
    });
    const el = await render([pair(), waiting]);
    const [inSync, asked] = [...el.querySelectorAll('.list > li')];
    expect(text(inSync.querySelector('.name'))).toBe('app');
    expect(text(inSync)).toContain('with DESKTOP-PC');
    expect(text(inSync.querySelector('.badge'))).toBe('In sync');
    expect(text(inSync)).toContain('/Users/you/dev/app ↔ D:\\dev\\app');
    expect(text(inSync)).toContain('Last: 2 files from DESKTOP-PC');
    expect(text(asked.querySelector('.badge'))).toBe('Waiting for DESKTOP-PC');
    expect(text(asked)).toContain(
      'DESKTOP-PC shows the request to whoever is at it: nothing syncs until it is confirmed there.',
    );
    expect(button(asked, 'Sync now')).toBeUndefined();

    backend.routes.set(`/machines/sync/${ID}/now`, () => ({
      status: 200,
      body: pair({ last: { ...pair().last!, direction: 'out', files: 1 } }),
    }));
    button(inSync, 'Sync now')!.click();
    await settle();
    expect(backend.calls).toContain(`POST /machines/sync/${ID}/now`);
    expect(text(el.querySelector('.list > li'))).toContain(
      'Last: 1 file from this Mac to DESKTOP-PC',
    );

    backend.routes.set('/machines/sync', (method) => ({
      status: 200,
      body: { pairs: method === 'PUT' ? [waiting] : [pair(), waiting], watching: true },
    }));
    button(el.querySelector('.list > li')!, 'Turn off')!.click();
    await settle();
    expect(backend.bodyOf('PUT /machines/sync')).toEqual({
      project: '/Users/you/dev/app',
      machine: PEER_ID,
      on: false,
    });
    expect(el.querySelectorAll('.list > li').length).toBe(1);
  });

  it('says why a project waits', async () => {
    asDesktop({ hostOs: 'macos' });
    hostOsSignal.set('macos');
    let n = 0;
    const paused = (reason: 'offline' | 'noGrant' | 'machinesOff' | 'moveRunning' | 'error') =>
      pair({
        id: `sy_${String((n += 1)).padStart(16, '0')}`,
        status: { state: 'paused', reason, detail: 'git exploded', since: Date.now() },
      });
    const off = pair({
      id: 'sy_2222222222222222',
      peer: { on: false, asked: false },
      status: { state: 'off', detail: 'Turned off on DESKTOP-PC.' },
    });
    const el = await render([
      paused('offline'),
      paused('noGrant'),
      paused('machinesOff'),
      paused('moveRunning'),
      paused('error'),
      off,
    ]);
    const rows = [...el.querySelectorAll('.list > li')].map((li) =>
      text(li.querySelector('.detail')),
    );
    expect(rows).toEqual([
      'Waits until DESKTOP-PC is back online.',
      'Waits: each computer must give the other the Move permission, as it writes into its project folder.',
      'Waits until both computers have Computers on.',
      'Waits while a session working in this project moves between computers.',
      'Waits: git exploded',
      'Turned off on DESKTOP-PC.',
    ]);
    const last = el.querySelectorAll('.list > li')[5];
    expect(text(last.querySelector('.badge'))).toBe('Off');
    expect(button(last, 'Turn on again')).toBeDefined();
    expect(button(last, 'Remove')).toBeDefined();
  });

  it('shows both sides of a conflict and keeps one only on a second tap', async () => {
    asDesktop({ hostOs: 'macos' });
    hostOsSignal.set('macos');
    const el = await render([conflicted()]);
    const row = el.querySelector('.list > li')!;
    expect(text(row.querySelector('.badge'))).toBe('Changed on both');
    expect(text(row.querySelector('.detail'))).toBe(
      'Both computers changed it since it was last synced: choose which one’s to keep.',
    );
    const sides = [...row.querySelectorAll('.sides dt, .sides dd')].map((x) => text(x));
    expect(sides).toEqual([
      'This Mac',
      '3 files changed',
      'DESKTOP-PC',
      '1 file changed · new commits',
    ]);
    expect(text(row.querySelector('.conflict'))).toContain(`refs/mother-claude/sync/${ID}/kept-…`);

    const kept = {
      ref: `refs/mother-claude/sync/${ID}/kept-20261112T101500-3fa2`,
      side: 'there' as const,
      commit: null,
      files: 1,
      at: Date.now(),
    };
    backend.routes.set(`/machines/sync/${ID}/resolve`, () => ({
      status: 200,
      body: pair({ kept: [kept] }),
    }));
    button(row, 'Keep this Mac’s')!.click();
    await settle();
    expect(backend.calls).not.toContain(`POST /machines/sync/${ID}/resolve`);
    expect(button(row, 'Tap again to keep this Mac’s')).toBeDefined();
    button(row, 'Tap again to keep this Mac’s')!.click();
    await settle();
    expect(backend.bodyOf(`POST /machines/sync/${ID}/resolve`)).toEqual({ keep: 'here' });
    const after = el.querySelector('.list > li')!;
    expect(text(after.querySelector('.badge'))).toBe('In sync');
    expect(after.querySelector('.conflict')).toBeNull();
    expect(text(after.querySelector('.kept'))).toContain('1 state kept');
    expect(text(after.querySelector('.kept'))).toContain(`${kept.ref} · on DESKTOP-PC`);
  });

  it('keeps the other computer’s side when that is the choice', async () => {
    asDesktop({ hostOs: 'windows' });
    hostOsSignal.set('windows');
    const el = await render([conflicted()]);
    backend.routes.set(`/machines/sync/${ID}/resolve`, () => ({ status: 200, body: pair() }));
    const row = el.querySelector('.list > li')!;
    button(row, 'Keep DESKTOP-PC’s')!.click();
    await settle();
    button(row, 'Tap again to keep DESKTOP-PC’s')!.click();
    await settle();
    expect(backend.bodyOf(`POST /machines/sync/${ID}/resolve`)).toEqual({ keep: 'there' });
  });

  it('is read-only on a phone', async () => {
    asPhoneOf('macos');
    hostOsSignal.set('macos');
    const el = await render(
      [conflicted(), pair({ id: 'sy_3333333333333333', name: 'site' })],
      true,
    );
    expect(el.querySelectorAll('.list > li').length).toBe(2);
    expect(text(el.querySelector('.sides'))).toContain('This Mac');
    expect(buttons(el)).toEqual([]);
  });

  it('follows the sync frames', async () => {
    const send = await machinesFrames(backend);
    const el = await render([pair()]);
    expect(text(el.querySelector('.badge'))).toBe('In sync');
    backend.routes.set('/machines/sync', () => ({
      status: 200,
      body: { pairs: [conflicted()], watching: true },
    }));
    send('sync');
    await settle();
    expect(text(el.querySelector('.badge'))).toBe('Changed on both');
  });
});
