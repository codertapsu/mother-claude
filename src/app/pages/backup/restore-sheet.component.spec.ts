import { ComponentFixture, TestBed } from '@angular/core/testing';

import { PRUNE_WARNING } from '../../core/backup';
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
import { RestoreSheetComponent } from './restore-sheet.component';

const H = 3_600_000;
const PRUNE =
  'Claude Code will delete this again at its next cleanup unless cleanupPeriodDays (now 30) is raised.';

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
  // A token account's sessions are in the Default's set.
  set(MOCK_ACCOUNT_PRO, 'Marcus (Pro)', { dir: '/b/Default', sharedWith: 'default' }),
];

const index = () => {
  const now = Date.now();
  const s = (id: string, action: string, extra: Record<string, unknown> = {}) => ({
    id,
    action,
    cwd: '/Users/me/dev/api',
    remoteBytes: 2048,
    host: 'marcus-air',
    pushedAt: now - H,
    newestMtime: now - H,
    ...extra,
  });
  return {
    account: 'default',
    name: 'Default',
    set: '/b/Default',
    machines: [],
    config: [],
    sessions: [
      s('aaaaaaaa-0001', 'new', { title: 'Refactor auth middleware' }),
      s('bbbbbbbb-0002', 'new', { cwd: '/Users/me/dev/old', pruneWarning: PRUNE }),
      s('cccccccc-0003', 'same', { localBytes: 2048 }),
      s('dddddddd-0004', 'live', { localBytes: 1024 }),
    ],
  };
};

const doneJob = (to = 'default'): JobView => ({
  id: 'job_r',
  kind: 'restore',
  state: 'done',
  account: 'default',
  to,
  startedAt: Date.now(),
  finishedAt: Date.now(),
  filesDone: 2,
  filesTotal: 2,
  bytesDone: 0,
  bytesTotal: 0,
  sessions: ['aaaaaaaa-0001'],
  undoUntil: Date.now() + 7 * 24 * H,
  result: {
    restored: ['aaaaaaaa-0001'],
    skipped: [
      {
        id: 'bbbbbbbb-0002',
        code: 'already_exists',
        reason: 'the target account already has session bbbbbbbb-0002',
      },
    ],
    warnings: [],
  },
});

describe('RestoreSheetComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<RestoreSheetComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend);
    serveAccounts(backend, buildAccounts(Date.now(), true));
    asDesktop();
    TestBed.inject(DataJobsService).pollMs = 1;
    backend.routes.set('/backup/index', () => ({ status: 200, body: index() }));
  });

  afterEach(() => setActiveMock(null));

  const open = async (account = 'default') => {
    fixture = TestBed.createComponent(RestoreSheetComponent);
    fixture.componentRef.setInput('sets', sets);
    fixture.componentRef.setInput('account', account);
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
  const boxes = (el: HTMLElement) => [
    ...el.querySelectorAll<HTMLInputElement>('.srow input[type="checkbox"]'),
  ];

  it('lists the set with what each session is, new ones selected', async () => {
    const el = await open();
    expect(backend.urls).toContain('/backup/index?account=default&into=default');
    const rows = [...el.querySelectorAll('.srow')];
    expect(rows.map((r) => text(r.querySelector('.act')))).toEqual([
      'New here',
      'New here',
      'Same',
      'Running here',
    ]);
    expect(boxes(el).map((b) => b.checked)).toEqual([true, true, false, false]);
    // A running session is never touched.
    expect(boxes(el)[3].disabled).toBeTrue();
    // The pruning warning sits next to its session.
    expect(text(rows[1].querySelector('.prune'))).toBe(PRUNE);
    expect(text(button(el, 'Restore'))).toBe('Restore 2 sessions');
    // What the session list called it is the main line; else its id.
    expect(text(rows[0].querySelector('.stitle'))).toBe('Refactor auth middleware');
    const parts = (r: Element) => [...r.querySelectorAll('.sline > span')].map((x) => text(x));
    expect(parts(rows[0])).toEqual(['aaaaaaaa', '~/dev/api']);
    expect(boxes(el)[0].getAttribute('aria-label')).toBe(
      'Session Refactor auth middleware: New here',
    );
    expect(rows[1].querySelector('.stitle')).toBeNull();
    expect(parts(rows[1])).toEqual(['bbbbbbbb', '~/dev/old']);
  });

  it("offers the sets of their own: a token account's is the Default's", async () => {
    const el = await open();
    const from = el.querySelectorAll<HTMLSelectElement>('.pick select')[0];
    expect([...from.options].map((o) => o.text.trim())).toEqual(['Default', 'Work']);
  });

  it('says before it starts that Claude Code would prune some again, then restores', async () => {
    backend.routes.set('/backup/restore', () => ({ status: 202, body: { job: 'job_r' } }));
    backend.routes.set('/data/jobs/job_r', () => ({ status: 200, body: doneJob() }));
    const el = await open();
    button(el, 'Restore 2 sessions')?.click();
    fixture.detectChanges();
    expect(backend.calls).not.toContain('POST /backup/restore');
    const warn = text(el.querySelector('.notice.warn'));
    expect(warn).toContain('1 of the selected sessions is older than Default keeps conversations.');
    expect(warn).toContain(PRUNE_WARNING);

    button(el, 'Restore anyway')?.click();
    await settle(5);
    expect(backend.bodyOf('POST /backup/restore')).toEqual({
      account: 'default',
      ids: ['aaaaaaaa-0001', 'bbbbbbbb-0002'],
    });
    const row = el.querySelector('mc-data-job');
    expect(row?.getAttribute('data-state')).toBe('done');
    expect(text(row)).toContain('Restore of Default');
    expect(text(row)).toContain('1 session restored · 1 skipped');
    expect(text(row)).toContain('The target account already has session bbbbbbbb-0002.');
    expect(button(el, 'Undo')).toBeDefined();
  });

  it('restores into another account, with the trust only when ticked', async () => {
    backend.routes.set('/backup/restore', () => ({
      status: 202,
      body: { job: doneJob(MOCK_ACCOUNT_WORK) },
    }));
    const el = await open();
    const into = el.querySelectorAll<HTMLSelectElement>('.pick select')[1];
    into.value = MOCK_ACCOUNT_WORK;
    into.dispatchEvent(new Event('change'));
    await settle();
    expect(backend.urls).toContain(`/backup/index?account=default&into=${MOCK_ACCOUNT_WORK}`);
    expect(text(el.querySelector('.check'))).toContain('Also trust their folders for Work');
    // Only the session without the pruning warning.
    boxes(el)[1].click();
    const trust = el.querySelector('.check input') as HTMLInputElement;
    trust.checked = true;
    trust.dispatchEvent(new Event('change'));
    fixture.detectChanges();
    button(el, 'Restore 1 session')?.click();
    await settle();
    expect(backend.bodyOf('POST /backup/restore')).toEqual({
      account: 'default',
      into: MOCK_ACCOUNT_WORK,
      ids: ['aaaaaaaa-0001'],
      trust: true,
    });
    // The job names both accounts.
    expect(text(el.querySelector('mc-data-job .jtitle'))).toBe('Restore of Default into Work');
  });

  it("says in plain words that a set failed its check, or that its key isn't here", async () => {
    let code = 'backup_unauthenticated';
    backend.routes.set('/backup/index', () => ({
      status: 409,
      body: {
        error: { code, message: 'the index of the backup set in /b/Default is not authentic' },
      },
    }));
    let el = await open();
    expect(text(el.querySelector('.notice.error'))).toBe(
      "This backup failed its check: its index was not written with this passphrase's key — someone changed it, or an older build of Mother Claude made it. Nothing was read from it.",
    );
    fixture.destroy();
    code = 'set_key_missing';
    el = await open();
    expect(text(el.querySelector('.notice.error'))).toContain(
      "The backup's key (identity.age) isn't in the folder on this Mac",
    );
  });

  it('says when the passphrase on this Mac does not open the set', async () => {
    backend.routes.set('/backup/index', () => ({
      status: 403,
      body: {
        error: {
          code: 'wrong_passphrase',
          message: 'the passphrase does not open this backup set',
        },
      },
    }));
    const el = await open();
    expect(text(el.querySelector('.notice.error'))).toContain(
      'The passphrase on this Mac does not open this backup.',
    );
    expect(button(el, 'Try again')).toBeDefined();
  });
});
