import { WritableSignal, signal } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { FakeBackend, flush, useFakeServer } from '../core/mock/fake-server.testing';
import { setActiveMock } from '../core/mock/mock-api';
import { PlanUsage } from '../core/models';
import { NowService } from '../core/now.service';
import { PlanUsageService } from '../core/plan-usage.service';
import { RealtimeService } from '../core/realtime.service';
import { compactPlan } from './fleet-summary.component';
import { PlanUsageComponent } from './plan-usage.component';
import { formatMinor } from './util';

const NOW = Date.UTC(2026, 8, 29, 9, 0, 0);
const M = 60_000;
const H = 60 * M;

const usage = (over: Partial<PlanUsage> = {}): PlanUsage => ({
  available: true,
  windows: [
    { key: 'five_hour', label: 'Current session', pct: 42, resetsAt: NOW + 2 * H + 10 * M },
    {
      key: 'seven_day',
      label: 'Weekly · all models',
      pct: 91,
      resetsAt: NOW + 76 * H,
      severity: 'critical',
    },
  ],
  fetchedAt: NOW - 2 * M,
  source: 'cache',
  refreshing: false,
  nextRefreshAt: NOW + 3 * M,
  intervalSecs: 300,
  ...over,
});

describe('PlanUsageComponent', () => {
  let backend: FakeBackend;
  let now: WritableSignal<number>;
  let fixture: ComponentFixture<PlanUsageComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    now = signal(NOW);
    useFakeServer(backend, [provideRouter([]), { provide: NowService, useValue: { now } }]);
  });

  afterEach(() => setActiveMock(null));

  const render = (u: PlanUsage | null) => {
    if (u) TestBed.inject(RealtimeService).mergePlanUsage(u);
    fixture = TestBed.createComponent(PlanUsageComponent);
    fixture.detectChanges();
    return fixture.nativeElement as HTMLElement;
  };
  const text = (el: HTMLElement) => (el.textContent ?? '').replace(/\s+/g, ' ');

  it('renders nothing until the server reported plan usage', () => {
    expect(text(render(null)).trim()).toBe('');
  });

  it('says so, without meters, when the account has no plan usage', () => {
    const el = render(usage({ available: false, windows: [] }));
    expect(el.querySelectorAll('[role="meter"]').length).toBe(0);
    expect(text(el)).toContain('no plan usage to show');
    // …and the compact view on the sessions page disappears entirely.
    expect(compactPlan(usage({ available: false }), NOW)).toBeNull();
  });

  it('shows every window as a meter with its reset countdown', () => {
    const el = render(usage());
    const meters = el.querySelectorAll('[role="meter"]');
    expect(meters.length).toBe(2);
    expect(meters[0].getAttribute('aria-label')).toBe('Current session');
    expect(meters[0].getAttribute('aria-valuenow')).toBe('42');
    expect(meters[0].getAttribute('aria-valuetext')).toBe('42% used, resets in 2h 10m');
    expect(text(meters[0] as HTMLElement)).toContain('Resets in 2h 10m');
    // Near the limit: said in words, not only in colour.
    expect(meters[1].getAttribute('data-tone')).toBe('red');
    expect(text(meters[1] as HTMLElement)).toContain('Almost at the limit');
    expect(text(el)).toContain("Updated 2m ago (Claude Code's last reading) · next check in 3m");
  });

  it('counts down on the shared clock and treats a passed reset as 0 %', () => {
    const el = render(usage());
    now.set(NOW + 10 * M);
    fixture.detectChanges();
    const meter = el.querySelector('[role="meter"]') as HTMLElement;
    expect(text(meter)).toContain('Resets in 2h');
    now.set(NOW + 3 * H);
    fixture.detectChanges();
    expect(meter.getAttribute('aria-valuenow')).toBe('0');
    expect(text(meter)).toContain('0%');
    expect(text(meter)).toContain('a new window starts with your next message');
  });

  it('says when the last check failed, and when the next one is due', () => {
    const rt = TestBed.inject(RealtimeService);
    const connection = (rt as unknown as { connectionS: { set(v: string): void } }).connectionS;
    const el = render(usage({ error: 'Usage endpoint is rate limited', nextRefreshAt: undefined }));
    expect(text(el)).toContain('Last check failed: Usage endpoint is rate limited');
    // No read scheduled and no live connection: the Mac doesn't read.
    expect(text(el)).toContain('automatic checks wait for a live connection');
    // Live: about to be scheduled (an older server: backing off, up to an hour)…
    connection.set('open');
    fixture.detectChanges();
    expect(text(el)).toContain('next try within an hour');
    // …or nothing active (an older server reads every 30 min).
    rt.mergePlanUsage(usage({ nextRefreshAt: undefined }));
    fixture.detectChanges();
    expect(text(el)).toContain('next check within 30 min, sooner when a session is active');
    expect(text(el)).toContain(
      'Checks every 5 min while a session is active, and every 30 min otherwise.',
    );
    rt.mergePlanUsage(usage({ intervalSecs: 0 }));
    fixture.detectChanges();
    expect(text(el)).toContain('automatic checks are off');
    expect(text(el)).toContain(
      'Automatic checks are off: it is read when you tap Refresh, and by a session started here',
    );
  });

  it('says when a running session last moved the session and weekly windows', () => {
    const el = render(usage({ source: 'live', fetchedAt: NOW - 12 * M, streamAt: NOW - 30_000 }));
    expect(text(el)).toContain(
      'Updated 30s ago by a running session (the rest 12m ago) · next check in 3m',
    );
    TestBed.inject(RealtimeService).mergePlanUsage(
      usage({ source: 'cache', fetchedAt: NOW - 12 * M, streamAt: NOW - 30_000 }),
    );
    fixture.detectChanges();
    expect(text(el)).toContain(
      "Updated 30s ago by a running session (Claude Code's last reading 12m ago)",
    );
  });

  it('shows extra usage without a currency when none is known', () => {
    const el = render(
      usage({ extraUsage: { enabled: true, usedMinor: 1_240, limitMinor: 5_000, decimals: 2 } }),
    );
    const plain = (minor: number) => formatMinor(minor, undefined, 2);
    expect(plain(1_240)).not.toContain('$');
    expect(text(el)).toContain(`Extra usage${plain(1_240)} of ${plain(5_000)}`);
  });

  it('shows extra usage in money', () => {
    const el = render(
      usage({
        extraUsage: {
          enabled: true,
          usedMinor: 1_240,
          limitMinor: 5_000,
          currency: 'USD',
          decimals: 2,
          utilization: 24.8,
        },
      }),
    );
    const money = (minor: number) => formatMinor(minor, 'USD', 2);
    expect(text(el)).toContain(`Extra usage${money(1_240)} of ${money(5_000)} (25%)`);
  });

  it('counts down the Refresh button while the server would refuse', async () => {
    backend.routes.set('/usage/refresh', () => ({
      status: 429,
      body: { error: { code: 'rate_limited', message: 'Too soon' } },
      headers: { 'retry-after': '42' },
    }));
    const el = render(usage());
    const button = () => el.querySelector('.status button') as HTMLButtonElement;
    expect(button().disabled).toBeFalse();
    button().click();
    await flush();
    fixture.detectChanges();
    expect(backend.calls).toContain('POST /usage/refresh');
    expect(button().disabled).toBeTrue();
    expect(text(button())).toContain('Refresh in 42s');
    now.set(NOW + 43_000);
    fixture.detectChanges();
    expect(button().disabled).toBeFalse();
  });

  it("never holds Refresh past a minute when this device's clock lags the Mac", () => {
    const el = render(usage({ source: 'live', fetchedAt: NOW + 5 * M }));
    const button = el.querySelector('.status button') as HTMLButtonElement;
    expect(text(button)).toContain('Refresh in 60s');
    now.set(NOW + 61_000);
    fixture.detectChanges();
    expect(button.disabled).toBeFalse();
  });

  it('counts the hold from when the numbers arrived, not from when they are shown', () => {
    // A live read arrives while no view shows it; this device lags the Mac.
    const rt = TestBed.inject(RealtimeService);
    rt.mergePlanUsage(usage({ source: 'live', fetchedAt: NOW + 5 * M }));
    expect(rt.planUsageReceivedAt()).toBe(NOW);
    now.set(NOW + 30_000);
    // A later copy of the same numbers doesn't restart the hold.
    rt.mergePlanUsage(usage({ source: 'live', fetchedAt: NOW + 5 * M, refreshing: false }));
    expect(rt.planUsageReceivedAt()).toBe(NOW);
    now.set(NOW + 3 * M);
    const el = render(null);
    const button = el.querySelector('.status button') as HTMLButtonElement;
    expect(button.disabled).toBeFalse();
    expect(text(button).trim()).toBe('Refresh');
    // New numbers: a new minute.
    rt.mergePlanUsage(usage({ source: 'live', fetchedAt: NOW + 8 * M }));
    fixture.detectChanges();
    expect(rt.planUsageReceivedAt()).toBe(NOW + 3 * M);
    expect(text(button)).toContain('Refresh in 60s');
  });

  it("takes a 429's wait from its body when the header is missing", async () => {
    backend.routes.set('/usage/refresh', () => ({
      status: 429,
      body: { error: { code: 'rate_limited', message: 'Too soon', retryAfter: 42 } },
    }));
    const el = render(usage());
    const button = () => el.querySelector('.status button') as HTMLButtonElement;
    button().click();
    await flush();
    fixture.detectChanges();
    expect(text(button())).toContain('Refresh in 42s');
  });

  it('keeps the next read time a newer frame brought when the interval changes', async () => {
    const rt = TestBed.inject(RealtimeService);
    backend.routes.set('/usage/settings', () => {
      // The server's frame with the rescheduled read overtakes its answer.
      (rt as unknown as { dispatch(raw: string): void }).dispatch(
        JSON.stringify({
          t: 'planUsage',
          usage: usage({ intervalSecs: 600, nextRefreshAt: NOW + 10 * M }),
        }),
      );
      return { status: 200, body: usage({ intervalSecs: 600, nextRefreshAt: NOW + 3 * M }) };
    });
    render(usage());
    expect(await TestBed.inject(PlanUsageService).setIntervalSecs(600)).toBeTrue();
    expect(rt.planUsage()?.intervalSecs).toBe(600);
    expect(rt.planUsage()?.nextRefreshAt).toBe(NOW + 10 * M);
    expect(backend.bodyOf('PUT /usage/settings')).toEqual({ intervalSecs: 600 });
  });

  it('holds Refresh for a minute after a read of the account', () => {
    const el = render(usage({ source: 'live', fetchedAt: NOW - 20_000 }));
    const button = el.querySelector('.status button') as HTMLButtonElement;
    expect(button.disabled).toBeTrue();
    expect(text(button)).toContain('Refresh in 40s');
  });
});
