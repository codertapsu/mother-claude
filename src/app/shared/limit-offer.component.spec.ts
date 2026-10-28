import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { FakeBackend, flush, serveAccounts, useFakeServer } from '../core/mock/fake-server.testing';
import { MOCK_ACCOUNT_PRO, buildAccounts, ownedCaps } from '../core/mock/fixtures';
import { setActiveMock } from '../core/mock/mock-api';
import { AccountRef, PlanUsage, SessionSummary } from '../core/models';
import { RealtimeService } from '../core/realtime.service';
import { LimitOfferComponent } from './limit-offer.component';

const H = 3_600_000;
const DEFAULT_REF: AccountRef = {
  id: 'default',
  name: 'Default',
  color: '#8b5cf6',
  kind: 'directory',
};
const PRO_REF: AccountRef = {
  id: MOCK_ACCOUNT_PRO,
  name: 'Marcus (Pro)',
  color: '#f59e0b',
  kind: 'token',
};

const usage = (pct: number): PlanUsage => ({
  available: true,
  windows: [
    { key: 'five_hour', label: 'Current session', pct, resetsAt: Date.now() + 2 * H + 60_000 },
    { key: 'seven_day', label: 'Weekly · all models', pct: 20 },
  ],
  refreshing: false,
  intervalSecs: 300,
});

const session = (id: string, account?: AccountRef): SessionSummary =>
  ({
    id,
    name: id,
    projectName: 'api',
    cwd: '/Users/me/dev/api',
    surface: 'mother-claude',
    owned: true,
    live: true,
    status: 'idle',
    activity: { kind: 'idle', label: 'Idle' },
    usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
    messageCount: 1,
    agents: { running: 0, total: 0, workflowsRunning: 0, backgroundRunning: 0 },
    queued: 0,
    pending: [],
    control: ownedCaps(),
    ...(account ? { account } : {}),
  }) as SessionSummary;

describe('LimitOfferComponent', () => {
  let backend: FakeBackend;
  let rt: RealtimeService;
  let fixture: ComponentFixture<LimitOfferComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    rt = TestBed.inject(RealtimeService);
  });

  afterEach(() => setActiveMock(null));

  const store = (list: SessionSummary[]) =>
    (rt as unknown as { replaceStore(l: SessionSummary[]): void }).replaceStore(list);
  const render = (s?: SessionSummary) => {
    fixture = TestBed.createComponent(LimitOfferComponent);
    if (s) fixture.componentRef.setInput('session', s);
    fixture.detectChanges();
    return fixture.nativeElement as HTMLElement;
  };
  const text = (el: Element) => (el.textContent ?? '').replace(/\s+/g, ' ').trim();

  it('stays away with only the Default account, even at its limit', () => {
    serveAccounts(backend, buildAccounts(Date.now(), false));
    rt.mergePlanUsage(usage(100));
    store([session('s1')]);
    const el = render();
    expect(el.querySelector('.offer')).toBeNull();
    expect(el.classList).not.toContain('has-offers');
  });

  it("offers to switch a limited account's conversations to a free one", async () => {
    serveAccounts(backend, buildAccounts(Date.now(), true));
    rt.mergePlanUsage(usage(100));
    rt.mergeAccountUsages({ accounts: { [MOCK_ACCOUNT_PRO]: usage(12) } });
    store([session('s1', DEFAULT_REF), session('s2', PRO_REF)]);
    const el = render();
    const offers = el.querySelectorAll('.offer');
    expect(offers.length).toBe(1);
    expect(text(offers[0])).toContain(
      'Account Default has reached its 5-hour limit — switch to Marcus (Pro)?',
    );
    expect(text(offers[0])).toMatch(/\(resets in 2h \dm\)/);
    expect(el.classList).toContain('has-offers');
    (offers[0].querySelector('.btn.primary') as HTMLButtonElement).click();
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    // Switch account opens with that account's conversation in it.
    const dialog = el.querySelector('dialog') as HTMLDialogElement;
    expect(dialog.open).toBeTrue();
    expect(text(dialog)).toContain('Switch to Marcus (Pro)');
  });

  it('offers to continue a session another app runs, which cannot switch', () => {
    serveAccounts(backend, buildAccounts(Date.now(), true));
    rt.mergePlanUsage(usage(100));
    const vscode = {
      ...session('s3', DEFAULT_REF),
      owned: false,
      surface: 'vscode' as const,
      control: { ...ownedCaps(), fork: true },
    };
    const el = render(vscode);
    expect(text(el.querySelector('.offer') as Element)).toContain(
      'has reached its 5-hour limit — continue on Marcus (Pro)?',
    );
    expect(text(el.querySelector('.offer .btn.primary') as Element)).toBe(
      'Continue on Marcus (Pro)',
    );
  });

  it("on a session page speaks only for that session's account", () => {
    serveAccounts(backend, buildAccounts(Date.now(), true));
    rt.mergePlanUsage(usage(100));
    expect(render(session('s2', PRO_REF)).querySelector('.offer')).toBeNull();
    expect(render(session('s1', DEFAULT_REF)).querySelector('.offer')).not.toBeNull();
  });

  it('"Not now" hides it until the window starts over', () => {
    serveAccounts(backend, buildAccounts(Date.now(), true));
    rt.mergePlanUsage(usage(100));
    const el = render(session('s1', DEFAULT_REF));
    (el.querySelector('[aria-label="Not now"]') as HTMLButtonElement).click();
    fixture.detectChanges();
    expect(el.querySelector('.offer')).toBeNull();
  });
});
