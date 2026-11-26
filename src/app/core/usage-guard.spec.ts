import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { UsageGuardSettingComponent } from '../pages/settings/usage-guard-setting.component';
import { SessionCardComponent } from '../shared/session-card.component';
import { UsagePauseBannerComponent } from '../shared/usage-pause-banner.component';
import { UsagePausedLineComponent } from '../shared/usage-paused-line.component';
import { FakeBackend, flush, me, useFakeServer } from './mock/fake-server.testing';
import { ownedCaps } from './mock/fixtures';
import { setActiveMock } from './mock/mock-api';
import { SessionSummary } from './models';
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

const session = (over: Partial<SessionSummary>): SessionSummary => ({
  id: 's1',
  name: 'auth',
  projectName: 'api',
  cwd: '/Users/me/dev/api',
  surface: 'vscode',
  owned: false,
  live: true,
  status: 'idle',
  activity: { kind: 'idle', label: 'Idle' },
  usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
  messageCount: 3,
  agents: { running: 0, total: 0, workflowsRunning: 0, backgroundRunning: 0 },
  queued: 0,
  pending: [],
  control: { ...ownedCaps(), channel: 'hooks' },
  ...over,
});

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
      held: false,
      doing: ['the turn it was working on', 'Agent “tests”'],
      released: false,
    },
    {
      id: 's2',
      name: 'docs',
      owned: false,
      stopped: true,
      held: false,
      doing: [],
      released: true,
    },
    {
      id: 's3',
      name: 'deploy watch',
      owned: false,
      stopped: false,
      held: true,
      runAt: Date.now() + 10 * 60_000,
      run: 'Check the deploy',
      doing: [],
      released: false,
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
    backend.routes.set('/usage-guard/resume', (_m, body) => {
      const only = (body as { pause?: string } | undefined)?.pause;
      view = {
        ...view,
        pauses: view.pauses.map((p) =>
          !only || p.id === only ? { ...p, ended: 'resumedByHand', endedAt: Date.now() } : p,
        ),
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
      pauses: [
        {
          id: 'up_1',
          window: 'seven_day_opus',
          label: 'Weekly · Opus',
          sessions: [{}, { id: 's9', held: true, runAt: 5, run: 'Check' }],
        },
      ],
    });
    expect(v.settings).toEqual({ ...DEFAULT_GUARD_SETTINGS, thresholdPct: 85 });
    expect(v.pauses[0].sessions[0]).toEqual(
      expect.objectContaining({ stopped: false, held: false, released: false, doing: [] }),
    );
    expect(v.pauses[0].sessions[1]).toEqual(
      expect.objectContaining({ held: true, runAt: 5, run: 'Check' }),
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

  it('shows a pause above every page: its limit, when it resumes, and which sessions', async () => {
    view = { ...view, pauses: [pause()] };
    const fixture = TestBed.createComponent(UsagePauseBannerComponent);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    const el = fixture.nativeElement as HTMLElement;
    expect(words(el.querySelector('.ub-head'))).toBe('Personal reached 91 % of the 5-hour limit.');
    expect(words(el.querySelector('.ub-when'))).toMatch(/^Resumes at .+ \(in 1h 16m\)\.$/);
    // The sessions it holds, each a link to its page: the stopped one
    // first, then the one whose scheduled run waits; not the one someone
    // sent a message to.
    const chips = [...el.querySelectorAll<HTMLAnchorElement>('.ub-chip')];
    expect(chips.map((c) => words(c.querySelector('.nm')))).toEqual(['auth', 'deploy watch']);
    expect(chips.map((c) => c.getAttribute('href'))).toEqual(['/session/s1', '/session/s3']);
    expect(chips[0].getAttribute('title')).toBe(
      'Stopped: the turn it was working on, agent “tests”.',
    );
    expect(chips[1].classList.contains('held')).toBe(true);
    expect(chips[1].getAttribute('title')).toMatch(
      /^Its scheduled run “Check the deploy” \(.+\) waits\.$/,
    );
    const resume = [...el.querySelectorAll('button')].find((b) =>
      words(b).includes('Resume now'),
    ) as HTMLButtonElement;
    resume.click();
    await flush();
    fixture.detectChanges();
    expect(backend.calls).toContain('POST /usage-guard/resume');
    expect(words(el)).toBe('');
  });

  it('names five sessions at most, the rest in Settings', async () => {
    const many = Array.from({ length: 7 }, (_, i) => ({
      id: `m${i}`,
      name: `job ${i}`,
      owned: true,
      stopped: true,
      held: false,
      doing: [],
      released: false,
    }));
    view = { ...view, pauses: [pause({ sessions: many })] };
    const fixture = TestBed.createComponent(UsagePauseBannerComponent);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    const el = fixture.nativeElement as HTMLElement;
    expect(el.querySelectorAll('.ub-chip').length).toBe(5);
    const more = el.querySelector('.ub-more') as HTMLAnchorElement;
    expect(words(more)).toBe('+2 more');
    expect(more.getAttribute('href')).toBe('/settings#usage-limits');
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
    expect(el.querySelector('.ub-list')).toBeNull();
  });

  it('marks each paused session in the list, and says why on its page', async () => {
    view = { ...view, pauses: [pause()] };
    const release = TestBed.inject(UsageGuardService).watch();
    await flush();
    const card = (s: Partial<SessionSummary>) => {
      const fixture = TestBed.createComponent(SessionCardComponent);
      fixture.componentRef.setInput('session', session(s));
      fixture.detectChanges();
      return fixture.nativeElement as HTMLElement;
    };
    // Its work was stopped: ⏸ Paused until …, an amber edge, why in the title.
    const stopped = card({ id: 's1', name: 'auth' });
    expect(stopped.classList.contains('paused')).toBe(true);
    const chip = stopped.querySelector('.pchip') as HTMLElement;
    expect(words(chip.querySelector('.ptext'))).toMatch(/^Paused until \S+/);
    expect(chip.getAttribute('title')).toMatch(
      /^Mother Claude paused it at 91 % of the 5-hour limit\. It resumes at .+ \(in 1h 16m\)\.$/,
    );
    // Its scheduled run waits.
    const held = card({ id: 's3', name: 'deploy watch' });
    expect(held.querySelector('.pchip')?.getAttribute('title')).toMatch(
      /^Its scheduled run waits: Mother Claude paused this account at 91 % of the 5-hour limit\. The run waits until .+ \(in 1h 16m\)\.$/,
    );
    // Someone sent it a message, or not paused, or another computer's: none.
    expect(card({ id: 's2', name: 'docs' }).querySelector('.pchip')).toBeNull();
    expect(card({ id: 's9', name: 'other' }).querySelector('.pchip')).toBeNull();
    expect(card({ id: 's1', name: 'auth', machine: 'mac' }).querySelector('.pchip')).toBeNull();

    // On its page: why, and Resume now (its whole pause).
    const line = TestBed.createComponent(UsagePausedLineComponent);
    line.componentRef.setInput('session', 's1');
    line.detectChanges();
    const el = line.nativeElement as HTMLElement;
    expect(words(el)).toContain('Mother Claude paused it at 91 % of the 5-hour limit.');
    expect(words(el)).toContain('Stopped: the turn it was working on, agent “tests”.');
    (el.querySelector('button') as HTMLButtonElement).click();
    await flush();
    expect(backend.bodyOf('POST /usage-guard/resume')).toEqual({ pause: 'up_0123456789ab' });
    line.detectChanges();
    expect(words(el)).toBe('');
    release();
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
              held: false,
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
    // Each session a link, with how the pause holds it: stopped first, then
    // the held run, then the one someone sent a message to.
    const rows = [...el.querySelectorAll('.pause:not(.ended) .sessions li')].map((r) => words(r));
    expect(rows[0]).toBe('auth · Stopped: the turn it was working on, agent “tests”.');
    expect(rows[1]).toMatch(/^deploy watch · Its scheduled run “Check the deploy” \(.+\) waits\.$/);
    expect(rows[2]).toBe('docs · you sent it a message: left alone');
    expect(el.querySelector('.sessions a')?.getAttribute('href')).toBe('/session/s1');
    expect(words(el)).toContain('Resumed after the reset, at');
    expect(words(el)).toContain('api: It ended.');
  });
});
