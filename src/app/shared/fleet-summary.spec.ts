import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { FakeBackend, serveAccounts, useFakeServer } from '../core/mock/fake-server.testing';
import { MOCK_ACCOUNT_WORK, buildAccounts } from '../core/mock/fixtures';
import { setActiveMock } from '../core/mock/mock-api';
import { PlanUsage, SessionSummary } from '../core/models';
import { storageGet, storageRemove } from '../core/platform';
import { RealtimeService } from '../core/realtime.service';
import { FleetSummaryComponent, compactPlan, fleetCounts } from './fleet-summary.component';

const s = (over: Record<string, unknown>): SessionSummary =>
  ({ id: 'x', live: true, status: 'idle', pending: [], ...over }) as unknown as SessionSummary;

describe('fleetCounts', () => {
  const midnight = 1_000;

  it('counts like the server fleet summary', () => {
    const c = fleetCounts(
      [
        s({ status: 'working', agents: { running: 2, backgroundRunning: 3 } }),
        s({ status: 'starting' }),
        s({ live: false, status: 'working' }),
        s({ status: 'waiting', pending: [{ answerable: false }] }),
        s({ status: 'waiting', pending: [{ answerable: false }, { answerable: true }] }),
      ],
      midnight,
    );
    expect(c.working).toBe(1);
    expect(c.agentsRunning).toBe(2);
    expect(c.needYou).toBe(1);
  });

  it("sums each session's costToday, and falls back to tokens when unpriced", () => {
    const priced = fleetCounts(
      [
        // Lifetime cost is not today's: only what was priced since midnight.
        s({ lastActivityAt: 2_000, costUsd: 5, costToday: 1.004 }),
        s({ lastActivityAt: 2_000, costUsd: 2, costToday: 2 }),
        s({ lastActivityAt: 500, startedAt: 2_000, costUsd: 9 }),
      ],
      midnight,
    );
    expect(priced.costToday).toBe(3);
    expect(priced.todaySessions).toBe(2);
    expect(priced.priced).toBeTrue();

    const unpriced = fleetCounts(
      [s({ lastActivityAt: 2_000, usage: { input: 10, output: 5 } })],
      midnight,
    );
    expect(unpriced.priced).toBeFalse();
    expect(unpriced.tokensToday).toBe(15);
  });
});

describe('compactPlan', () => {
  const NOW = 1_000_000_000_000;
  const H = 3_600_000;
  const u = (over: Partial<PlanUsage> = {}): PlanUsage => ({
    available: true,
    windows: [
      { key: 'five_hour', label: 'Current session', pct: 42, resetsAt: NOW + 2 * H },
      { key: 'seven_day', label: 'Weekly · all models', pct: 18, resetsAt: NOW + 80 * H },
      { key: 'seven_day_opus', label: 'Weekly · Opus', pct: 12, resetsAt: NOW + 80 * H },
    ],
    refreshing: false,
    intervalSecs: 300,
    ...over,
  });

  it('shows the session (with its countdown) and the week', () => {
    const p = compactPlan(u(), NOW);
    expect(p?.meters.map((m) => [m.short, m.pct, m.left])).toEqual([
      ['Session', 42, '2h'],
      ['Week', 18, '3d 8h'],
    ]);
    expect(p?.tone).toBe('accent');
    expect(p?.label).toBe(
      'Plan usage: Current session 42%, resets in 2h; Weekly · all models 18%, resets in 3d 8h; Weekly · Opus 12%, resets in 3d 8h. Show details',
    );
  });

  it('adds a window near its limit and takes the worst tone', () => {
    const windows = u().windows.map((w) => (w.key === 'seven_day_opus' ? { ...w, pct: 93 } : w));
    const p = compactPlan(u({ windows }), NOW);
    expect(p?.meters.map((m) => [m.short, m.tone])).toEqual([
      ['Session', 'accent'],
      ['Week', 'accent'],
      ['Opus', 'red'],
    ]);
    expect(p?.tone).toBe('red');
  });

  it('is hidden without plan usage, and reads 0 % after a reset', () => {
    expect(compactPlan(null, NOW)).toBeNull();
    expect(compactPlan(u({ available: false }), NOW)).toBeNull();
    const later = compactPlan(u(), NOW + 3 * H);
    expect(later?.meters[0]).toEqual(jasmine.objectContaining({ pct: 0, left: '' }));
  });
});

describe('FleetSummaryComponent plan meter', () => {
  let backend: FakeBackend;
  const H = 3_600_000;
  const plan = (pct: number): PlanUsage => ({
    available: true,
    windows: [{ key: 'five_hour', label: 'Current session', pct, resetsAt: Date.now() + H }],
    refreshing: false,
    intervalSecs: 300,
  });

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    TestBed.inject(RealtimeService).mergePlanUsage(plan(42));
  });

  afterEach(() => {
    storageRemove('mc_plan_account');
    setActiveMock(null);
  });

  const render = () => {
    const fixture = TestBed.createComponent(FleetSummaryComponent);
    fixture.detectChanges();
    return { fixture, el: fixture.nativeElement as HTMLElement };
  };
  const planButton = (el: HTMLElement) => el.querySelector('.seg.plan') as HTMLButtonElement;

  it('has no switcher with only the Default account', () => {
    serveAccounts(backend, buildAccounts(Date.now(), false));
    const { el } = render();
    expect(el.querySelector('select')).toBeNull();
    expect(planButton(el).getAttribute('aria-label')).toMatch(/^Plan usage: Current session 42%/);
  });

  it("switches the meter to another account's usage and remembers the pick", () => {
    serveAccounts(backend, buildAccounts(Date.now(), true));
    TestBed.inject(RealtimeService).mergeAccountUsages({
      accounts: { [MOCK_ACCOUNT_WORK]: plan(77) },
    });
    const { fixture, el } = render();
    const sel = el.querySelector('.seg.acct select') as HTMLSelectElement;
    expect(sel).not.toBeNull();
    expect((el.querySelector('.aname')?.textContent ?? '').trim()).toBe('Default');
    expect(planButton(el).textContent).toContain('42%');
    sel.value = MOCK_ACCOUNT_WORK;
    sel.dispatchEvent(new Event('change'));
    fixture.detectChanges();
    expect((el.querySelector('.aname')?.textContent ?? '').trim()).toBe('Work');
    expect(planButton(el).textContent).toContain('77%');
    expect(planButton(el).getAttribute('aria-label')).toMatch(/^Plan usage of Work: /);
    expect(storageGet('mc_plan_account')).toBe(MOCK_ACCOUNT_WORK);
  });
});
