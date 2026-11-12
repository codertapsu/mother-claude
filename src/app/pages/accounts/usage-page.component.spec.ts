import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import {
  FakeBackend,
  asDesktop,
  asPhoneOf,
  flush,
  serveAccounts,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import { MOCK_ACCOUNT_WORK, buildAccounts } from '../../core/mock/fixtures';
import { setActiveMock } from '../../core/mock/mock-api';
import { PlanUsage } from '../../core/models';
import { RealtimeService } from '../../core/realtime.service';
import { UsagePageComponent } from './usage-page.component';

const NOW = Date.now();
const H = 3_600_000;

const usage = (pct: number, over: Partial<PlanUsage> = {}): PlanUsage => ({
  available: true,
  windows: [
    { key: 'five_hour', label: 'Current session', pct, resetsAt: NOW + 2 * H },
    { key: 'seven_day', label: 'Weekly · all models', pct: 18, resetsAt: NOW + 70 * H },
  ],
  fetchedAt: NOW - 120_000,
  source: 'cache',
  refreshing: false,
  nextRefreshAt: NOW + 180_000,
  intervalSecs: 300,
  ...over,
});

/** Accounts › Usage (docs/UI-REDESIGN.md §6.6): every account's windows,
 * how often they are read, the rate limits the teams hit. */
describe('UsagePageComponent: Accounts › Usage', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<UsagePageComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    backend.routes.set('/usage', () => ({ status: 200, body: usage(42) }));
  });

  afterEach(() => {
    fixture?.destroy();
    setActiveMock(null);
  });

  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const settle = async () => {
    for (let i = 0; i < 4; i++) {
      await flush();
      fixture.detectChanges();
    }
  };
  const render = async () => {
    fixture = TestBed.createComponent(UsagePageComponent);
    fixture.detectChanges();
    await settle();
    return fixture.nativeElement as HTMLElement;
  };
  const sessions = (list: unknown[]) =>
    (
      TestBed.inject(RealtimeService) as unknown as { replaceStore(l: unknown[]): void }
    ).replaceStore(list);

  it("lists every account's plan, its sessions running, and the rate limits teams hit", async () => {
    asDesktop();
    serveAccounts(backend, buildAccounts(NOW, true));
    const rt = TestBed.inject(RealtimeService);
    rt.mergePlanUsage(usage(42));
    rt.mergeAccountUsages({ accounts: { [MOCK_ACCOUNT_WORK]: usage(77) } });
    sessions([
      { id: 's1', name: 's1', live: true, account: { id: MOCK_ACCOUNT_WORK, name: 'Work' } },
      { id: 's2', name: 's2', live: true, account: { id: MOCK_ACCOUNT_WORK, name: 'Work' } },
      { id: 's3', name: 's3', live: false },
    ]);
    backend.routes.set('/capacity', () => ({
      status: 200,
      body: {
        caps: {},
        settings: {},
        host: {},
        rateLimits: [
          {
            at: NOW - 60_000,
            account: MOCK_ACCOUNT_WORK,
            status: 'rejected',
            limit: 'five_hour',
            source: 'stream',
          },
        ],
      },
    }));
    const el = await render();
    const cards = [...el.querySelectorAll('section.acct')];
    expect(cards.map((c) => c.getAttribute('aria-label'))).toEqual([
      'Plan usage of Default',
      'Plan usage of Work',
      'Plan usage of Marcus (Pro)',
    ]);
    expect(text(cards[1])).toContain('77%');
    expect(text(cards[1].querySelector('.running'))).toBe('2 sessions running');
    expect(text(cards[0].querySelector('.running'))).toBe('No session running');
    const limits = el.querySelector('section[aria-label="Rate limits teams hit"]');
    expect(text(limits)).toContain('Claude refused a request on the 5-hour limit · Work');
    expect(el.querySelector('a[href="/capacity"]')).not.toBeNull();
  });

  it('sets how often usage is read on the desktop', async () => {
    asDesktop();
    serveAccounts(backend, buildAccounts(NOW, false));
    TestBed.inject(RealtimeService).mergePlanUsage(usage(42));
    backend.routes.set('/usage/settings', (_m, body) => ({
      status: 200,
      body: usage(42, { intervalSecs: (body as { intervalSecs: number }).intervalSecs }),
    }));
    const el = await render();
    const select = el.querySelector<HTMLSelectElement>('#usage-interval')!;
    expect(select.value).toBe('300');
    select.value = '600';
    select.dispatchEvent(new Event('change'));
    await settle();
    expect(backend.bodyOf('PUT /usage/settings')).toEqual({ intervalSecs: 600 });
    expect(text(el.querySelector('.why'))).toContain('While nothing is active');
  });

  it('shows the interval on a phone, changed on the desktop', async () => {
    asPhoneOf('macos');
    serveAccounts(backend, buildAccounts(NOW, false));
    TestBed.inject(RealtimeService).mergePlanUsage(usage(42));
    const el = await render();
    expect(el.querySelector('#usage-interval')).toBeNull();
    const value = el.querySelector('mc-desktop-only');
    expect(text(value?.querySelector('.value'))).toBe('Every 5 min');
    expect(text(value?.querySelector('.line'))).toBe('Changed on the desktop app.');
  });
});
