import { ComponentFixture, TestBed } from '@angular/core/testing';

import { DataJobsService } from '../../core/data-jobs.service';
import {
  FakeBackend,
  asDesktop,
  flush,
  serveAccounts,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import { MOCK_ACCOUNT_PRO, MOCK_ACCOUNT_WORK, buildAccounts } from '../../core/mock/fixtures';
import { setActiveMock } from '../../core/mock/mock-api';
import { BackupSet, JobView } from '../../core/models';
import { PullSheetComponent } from './pull-sheet.component';

const H = 3_600_000;

const set = (account: string, name: string, over: Partial<BackupSet> = {}): BackupSet => ({
  account,
  name,
  dir: `/b/${name}`,
  exists: true,
  keyMissing: false,
  machines: [],
  ...over,
});

const sets: BackupSet[] = [
  set('default', 'Default'),
  set(MOCK_ACCOUNT_WORK, 'Work'),
  // A token account's set is the Default's (the server lists it, with the
  // Default's details).
  set(MOCK_ACCOUNT_PRO, 'Marcus (Pro)', { dir: '/b/Default', sharedWith: 'default' }),
];

const s = (id: string, action: string) => ({
  id,
  action,
  cwd: '/Users/me/dev/api',
  remoteBytes: 2048,
  localBytes: 1024,
  host: 'marcus-air',
  pushedAt: Date.now() - H,
  newestMtime: Date.now() - H,
});

const indexOf = (account: string) =>
  account === 'default'
    ? {
        account,
        name: 'Default',
        set: '/b/Default',
        machines: [],
        config: [],
        sessions: [
          s('a0000001', 'new'),
          s('a0000002', 'update'),
          s('a0000003', 'conflict'),
          s('a0000004', 'live'),
          s('a0000005', 'same'),
        ],
      }
    : {
        account,
        name: 'Work',
        set: '/b/Work',
        machines: [],
        config: [],
        sessions: [s('w0000001', 'new'), s('w0000002', 'localNewer')],
      };

const pullJob = (id: string, result: unknown, account = 'default'): JobView => ({
  id,
  kind: 'pull',
  state: 'done',
  account,
  startedAt: Date.now(),
  finishedAt: Date.now(),
  filesDone: 0,
  filesTotal: 0,
  bytesDone: 0,
  bytesTotal: 0,
  sessions: [],
  undoUntil: Date.now() + 7 * 24 * H,
  result,
});

describe('PullSheetComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<PullSheetComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend);
    serveAccounts(backend, buildAccounts(Date.now(), true));
    asDesktop();
    TestBed.inject(DataJobsService).pollMs = 1;
    backend.routes.set('/backup/index', () => {
      const url = backend.urls[backend.urls.length - 1];
      const account = new URLSearchParams(url.split('?')[1]).get('account') ?? '';
      return { status: 200, body: indexOf(account) };
    });
  });

  afterEach(() => setActiveMock(null));

  const open = async (account: string | null) => {
    fixture = TestBed.createComponent(PullSheetComponent);
    fixture.componentRef.setInput('sets', sets);
    fixture.componentRef.setInput('account', account);
    fixture.componentRef.setInput('open', true);
    fixture.detectChanges();
    await settle();
    return fixture.nativeElement as HTMLElement;
  };
  const settle = async (times = 3) => {
    for (let i = 0; i < times; i++) {
      await flush();
      fixture.detectChanges();
    }
  };
  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const button = (el: HTMLElement, label: string) =>
    [...el.querySelectorAll<HTMLButtonElement>('button')].find((b) => text(b).startsWith(label));

  it('says what a pull does with each session before anything is written', async () => {
    const el = await open('default');
    expect(backend.urls).toEqual(['/backup/index?account=default']);
    const open_ = [...el.querySelectorAll('.rows > .srow')].filter((r) => !r.closest('details'));
    expect(open_.map((r) => text(r.querySelector('.act')))).toEqual([
      'New here',
      "Newer in the backup — replaces this Mac's copy, kept for undo",
      'Changed on both — kept side by side',
    ]);
    // What needs nothing is folded away, a running one among them.
    const rest = el.querySelector('details.rest');
    expect(text(rest?.querySelector('summary'))).toBe(
      '2 need nothing (the same, newer here, or running here)',
    );
    expect(text(rest)).toContain('Running here — skipped');
    expect(text(button(el, 'Pull'))).toBe('Pull 3 sessions');
    expect(backend.calls.some((c) => c.startsWith('POST'))).toBeFalse();
  });

  it('pulls every account that has a backup, and lists the conflicts it kept', async () => {
    let n = 0;
    backend.routes.set('/backup/pull', () => ({
      status: 202,
      body: { jobs: [`job_p${++n}`], skipped: [] },
    }));
    backend.routes.set('/data/jobs/job_p1', () => ({
      status: 200,
      body: pullJob('job_p1', {
        restored: ['a0000001'],
        updated: ['a0000002'],
        conflicts: [
          ['a0000003', '/Users/me/.claude/projects/x/a0000003.conflict-marcus-air-17.jsonl'],
        ],
        skipped: [],
        warnings: [],
        historyLines: 4,
      }),
    }));
    backend.routes.set('/data/jobs/job_p2', () => ({
      status: 200,
      body: pullJob(
        'job_p2',
        {
          restored: ['w0000001'],
          updated: [],
          conflicts: [],
          skipped: [],
          warnings: [],
          historyLines: 0,
        },
        MOCK_ACCOUNT_WORK,
      ),
    }));
    const el = await open(null);
    // Each set of its own once (the token account's is the Default's),
    // under its account's name.
    expect(backend.urls.filter((u) => u.startsWith('/backup/index')).length).toBe(2);
    expect([...el.querySelectorAll('.grp h3')].map((h) => text(h))).toEqual(['Default', 'Work']);
    expect(text(button(el, 'Pull'))).toBe('Pull 4 sessions');

    button(el, 'Pull 4 sessions')?.click();
    await settle(6);
    const pulls = backend.calls
      .map((c, i) => [c, backend.bodies[i]] as const)
      .filter(([c]) => c === 'POST /backup/pull')
      .map(([, b]) => b);
    expect(pulls).toEqual([
      { account: 'default', ids: ['a0000001', 'a0000002', 'a0000003'] },
      { account: MOCK_ACCOUNT_WORK, ids: ['w0000001'] },
    ]);
    const rows = [...el.querySelectorAll('mc-data-job')];
    expect(rows.map((r) => r.getAttribute('data-state'))).toEqual(['done', 'done']);
    expect(text(rows[0])).toContain('Pull of Default');
    expect(text(rows[0])).toContain(
      '1 new · 1 updated · 1 conflict · 4 prompt-history lines added',
    );
    expect(text(rows[0].querySelector('mc-job-details p'))).toBe(
      "Changed on both — the backup's copy is kept beside this Mac's:",
    );
    expect(text(rows[0].querySelector('mc-job-details li'))).toBe(
      'a0000003 → ~/.claude/projects/x/a0000003.conflict-marcus-air-17.jsonl',
    );
    expect(text(rows[1])).toContain('Pull of Work');
  });

  it("says why a set can't be pulled: its key isn't here", async () => {
    backend.routes.set('/backup/index', () => {
      const url = backend.urls[backend.urls.length - 1];
      const account = new URLSearchParams(url.split('?')[1]).get('account') ?? '';
      return account === MOCK_ACCOUNT_WORK
        ? {
            status: 409,
            body: {
              error: { code: 'set_key_missing', message: 'the backup set in /b/Work has no key' },
            },
          }
        : { status: 200, body: indexOf(account) };
    });
    fixture = TestBed.createComponent(PullSheetComponent);
    fixture.componentRef.setInput('sets', [
      sets[0],
      set(MOCK_ACCOUNT_WORK, 'Work', { exists: false, keyMissing: true }),
      sets[2],
    ]);
    fixture.componentRef.setInput('account', null);
    fixture.componentRef.setInput('open', true);
    fixture.detectChanges();
    await settle();
    const el = fixture.nativeElement as HTMLElement;
    const groups = [...el.querySelectorAll('.grp')];
    expect(groups.map((g) => text(g.querySelector('h3')))).toEqual(['Default', 'Work']);
    expect(text(groups[1].querySelector('.notice.error'))).toContain(
      "The backup's key (identity.age) isn't in the folder on this Mac",
    );
    // The Default's sessions can still be pulled.
    expect(text(button(el, 'Pull'))).toBe('Pull 3 sessions');
  });

  it("names the sets a pull passed over, from the server's objects or its sentences", async () => {
    backend.routes.set('/backup/pull', () => ({
      status: 202,
      body: {
        jobs: ['job_y'],
        skipped: [
          {
            account: MOCK_ACCOUNT_WORK,
            code: 'invalid_backup',
            message: 'there is no backup of Work',
          },
          'an older server said this',
        ],
      },
    }));
    backend.routes.set('/data/jobs/job_y', () => ({
      status: 200,
      body: pullJob('job_y', { restored: [], updated: [], conflicts: [] }),
    }));
    const el = await open('default');
    button(el, 'Pull 3 sessions')?.click();
    await settle(5);
    const lines = [...el.querySelectorAll('.runs ~ p.muted')].map((p) => text(p));
    expect(lines).toEqual(['Work: There is no backup of Work.', 'An older server said this.']);
  });

  it('sends only what stays ticked', async () => {
    backend.routes.set('/backup/pull', () => ({
      status: 202,
      body: { jobs: ['job_x'], skipped: [] },
    }));
    backend.routes.set('/data/jobs/job_x', () => ({
      status: 200,
      body: pullJob('job_x', { restored: [], updated: [], conflicts: [] }),
    }));
    const el = await open('default');
    const [first] = [...el.querySelectorAll<HTMLInputElement>('.srow input[type="checkbox"]')];
    first.click();
    fixture.detectChanges();
    const trust = el.querySelector('.check input') as HTMLInputElement;
    trust.checked = true;
    trust.dispatchEvent(new Event('change'));
    fixture.detectChanges();
    button(el, 'Pull 2 sessions')?.click();
    await settle();
    expect(backend.bodyOf('POST /backup/pull')).toEqual({
      account: 'default',
      ids: ['a0000002', 'a0000003'],
      trust: true,
    });
  });
});
