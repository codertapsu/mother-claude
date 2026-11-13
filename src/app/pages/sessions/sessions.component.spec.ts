import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { AuthService } from '../../core/auth.service';
import {
  FakeBackend,
  flush,
  me,
  serveAccounts,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import { MOCK_ACCOUNT_WORK, buildAccounts, ownedCaps } from '../../core/mock/fixtures';
import { setActiveMock } from '../../core/mock/mock-api';
import { AccountRef, SessionSummary } from '../../core/models';
import { storageRemove } from '../../core/platform';
import { RealtimeService } from '../../core/realtime.service';
import { inAccount } from '../../shared/accounts';
import { sessionMatches } from '../../shared/session-search';
import { SessionsComponent } from './sessions.component';

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
    expect(inAccount(work, '')).toBe(true);
    expect(inAccount(work, 'acc_w')).toBe(true);
    expect(inAccount(work, 'default')).toBe(false);
    // A session in a directory nobody registered names no account.
    expect(inAccount({}, 'default')).toBe(false);
    expect(inAccount(undefined, '')).toBe(true);
  });
});

describe('session search', () => {
  it('finds what the card chips say', () => {
    const xhigh = s({ effort: 'xhigh', effortSource: 'observed' });
    // The chip reads "X-high"; the Info tab "Extra high".
    expect(sessionMatches(xhigh, 'x-high')).toBe(true);
    expect(sessionMatches(xhigh, 'extra high')).toBe(true);
    expect(sessionMatches(xhigh, 'xhigh')).toBe(true);
    expect(sessionMatches(s({ effort: 'medium' }), 'med')).toBe(true);

    const off = s({ thinking: { mode: 'off', source: 'launch' } });
    expect(sessionMatches(off, 'thinking off')).toBe(true);
    expect(sessionMatches(s({}), 'thinking off')).toBe(false);
    expect(sessionMatches(s({ ultracode: 'on' }), 'ultracode')).toBe(true);
    expect(sessionMatches(s({ ultracode: 'off' }), 'ultracode')).toBe(false);
    // The model's id too, as the server matches it.
    const opus = s({ model: { id: 'claude-opus-4-8', label: 'Opus 4.8' } });
    expect(sessionMatches(opus, 'claude-opus')).toBe(true);
    expect(sessionMatches(opus, 'opus 4.8')).toBe(true);
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

  afterEach(() => {
    setActiveMock(null);
    storageRemove('mc_sessions_filter');
  });

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

  it('groups sessions that resume by themselves apart from idle ones', async () => {
    serveAccounts(backend, buildAccounts(Date.now(), false));
    store([
      card('a'),
      card('b', undefined, {
        surface: 'vscode',
        status: 'background',
        activity: { kind: 'background', label: 'Background work', detail: 'Release build' },
      }),
      card('c', undefined, {
        status: 'background',
        activity: {
          kind: 'scheduled',
          label: 'Scheduled to resume',
          detail: 'Check the rollout',
          resumesAt: Date.now() + 12 * 60_000,
        },
      }),
      card('d', undefined, { status: 'idle', activity: { kind: 'idle', label: 'Idle' } }),
    ]);
    const el = await render();
    // Sorted: two of them share a group in whichever order.
    const groups = [...el.querySelectorAll('section.group')].map((g) => ({
      title: g.getAttribute('aria-label'),
      cards: [...g.querySelectorAll('mc-session-card .name')].map((n) => text(n)).sort(),
      pills: [...g.querySelectorAll('mc-status-pill')]
        .map((p) => text(p).replace(', live', ''))
        .sort(),
    }));
    expect(groups).toEqual([
      { title: 'Working', cards: ['Session a'], pills: ['Working'] },
      {
        title: 'Will resume',
        cards: ['Session b', 'Session c'],
        pills: ['Background work', 'Scheduled'],
      },
      { title: 'Idle', cards: ['Session d'], pills: ['Idle'] },
    ]);
    // What it waits on, and when.
    const scheduled = [...el.querySelectorAll('mc-session-card')].find((c) =>
      text(c).includes('Session c'),
    );
    expect(text(scheduled?.querySelector('mc-activity-line'))).toMatch(
      // The countdown rounds up: a clock a millisecond behind reads 13m.
      /^Scheduled to resume\s*·\s*in 1[123]m\s*·\s*Check the rollout$/,
    );
    // The fleet counts them apart.
    expect(text(el.querySelector('mc-fleet-summary'))).toContain('2 will resume');
  });

  it('with only the Default shows no account filter and no per-account counts', async () => {
    serveAccounts(backend, buildAccounts(Date.now(), false));
    store([card('a'), card('b')]);
    const el = await render();
    expect(el.querySelector('select[aria-label="Account"]')).toBeNull();
    expect(el.querySelector('.acctsum')).toBeNull();
    expect(names(el)).toEqual(['Session a', 'Session b']);
    // Select is always there; with nowhere to move them, only Delete.
    const select = [...el.querySelectorAll<HTMLButtonElement>('button')].find(
      (b) => text(b) === 'Select',
    );
    select?.click();
    fixture.detectChanges();
    const bar = text(el.querySelector('.selbar'));
    expect(bar).toContain('Delete');
    expect(bar).not.toContain('Move or copy');
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

  it('selects cards and opens Move data with them, each from where it is', async () => {
    serveAccounts(backend, buildAccounts(Date.now(), true));
    backend.routes.set('/backup', () => ({ status: 200, body: { settings: { folder: null } } }));
    store([card('a', DEFAULT_REF), card('b', WORK_REF), card('c', WORK_REF)]);
    const el = await render();
    const button = (label: string) =>
      [...el.querySelectorAll<HTMLButtonElement>('button')].find((b) => text(b).startsWith(label));
    expect(el.querySelector('mc-session-card .pick')).toBeNull();
    button('Select')?.click();
    fixture.detectChanges();
    const boxes = () => [...el.querySelectorAll<HTMLInputElement>('mc-session-card .pick input')];
    expect(boxes().length).toBe(3);
    boxes()[0].click();
    boxes()[2].click();
    fixture.detectChanges();
    expect(text(el.querySelector('.selcount'))).toBe('2 selected');
    expect(el.querySelectorAll('mc-session-card[data-selected]').length).toBe(2);
    backend.routes.set('/sessions', () => ({ status: 200, body: { ids: ['a', 'b', 'c'] } }));
    button('Select all')?.click();
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    expect(text(el.querySelector('.selcount'))).toBe('3 selected');
    expect(backend.urls).toContain('/sessions?scope=all&view=ids');

    button('Move or copy 3')?.click();
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    const sheet = el.querySelector('mc-move-data-sheet') as HTMLElement;
    const from = sheet.querySelector<HTMLSelectElement>('select[name="from"]');
    expect(text(from?.selectedOptions[0])).toBe('Where each one is');
    expect(text(sheet.querySelector('mc-move-checklist'))).toContain('3 of 3 selected');

    sheet.querySelector<HTMLButtonElement>('button[aria-label="Close"]')?.click();
    fixture.detectChanges();

    // Delete: one request per account the conversations are in.
    backend.routes.set('/data/delete', () => ({
      status: 200,
      body: { plan: { sessions: [{ id: 'a' }], skipped: [] } },
    }));
    TestBed.inject(AuthService).me.set(me({ principal: 'desktop', allowDangerous: true }));
    button('Delete 3')?.click();
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    const asked = backend.calls
      .map((c, i) => (c === 'POST /data/delete' ? backend.bodies[i] : null))
      .filter(Boolean);
    // (In whichever order the cards sorted: they share a time.)
    const byAccount = (bodies: unknown[]) =>
      (bodies as { from: string; items: { sessions: string[] } }[])
        .map((b) => ({ ...b, items: { ...b.items, sessions: [...b.items.sessions].sort() } }))
        .sort((x, y) => x.from.localeCompare(y.from));
    expect(byAccount(asked)).toEqual(
      byAccount([
        { from: 'default', items: { sessions: ['a'] }, undo: true, dryRun: true },
        { from: MOCK_ACCOUNT_WORK, items: { sessions: ['b', 'c'] }, undo: true, dryRun: true },
      ]),
    );
    expect(text(el.querySelector('mc-delete-data-sheet .planline'))).toContain('Deletes');

    // Done leaves Select, and forgets the picks.
    button('Done')?.click();
    fixture.detectChanges();
    expect(el.querySelector('.selbar')).toBeNull();
    button('Select')?.click();
    fixture.detectChanges();
    expect(text(el.querySelector('.selcount'))).toBe('0 selected');
  });

  it('selects every session the search matches, earlier ones not read yet too', async () => {
    serveAccounts(backend, buildAccounts(Date.now(), false));
    backend.routes.set('/backup', () => ({ status: 200, body: { settings: { folder: null } } }));
    store([
      card('a', undefined, { title: 'Xiangqi engine' }),
      card('b', undefined, { title: 'Chess clock' }),
    ]);
    const el = await render();
    const button = (label: string) =>
      [...el.querySelectorAll<HTMLButtonElement>('button')].find((b) => text(b).startsWith(label));
    const search = el.querySelector<HTMLInputElement>('input[aria-label="Search sessions"]')!;
    search.value = 'Xiangqi';
    search.dispatchEvent(new Event('input'));
    fixture.detectChanges();
    expect(names(el)).toEqual(['Xiangqi engine']);

    // The server finds the earlier ones; the card shown is taken either way.
    backend.routes.set('/sessions', () => ({
      status: 200,
      body: { ids: ['a', 'old-1', 'old-2'] },
    }));
    button('Select')?.click();
    fixture.detectChanges();
    button('Select all matching')?.click();
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    expect(text(el.querySelector('.selcount'))).toBe('3 selected');
    expect(backend.urls).toContain('/sessions?scope=all&q=Xiangqi&view=ids');

    // Without the server, the cards shown are selected, and the bar says so.
    button('Clear')?.click();
    backend.routes.set('/sessions', () => ({ status: 500, body: {} }));
    button('Select all matching')?.click();
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    expect(text(el.querySelector('.selcount'))).toBe('1 selected');
    expect(text(el.querySelector('.selerr'))).toContain('Only the sessions shown were selected');
  });

  it('searches earlier sessions on the server as soon as a search is typed', async () => {
    serveAccounts(backend, buildAccounts(Date.now(), false));
    store([card('a', undefined, { title: 'Chess clock' })]);
    backend.routes.set('/sessions', () => ({
      status: 200,
      body: { sessions: [card('old', undefined, { title: 'Xiangqi openings', status: 'ended' })] },
    }));
    const el = await render();
    expect(el.querySelector('.history h2')).toBeNull();
    const search = el.querySelector<HTMLInputElement>('input[aria-label="Search sessions"]')!;
    search.value = 'xiangqi';
    search.dispatchEvent(new Event('input'));
    fixture.detectChanges();
    // Once the typing stops.
    await new Promise((r) => setTimeout(r, 350));
    await flush();
    fixture.detectChanges();
    expect(backend.urls).toContain('/sessions?scope=all&limit=25&q=xiangqi');
    expect(names(el)).toEqual(['Xiangqi openings']);
    // No current one matches, and the page says where the match is.
    expect(text(el.querySelector('.empty-state strong'))).toBe('No current sessions match');
  });

  it('heads the list with Select and New session, and the tabs of Sessions', async () => {
    serveAccounts(backend, buildAccounts(Date.now(), false));
    store([card('a')]);
    const el = await render();
    const head = el.querySelector('mc-sessions-head')!;
    expect(text(head.querySelector('h1'))).toBe('Sessions');
    expect([...head.querySelectorAll('.page-head .btn')].map((b) => text(b))).toEqual([
      'Select',
      'New session',
    ]);
    expect(head.querySelector('a.btn.primary')?.getAttribute('href')).toBe('/new');
    expect([...head.querySelectorAll('nav.hub-tabs a')].map((a) => text(a))).toEqual([
      'Sessions',
      'Agents',
      'Projects',
    ]);
    // Projects is a tab of its own (`/projects`): no switch on the list.
    expect(el.querySelector('mc-projects-view')).toBeNull();
    expect(el.querySelector('[aria-label="Show"]')).toBeNull();
    // The filter of the app a session runs in.
    const apps = el.querySelector<HTMLSelectElement>('select[aria-label="App"]')!;
    expect(text(apps.options[0])).toBe('All apps');
  });
});
