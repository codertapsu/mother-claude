import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import {
  FakeBackend,
  flush,
  serveAccounts,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import { MOCK_ACCOUNT_WORK, buildAccounts, ownedCaps } from '../../core/mock/fixtures';
import { setActiveMock } from '../../core/mock/mock-api';
import { AccountRef, SessionSummary } from '../../core/models';
import { RealtimeService } from '../../core/realtime.service';
import { inAccount } from '../../shared/accounts';
import { SessionsComponent, sessionMatches } from './sessions.component';

const s = (over: Partial<SessionSummary>): SessionSummary =>
  ({
    id: 's1',
    name: 'auth',
    projectName: 'api',
    cwd: '/Users/me/dev/api',
    usage: { input: 0, output: 10_000, cacheRead: 0, cacheWrite: 0 },
    ...over,
  }) as SessionSummary;

describe('the account filter', () => {
  it('matches the account picked, and every session when none is', () => {
    const work = { account: { id: 'acc_w' } };
    expect(inAccount(work, '')).toBeTrue();
    expect(inAccount(work, 'acc_w')).toBeTrue();
    expect(inAccount(work, 'default')).toBeFalse();
    // A session in a directory nobody registered names no account.
    expect(inAccount({}, 'default')).toBeFalse();
    expect(inAccount(undefined, '')).toBeTrue();
  });
});

describe('session search', () => {
  it('finds what the card chips say', () => {
    const xhigh = s({ effort: 'xhigh', effortSource: 'observed' });
    // The chip reads "X-high"; the Info tab "Extra high".
    expect(sessionMatches(xhigh, 'x-high')).toBeTrue();
    expect(sessionMatches(xhigh, 'extra high')).toBeTrue();
    expect(sessionMatches(xhigh, 'xhigh')).toBeTrue();
    expect(sessionMatches(s({ effort: 'medium' }), 'med')).toBeTrue();

    const off = s({ thinking: { mode: 'off', source: 'launch' } });
    expect(sessionMatches(off, 'thinking off')).toBeTrue();
    expect(sessionMatches(s({}), 'thinking off')).toBeFalse();
    expect(sessionMatches(s({ ultracode: 'on' }), 'ultracode')).toBeTrue();
    expect(sessionMatches(s({ ultracode: 'off' }), 'ultracode')).toBeFalse();
  });
});

describe('SessionsComponent accounts', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<SessionsComponent>;

  const DEFAULT_REF: AccountRef = {
    id: 'default',
    name: 'Default',
    color: '#8b5cf6',
    kind: 'directory',
  };
  const WORK_REF: AccountRef = {
    id: MOCK_ACCOUNT_WORK,
    name: 'Work',
    color: '#0ea5e9',
    kind: 'directory',
  };
  const card = (id: string, account?: AccountRef, over: Partial<SessionSummary> = {}) =>
    ({
      id,
      name: id,
      title: `Session ${id}`,
      projectName: 'api',
      cwd: '/Users/me/dev/api',
      surface: 'terminal',
      owned: false,
      live: true,
      status: 'working',
      activity: { kind: 'thinking', label: 'Thinking' },
      usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
      messageCount: 1,
      agents: { running: 0, total: 0, workflowsRunning: 0, backgroundRunning: 0 },
      queued: 0,
      pending: [],
      control: { ...ownedCaps(), channel: 'none' },
      lastActivityAt: Date.now(),
      ...(account ? { account } : {}),
      ...over,
    }) as SessionSummary;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
  });

  afterEach(() => setActiveMock(null));

  const store = (list: SessionSummary[]) =>
    (
      TestBed.inject(RealtimeService) as unknown as { replaceStore(l: SessionSummary[]): void }
    ).replaceStore(list);
  const render = async () => {
    fixture = TestBed.createComponent(SessionsComponent);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    return fixture.nativeElement as HTMLElement;
  };
  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const names = (el: HTMLElement) =>
    [...el.querySelectorAll('mc-session-card .name')].map((n) => text(n)).sort();

  it('with only the Default shows no account filter and no per-account counts', async () => {
    serveAccounts(backend, buildAccounts(Date.now(), false));
    store([card('a'), card('b')]);
    const el = await render();
    expect(el.querySelector('select[aria-label="Account"]')).toBeNull();
    expect(el.querySelector('.acctsum')).toBeNull();
    expect(names(el)).toEqual(['Session a', 'Session b']);
  });

  it("filters by account, from the select or an account's counts", async () => {
    serveAccounts(backend, buildAccounts(Date.now(), true));
    store([
      card('a', DEFAULT_REF, { costToday: 1.5 }),
      card('b', WORK_REF, {
        status: 'waiting',
        pending: [
          {
            id: 'p',
            kind: 'question',
            title: 'Q?',
            dangerous: false,
            answerable: true,
            createdAt: 1,
          },
        ],
      }),
      card('c', WORK_REF, { live: false, status: 'ended' }),
    ]);
    const el = await render();
    const pills = [...el.querySelectorAll<HTMLButtonElement>('.acctsum')];
    expect(pills.map((p) => p.getAttribute('aria-label'))).toEqual([
      'Default: 1 live, 1 working, $1.50 today. Show its sessions',
      'Work: 1 live, 1 needs you. Show its sessions',
    ]);
    pills[1].click();
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    expect(names(el)).toEqual(['Session b', 'Session c']);
    expect((el.querySelector('select[aria-label="Account"]') as HTMLSelectElement).value).toBe(
      MOCK_ACCOUNT_WORK,
    );
    expect(pills[1].getAttribute('aria-pressed')).toBe('true');
    // Tapping it again shows every account.
    pills[1].click();
    fixture.detectChanges();
    expect(names(el)).toEqual(['Session a', 'Session b', 'Session c']);
  });
});
