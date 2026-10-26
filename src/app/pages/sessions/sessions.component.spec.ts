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
import { storageGet, storageRemove } from '../../core/platform';
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
    // The model's id too, as the server matches it.
    const opus = s({ model: { id: 'claude-opus-4-8', label: 'Opus 4.8' } });
    expect(sessionMatches(opus, 'claude-opus')).toBeTrue();
    expect(sessionMatches(opus, 'opus 4.8')).toBeTrue();
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
    storageRemove('mc_sessions_view');
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
    expect(asked).toEqual([
      { from: 'default', items: { sessions: ['a'] }, undo: true, dryRun: true },
      {
        from: MOCK_ACCOUNT_WORK,
        items: { sessions: jasmine.arrayWithExactContents(['b', 'c']) },
        undo: true,
        dryRun: true,
      },
    ]);
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

  it('switches to the projects on disk, and remembers it', async () => {
    serveAccounts(backend, buildAccounts(Date.now(), false));
    backend.routes.set('/data/projects', () => ({ status: 200, body: { projects: [] } }));
    backend.routes.set('/data/jobs', () => ({ status: 200, body: { jobs: [] } }));
    store([card('a')]);
    let el = await render();
    const view = (label: string) =>
      [...el.querySelectorAll<HTMLButtonElement>('[aria-label="Show"] button')].find(
        (b) => text(b) === label,
      )!;
    expect(view('Sessions').getAttribute('aria-pressed')).toBe('true');
    view('Projects').click();
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    expect(el.querySelector('mc-session-card')).toBeNull();
    expect(text(el.querySelector('mc-projects-view .empty-state'))).toContain(
      'No conversations on disk',
    );
    // The page's Select is the sessions'; the projects have their own.
    expect(
      [...el.querySelectorAll('.page-head button')].map((b) => text(b)).includes('Select'),
    ).toBeFalse();
    expect(storageGet('mc_sessions_view')).toBe('projects');
    fixture.destroy();
    el = await render();
    expect(el.querySelector('mc-projects-view')).not.toBeNull();
  });
});
