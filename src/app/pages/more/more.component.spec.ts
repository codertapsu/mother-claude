import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { MachinesService } from '../../core/machines.service';
import {
  FakeBackend,
  flush,
  serveAccounts,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import { MOCK_ACCOUNT_WORK, buildAccounts } from '../../core/mock/fixtures';
import { setActiveMock } from '../../core/mock/mock-api';
import { PlanUsage } from '../../core/models';
import { PushService } from '../../core/push.service';
import { RealtimeService } from '../../core/realtime.service';
import { ThemeService } from '../../core/theme.service';
import { machinesView, peer } from '../machines/machines.testing';
import { MoreComponent } from './more.component';

const H = 3_600_000;

const usage = (session: number, week = 18): PlanUsage =>
  ({
    available: true,
    windows: [
      { key: 'five_hour', label: 'Current session', pct: session, resetsAt: Date.now() + 2 * H },
      { key: 'seven_day', label: 'Weekly · all models', pct: week, resetsAt: Date.now() + 70 * H },
    ],
    fetchedAt: Date.now(),
    source: 'cache',
    refreshing: false,
    intervalSecs: 300,
  }) as PlanUsage;

describe('MoreComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<MoreComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
  });

  afterEach(() => {
    fixture?.destroy();
    setActiveMock(null);
  });

  const text = (e: Element | null | undefined) =>
    (e?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const render = async () => {
    fixture = TestBed.createComponent(MoreComponent);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    return fixture.nativeElement as HTMLElement;
  };
  const rows = (el: HTMLElement) => [...el.querySelectorAll<HTMLAnchorElement>('a.entry')];
  const line = (el: HTMLElement, label: string) =>
    text(
      rows(el)
        .find((r) => text(r.querySelector('.label')) === label)
        ?.querySelector('.state'),
    );

  it('lists what you look after, then Help, as the sidebar orders them', async () => {
    const el = await render();
    expect(rows(el).map((r) => text(r.querySelector('.label')))).toEqual([
      'Computers',
      'Accounts',
      'Settings',
      'Help',
    ]);
    expect(rows(el).map((r) => r.getAttribute('href'))).toEqual([
      '/computers',
      '/accounts',
      '/settings',
      '/help',
    ]);
    expect(line(el, 'Help')).toBe('How to use every feature, step by step');
    // A phone reads Machines' summary here (the desktop's shell keeps it).
    expect(backend.calls).toContain('GET /machines');
  });

  it('says which computers are online and what needs fixing, with the red dot', async () => {
    backend.routes.set('/machines', () => ({
      status: 200,
      body: machinesView({ enabled: true, peers: [peer()] }),
    }));
    const el = await render();
    expect(line(el, 'Computers')).toBe('DESKTOP-PC online');
    expect(rows(el)[0].querySelector('.count-badge.dot')).toBeNull();

    TestBed.inject(MachinesService).apply(
      machinesView({
        enabled: true,
        peers: [
          peer(),
          peer({ id: 'e'.repeat(32), name: 'BUILD-BOX', online: false, lastSeen: Date.now() }),
        ],
        health: { state: 'blocker', blockers: 2, warnings: 0, at: Date.now() },
      }),
    );
    fixture.detectChanges();
    expect(line(el, 'Computers')).toBe('DESKTOP-PC online · 2 to fix');
    expect(rows(el)[0].querySelector('.count-badge.dot')).not.toBeNull();
    expect(rows(el)[0].getAttribute('aria-label')).toBe('Computers, needs attention');

    const box = (online: boolean) =>
      peer({ id: 'e'.repeat(32), name: 'BUILD-BOX', online, lastSeen: Date.now() });
    const third = peer({ id: 'd'.repeat(32), name: 'NAS', online: false, lastSeen: Date.now() });
    TestBed.inject(MachinesService).apply(
      machinesView({ enabled: true, peers: [peer(), box(true)] }),
    );
    fixture.detectChanges();
    expect(line(el, 'Computers')).toBe('2 computers online');
    TestBed.inject(MachinesService).apply(
      machinesView({ enabled: true, peers: [peer(), box(true), third] }),
    );
    fixture.detectChanges();
    expect(line(el, 'Computers')).toBe('2 of 3 computers online');

    TestBed.inject(MachinesService).apply(machinesView());
    fixture.detectChanges();
    expect(line(el, 'Computers')).toBe('Not set up yet');
    TestBed.inject(MachinesService).apply(machinesView({ setupStep: 3 }));
    fixture.detectChanges();
    expect(line(el, 'Computers')).toBe('Setup: step 3 of 7');
    TestBed.inject(MachinesService).apply(
      machinesView({ enabled: false, peers: [peer({ online: false })] }),
    );
    fixture.detectChanges();
    expect(line(el, 'Computers')).toBe('No computer online · Off');
  });

  it("says how much of each account's plan is used", async () => {
    const rt = TestBed.inject(RealtimeService);
    rt.mergePlanUsage(usage(42));
    const el = await render();
    expect(line(el, 'Accounts')).toBe('Session 42% · Week 18%');

    serveAccounts(backend, buildAccounts(Date.now(), true));
    rt.mergePlanUsage(usage(71));
    rt.mergeAccountUsages({ accounts: { [MOCK_ACCOUNT_WORK]: usage(18) } });
    fixture.detectChanges();
    expect(line(el, 'Accounts')).toBe('Default 71% · Work 18%');
  });

  it('says whether notifications are on, and the theme', async () => {
    TestBed.inject(PushService).state.set('on');
    TestBed.inject(ThemeService).set('dark');
    const el = await render();
    expect(line(el, 'Settings')).toBe('Notifications on · Dark');
    TestBed.inject(PushService).state.set('denied');
    TestBed.inject(ThemeService).set('light');
    fixture.detectChanges();
    expect(line(el, 'Settings')).toBe('Notifications blocked · Light');
  });
});
