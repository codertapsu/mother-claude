import { PlanWindow } from './models';
import {
  compactWindows,
  limitName,
  limitWindow,
  normalizeAccountUsages,
  normalizePlanUsage,
  planIntervalLabel,
  primaryWindows,
  windowPct,
  windowSeverity,
  worstSeverity,
} from './plan-usage';

const NOW = 1_000_000_000_000;
const w = (over: Partial<PlanWindow> & Pick<PlanWindow, 'key'>): PlanWindow => ({
  label: over.key,
  pct: 10,
  ...over,
});

describe('plan usage helpers', () => {
  it('reads a plan usage tolerantly', () => {
    const u = normalizePlanUsage({
      available: true,
      windows: [
        { key: 'five_hour', label: 'Current session', pct: 42.4, resetsAt: NOW + 1, severity: 'x' },
        { key: 'broken', label: 'No pct' },
        null,
        { key: 'seven_day', pct: '18' },
      ],
      extraUsage: { enabled: true, usedMinor: 1240, currency: 'EUR', decimals: 2 },
      fetchedAt: NOW,
      source: 'live',
      refreshing: 'yes',
      intervalSecs: 180,
      error: '   ',
      spend: { secret: 1 },
    });
    expect(u).toEqual({
      available: true,
      windows: [{ key: 'five_hour', label: 'Current session', pct: 42.4, resetsAt: NOW + 1 }],
      extraUsage: { enabled: true, usedMinor: 1240, currency: 'EUR', decimals: 2 },
      fetchedAt: NOW,
      source: 'live',
      refreshing: false,
      intervalSecs: 180,
    });
    expect(normalizePlanUsage(null)).toBeNull();
    expect(normalizePlanUsage('nope')).toBeNull();
    expect(normalizePlanUsage({})).toEqual({
      available: false,
      windows: [],
      refreshing: false,
      intervalSecs: 300,
    });
  });

  it('keeps when a running session last moved the windows, and never guesses a currency', () => {
    const u = normalizePlanUsage({
      available: true,
      windows: [],
      fetchedAt: NOW - 1_000,
      streamAt: NOW,
      extraUsage: { enabled: true, usedMinor: 1240, decimals: 2 },
    });
    expect(u?.streamAt).toBe(NOW);
    expect(u?.extraUsage).toEqual({ enabled: true, usedMinor: 1240, decimals: 2 });
    expect(normalizePlanUsage({ available: true, streamAt: 'soon' })?.streamAt).toBeUndefined();
  });

  it("keeps the server's decimal places for extra usage, never a clamped exponent", () => {
    const decimals = (d: unknown) =>
      normalizePlanUsage({
        available: true,
        extraUsage: { enabled: true, usedMinor: 1_234_567, currency: 'XYZ', decimals: d },
      })?.extraUsage?.decimals;
    expect(decimals(0)).toBe(0);
    expect(decimals(3)).toBe(3);
    // The server accepts up to 6: dividing by 10^4 instead would show 100x too much.
    expect(decimals(6)).toBe(6);
    // Anything else is dropped for the default, not squeezed into range.
    expect(decimals(7)).toBe(2);
    expect(decimals(2.5)).toBe(2);
    expect(decimals(-1)).toBe(2);
    expect(decimals('3')).toBe(2);
  });

  it('counts a window whose reset time has passed as 0 %', () => {
    const past = w({ key: 'five_hour', pct: 97, resetsAt: NOW - 1, severity: 'critical' });
    expect(windowPct(past, NOW)).toBe(0);
    expect(windowSeverity(past, NOW)).toBe('normal');
    expect(windowPct(w({ key: 'five_hour', pct: 42.6, resetsAt: NOW + 1 }), NOW)).toBe(43);
  });

  it('rates severity by percentage or by the server, whichever is worse', () => {
    expect(windowSeverity(w({ key: 'a', pct: 74 }), NOW)).toBe('normal');
    expect(windowSeverity(w({ key: 'a', pct: 75 }), NOW)).toBe('warning');
    expect(windowSeverity(w({ key: 'a', pct: 90 }), NOW)).toBe('critical');
    expect(windowSeverity(w({ key: 'a', pct: 20, severity: 'warning' }), NOW)).toBe('warning');
    expect(
      worstSeverity([w({ key: 'a', pct: 10 }), w({ key: 'b', pct: 80 }), w({ key: 'c' })], NOW),
    ).toBe('warning');
  });

  it('picks the session and the all-models week for compact views', () => {
    const list = [
      w({ key: 'seven_day_fable', label: 'Weekly · Fable', pct: 81 }),
      w({ key: 'seven_day', label: 'Weekly · all models' }),
      w({ key: 'five_hour', label: 'Current session' }),
    ];
    expect(primaryWindows(list).map((p) => [p.window.key, p.short])).toEqual([
      ['five_hour', 'Session'],
      ['seven_day', 'Week'],
    ]);
    // Keys from the CLI's limits rows work the same way.
    expect(
      primaryWindows([w({ key: 'weekly_all' }), w({ key: 'session' })]).map((p) => p.short),
    ).toEqual(['Session', 'Week']);
    // A window near its limit is added, named by its scope.
    expect(compactWindows(list, NOW).map((p) => p.short)).toEqual(['Session', 'Week', 'Fable']);
    // Unknown keys: the first two, as the server ordered them.
    expect(
      primaryWindows([w({ key: 'x', label: 'Mystery · a' }), w({ key: 'y' }), w({ key: 'z' })]).map(
        (p) => p.short,
      ),
    ).toEqual(['Mystery', 'y']);
  });

  it('names intervals', () => {
    expect(planIntervalLabel(300)).toBe('Every 5 min');
    expect(planIntervalLabel(0)).toBe('Manually only');
    expect(planIntervalLabel(420)).toBe('Every 7 min');
    expect(planIntervalLabel(undefined)).toBe('Every 5 min');
  });

  it('reads the other accounts’ usage from GET /usage, dropping what is not usage', () => {
    const m = normalizeAccountUsages({
      available: true,
      windows: [],
      accounts: {
        acc_5f1c9e2b7a40: { available: true, windows: [{ key: 'five_hour', pct: 12 }] },
        acc_9b02d3e6c1f8: 'nope',
      },
    });
    expect([...m.keys()]).toEqual(['acc_5f1c9e2b7a40']);
    expect(m.get('acc_5f1c9e2b7a40')?.windows[0].pct).toBe(12);
    // Only the Default (or an older server): no `accounts` at all.
    expect(normalizeAccountUsages({ available: true, windows: [] }).size).toBe(0);
    expect(normalizeAccountUsages({ accounts: [1, 2] }).size).toBe(0);
    expect(normalizeAccountUsages(null).size).toBe(0);
  });

  it('finds the limit an account has reached, the 5-hour one first', () => {
    const week = w({ key: 'seven_day', label: 'Weekly · all models', pct: 100, resetsAt: NOW + 9 });
    const fable = w({ key: 'seven_day_fable', label: 'Weekly · Fable', pct: 104 });
    const session = w({ key: 'five_hour', label: 'Current session', pct: 100, resetsAt: NOW + 5 });
    expect(limitWindow([w({ key: 'five_hour', pct: 99.4 })], NOW)).toBeNull();
    expect(limitWindow([fable, week, session], NOW)).toBe(session);
    expect(limitWindow([fable, week], NOW)).toBe(week);
    expect(limitWindow([fable], NOW)).toBe(fable);
    // A window whose reset time has passed is not at its limit any more.
    expect(limitWindow([{ ...session, resetsAt: NOW - 1 }], NOW)).toBeNull();
    expect(limitName(session)).toBe('5-hour limit');
    expect(limitName(week)).toBe('weekly limit');
    expect(limitName(fable)).toBe('weekly Fable limit');
    expect(limitName(w({ key: 'daily', label: 'Daily · Opus' }))).toBe('Daily · Opus limit');
  });
});
