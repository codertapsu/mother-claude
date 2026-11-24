import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { UsageGuardSettingComponent } from '../pages/settings/usage-guard-setting.component';
import { UsagePauseBannerComponent } from '../shared/usage-pause-banner.component';
import { FakeBackend, flush, me, useFakeServer } from './mock/fake-server.testing';
import { setActiveMock } from './mock/mock-api';
import { RealtimeService } from './realtime.service';
import {
  DEFAULT_GUARD_SETTINGS,
  UsageGuardView,
  UsagePause,
  normalizeUsageGuard,
  windowText,
} from './usage-guard';
import { UsageGuardService } from './usage-guard.service';

const words = (el: Element | null | undefined) =>
  (el?.textContent ?? '').replace(/\s+/g, ' ').trim();

const pause = (over: Partial<UsagePause> = {}): UsagePause => ({
  id: 'up_0123456789ab',
  account: 'default',
  accountName: 'Personal',
  window: 'five_hour',
  label: 'Current session',
  pct: 91.4,
  thresholdPct: 90,
  pausedAt: Date.now() - 60_000,
  resetsAt: Date.now() + 75 * 60_000,
  resumeAt: Date.now() + 76 * 60_000,
  sessions: [
    {
      id: 's1',
      name: 'auth',
      owned: true,
      stopped: true,
      doing: ['the turn it was working on'],
      released: false,
    },
    {
      id: 's2',
      name: 'docs',
      owned: false,
      stopped: true,
      doing: [],
      released: true,
    },
  ],
  hold: false,
  ...over,
});

describe('usage guard', () => {
  let backend: FakeBackend;
  let view: UsageGuardView;

  beforeEach(() => {
    backend = new FakeBackend();
    view = { settings: { ...DEFAULT_GUARD_SETTINGS }, pauses: [] };
    backend.routes.set('/usage-guard', () => ({ status: 200, body: view }));
    backend.routes.set('/usage-guard/settings', (_m, body) => {
      view = { ...view, settings: { ...view.settings, ...(body as object) } };
      return { status: 200, body: view };
    });
    backend.routes.set('/usage-guard/resume', () => {
      view = {
        ...view,
        pauses: view.pauses.map((p) => ({ ...p, ended: 'resumedByHand', endedAt: Date.now() })),
      };
      return { status: 200, body: view };
    });
    backend.routes.set('/usage-guard/pauses/up_0123456789ab/hold', (_m, body) => {
      view = {
        ...view,
        pauses: view.pauses.map((p) => ({ ...p, hold: (body as { hold: boolean }).hold })),
      };
      return { status: 200, body: view };
    });
    useFakeServer(backend, [provideRouter([])]);
  });

  afterEach(() => {
    TestBed.inject(RealtimeService).stop();
    setActiveMock(null);
  });

  it('reads the answer tolerantly and names the windows', () => {
    const v = normalizeUsageGuard({
      settings: { thresholdPct: 85, extra: 1 },
      pauses: [{ id: 'up_1', window: 'seven_day_opus', label: 'Weekly · Opus', sessions: [{}] }],
    });
    expect(v.settings).toEqual({ ...DEFAULT_GUARD_SETTINGS, thresholdPct: 85 });
    expect(v.pauses[0].sessions[0]).toEqual(
      expect.objectContaining({ stopped: false, released: false, doing: [] }),
    );
    expect(normalizeUsageGuard(null).pauses).toEqual([]);
    expect(windowText('five_hour', '')).toBe('the 5-hour limit');
    expect(windowText('seven_day', '')).toBe('the weekly limit');
    expect(windowText('seven_day_opus', 'Weekly · Opus')).toBe('the weekly Opus limit');
  });

  it('loads while watched, refetches on a `usageGuard` frame, and applies its answers', async () => {
    const svc = TestBed.inject(UsageGuardService);
    const release = svc.watch();
    await flush();
    expect(svc.view()?.settings.thresholdPct).toBe(90);
    expect(svc.active()).toEqual([]);

    view = { ...view, pauses: [pause()] };
    const rt = TestBed.inject(RealtimeService);
    await rt.start();
    backend.last.open();
    backend.last.frame({ t: 'hello', v: 2, me: me(), serverTime: Date.now() });
    await flush();
    backend.last.frame({ t: 'usageGuard' });
    await flush();
    expect(svc.active().map((p) => p.id)).toEqual(['up_0123456789ab']);

    await svc.setSettings({ thresholdPct: 80 });
    expect(backend.bodyOf('PUT /usage-guard/settings')).toEqual({ thresholdPct: 80 });
    expect(svc.view()?.settings.thresholdPct).toBe(80);
    await svc.hold('up_0123456789ab', true);
    expect(backend.bodyOf('POST /usage-guard/pauses/up_0123456789ab/hold')).toEqual({
      hold: true,
    });
    await svc.resume();
    expect(backend.bodyOf('POST /usage-guard/resume')).toEqual({});
    expect(svc.active()).toEqual([]);
    expect(svc.ended().length).toBe(1);
    release();
  });

  it('shows a pause above every page, counting down to its resume', async () => {
    view = { ...view, pauses: [pause()] };
    const fixture = TestBed.createComponent(UsagePauseBannerComponent);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    const el = fixture.nativeElement as HTMLElement;
    expect(words(el)).toContain('Personal reached 91 % of the 5-hour limit.');
    expect(words(el)).toContain('Mother Claude paused 2 sessions and resumes them at');
    expect(words(el)).toContain('(in 1h 16m)');
    const resume = [...el.querySelectorAll('button')].find((b) =>
      words(b).includes('Resume now'),
    ) as HTMLButtonElement;
    resume.click();
    await flush();
    fixture.detectChanges();
    expect(backend.calls).toContain('POST /usage-guard/resume');
    expect(words(el)).toBe('');
  });

  it('says a held pause waits for you', async () => {
    view = { ...view, pauses: [pause({ hold: true, sessions: [] })] };
    const fixture = TestBed.createComponent(UsagePauseBannerComponent);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    const el = fixture.nativeElement as HTMLElement;
    expect(words(el)).toContain('New work waits until you resume it.');
    expect(words(el)).toContain('Resume by itself');
  });

  it('sets the threshold and the switches in Settings, and lists the pauses', async () => {
    view = {
      ...view,
      pauses: [
        pause(),
        pause({
          id: 'up_ffffffffffff',
          ended: 'resumed',
          endedAt: Date.now(),
          sessions: [
            {
              id: 's3',
              name: 'api',
              owned: true,
              stopped: true,
              doing: [],
              released: false,
              resumeError: 'It ended.',
            },
          ],
        }),
      ],
    };
    const fixture = TestBed.createComponent(UsageGuardSettingComponent);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    const el = fixture.nativeElement as HTMLElement;
    const input = el.querySelector('#usage-threshold') as HTMLInputElement;
    expect(input.value).toBe('90');
    input.value = '85';
    input.dispatchEvent(new Event('change'));
    await flush();
    expect(backend.bodyOf('PUT /usage-guard/settings')).toEqual({ thresholdPct: 85 });
    // Out of range: put back, nothing sent.
    const calls = backend.calls.length;
    input.value = '120';
    input.dispatchEvent(new Event('change'));
    await flush();
    expect(backend.calls.length).toBe(calls);
    fixture.detectChanges();
    expect(input.value).toBe('85');

    const switches = [...el.querySelectorAll<HTMLInputElement>('input[type="checkbox"]')];
    expect(switches.length).toBe(3);
    switches[1].click();
    await flush();
    expect(backend.bodyOf('PUT /usage-guard/settings')).toEqual({ autoResume: false });

    fixture.detectChanges();
    expect(words(el)).toContain('Personal: 91 % of the 5-hour limit');
    expect(words(el)).toContain('docs · you sent it a message: left alone');
    expect(words(el)).toContain('Resumed after the reset, at');
    expect(words(el)).toContain('api: It ended.');
  });
});
