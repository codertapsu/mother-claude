import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { AuthService } from '../core/auth.service';
import { DataJobsService } from '../core/data-jobs.service';
import {
  FakeBackend,
  asDesktop,
  flush,
  me,
  serveAccounts,
  useFakeServer,
} from '../core/mock/fake-server.testing';
import { MOCK_ACCOUNT_WORK, buildAccounts } from '../core/mock/fixtures';
import { setActiveMock } from '../core/mock/mock-api';
import { JobView, SessionSummary } from '../core/models';
import { RealtimeService } from '../core/realtime.service';
import {
  DeleteDataSheetComponent,
  DeleteStart,
  countedWhat,
  deletedIds,
} from './delete-data-sheet.component';

const session = (id: string, title: string): SessionSummary =>
  ({ id, title, name: id, cwd: '/Users/me/dev/api', projectName: 'api' }) as SessionSummary;

const job = (id: string, account: string, over: Partial<JobView> = {}): JobView => ({
  id,
  kind: 'delete',
  state: 'running',
  startedAt: Date.now(),
  filesDone: 0,
  filesTotal: 3,
  bytesDone: 0,
  bytesTotal: 300,
  sessions: [],
  account,
  ...over,
});

describe('DeleteDataSheetComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<DeleteDataSheetComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    serveAccounts(backend, buildAccounts(Date.now(), true));
    asDesktop();
    TestBed.inject(DataJobsService).pollMs = 1;
  });

  afterEach(() => setActiveMock(null));

  const settle = async (times = 4) => {
    for (let i = 0; i < times; i++) {
      await flush();
      fixture.detectChanges();
    }
  };
  const open = async (start: DeleteStart) => {
    fixture = TestBed.createComponent(DeleteDataSheetComponent);
    fixture.componentRef.setInput('start', start);
    fixture.componentRef.setInput('open', true);
    fixture.detectChanges();
    await settle();
    return fixture.nativeElement as HTMLElement;
  };
  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const button = (el: HTMLElement, label: string) =>
    [...el.querySelectorAll<HTMLButtonElement>('button')].find((b) => text(b).startsWith(label));
  const bodies = () =>
    backend.calls
      .map((c, i) => (c === 'POST /data/delete' ? backend.bodies[i] : null))
      .filter((b): b is Record<string, unknown> => !!b);

  /** Answers a dry run per account, and the runs with a job each. */
  const serve = () =>
    backend.routes.set('/data/delete', (_m, raw) => {
      const body = raw as { from?: string; dryRun?: boolean };
      const work = body.from === MOCK_ACCOUNT_WORK;
      if (body.dryRun) {
        return {
          status: 200,
          body: {
            plan: work
              ? { sessions: [{ id: 'w1', files: 2, bytes: 1024, account: { account: body.from } }] }
              : {
                  sessions: [
                    {
                      id: 's1',
                      title: 'Fix the login',
                      cwd: '/Users/me/dev/api',
                      files: 3,
                      bytes: 2048,
                    },
                    { id: 's2', files: 1, bytes: 1024 },
                  ],
                  skipped: [
                    {
                      id: 's3',
                      code: 'session_live',
                      message: 'it is open in the API: close it there first',
                    },
                  ],
                },
          },
        };
      }
      const j = job(work ? 'jw' : 'jd', body.from ?? 'default');
      return { status: 202, body: { jobs: [j], job: j } };
    });

  it('opens on the dry run of each account, and names the count on its button', async () => {
    serve();
    const el = await open({
      what: '4 conversations',
      groups: [
        { from: 'default', sessions: ['s1', 's2', 's3'] },
        { from: MOCK_ACCOUNT_WORK, sessions: ['w1'] },
      ],
      known: [session('s2', 'Cache the lookups'), session('s3', 'Open in the API')],
    });
    expect(bodies()).toEqual([
      { from: 'default', items: { sessions: ['s1', 's2', 's3'] }, undo: true, dryRun: true },
      { from: MOCK_ACCOUNT_WORK, items: { sessions: ['w1'] }, undo: true, dryRun: true },
    ]);
    expect(text(el.querySelector('.planline'))).toBe('Deletes 3 conversations (6 files, 4.0 KB).');
    expect(text(el)).toContain('It runs as 2 jobs');
    const goes = text(el.querySelector('[aria-label="What goes"]'));
    expect(goes).toContain('Fix the login');
    expect(goes).toContain('Cache the lookups');
    expect(goes).toContain('in Work');
    expect(text(el.querySelector('.skips .ptitle'))).toBe('Open in the API');
    expect(text(el.querySelector('.skips .why'))).toBe(
      'It is open in the API: close it there first.',
    );
    expect(button(el, 'Delete 3 conversations')).toBeDefined();
    expect((el.querySelector('.check input') as HTMLInputElement).checked).toBeTrue();
  });

  it('deletes for good when the copy is turned off, one job per account', async () => {
    serve();
    let polls = 0;
    const done = (id: string, account: string, ids: string[]) =>
      job(id, account, { state: 'done', result: { deleted: ids, skipped: [] }, sessions: ids });
    backend.routes.set('/data/jobs/jd', () => ({
      status: 200,
      body: ++polls < 2 ? job('jd', 'default') : done('jd', 'default', ['s1', 's2']),
    }));
    backend.routes.set('/data/jobs/jw', () => ({
      status: 200,
      body: done('jw', MOCK_ACCOUNT_WORK, ['w1']),
    }));
    const closed = jasmine.createSpy('closed');
    const el = await open({
      what: '3 conversations',
      groups: [
        { from: 'default', sessions: ['s1', 's2'] },
        { from: MOCK_ACCOUNT_WORK, sessions: ['w1'] },
      ],
    });
    fixture.componentInstance.closed.subscribe(closed);
    const keep = el.querySelector('.check input') as HTMLInputElement;
    keep.click();
    fixture.detectChanges();
    expect(text(el.querySelector('.notice.warn'))).toBe(
      'Deleted for good, right away: there is no Undo.',
    );
    button(el, 'Delete 3 conversations')?.click();
    await settle();
    await new Promise((r) => setTimeout(r, 20));
    await settle();
    expect(bodies().slice(2)).toEqual([
      { from: 'default', items: { sessions: ['s1', 's2'] }, undo: false, dryRun: false },
      { from: MOCK_ACCOUNT_WORK, items: { sessions: ['w1'] }, undo: false, dryRun: false },
    ]);
    const rows = [...el.querySelectorAll('mc-data-job')];
    expect(rows.map((r) => text(r.querySelector('.jtitle')))).toEqual([
      'Delete from Default',
      'Delete from Work',
    ]);
    expect(text(rows[0])).toContain('Deleted 2 conversations');
    // Nothing kept: no Undo.
    expect(el.querySelector('.undo')).toBeNull();
    expect(el.querySelector('.later')).toBeNull();
    button(el, 'Close')?.click();
    const ran = closed.calls.mostRecent().args[0] as JobView[];
    expect([...deletedIds(ran)]).toEqual(['s1', 's2', 'w1']);
  });

  it('keeps a copy by default, and its job offers Undo', async () => {
    serve();
    backend.routes.set('/data/jobs/jd', () => ({
      status: 200,
      body: job('jd', 'default', {
        state: 'done',
        result: { deleted: ['s1', 's2'], skipped: [] },
        undoUntil: Date.now() + 7 * 86_400_000,
      }),
    }));
    backend.routes.set('/data/jobs/jd/undo', () => ({
      status: 200,
      body: job('jd', 'default', { state: 'undone' }),
    }));
    const el = await open({
      what: 'this project',
      groups: [{ from: 'default', projects: ['/p'] }],
    });
    button(el, 'Delete 2 conversations')?.click();
    await settle();
    await new Promise((r) => setTimeout(r, 20));
    await settle();
    expect((bodies()[1] as { undo: boolean }).undo).toBeTrue();
    expect(text(el.querySelector('.undo'))).toContain('Undo');
    expect(text(el.querySelector('.later'))).toBe(
      'Undo stays in Sessions › Projects, under Recently deleted, for seven days.',
    );
    button(el, 'Undo')?.click();
    fixture.detectChanges();
    button(el, 'Tap again to undo')?.click();
    await settle();
    expect(backend.calls).toContain('POST /data/jobs/jd/undo');
    expect(el.querySelector('mc-data-job')?.getAttribute('data-state')).toBe('undone');
  });

  it('tells a device without the dangerous right, and asks the Mac nothing', async () => {
    TestBed.inject(AuthService).me.set(me({ allowDangerous: false }));
    const el = await open({ what: 'this conversation', groups: [{ sessions: ['s1'] }] });
    expect(text(el.querySelector('.notice.warn'))).toBe(
      'Deleting conversations needs dangerous actions for this device, granted on the Mac in Remote › Devices.',
    );
    expect(backend.calls).not.toContain('POST /data/delete');
    expect(button(el, 'Delete')).toBeUndefined();
  });

  it('puts a refusal in words, and checks again on asking', async () => {
    let calls = 0;
    backend.routes.set('/data/delete', () =>
      ++calls === 1
        ? {
            status: 409,
            body: {
              error: {
                code: 'data_job_running',
                message: 'a backup, restore or transfer is running',
              },
            },
          }
        : {
            status: 200,
            body: { plan: { sessions: [], skipped: [{ id: 's1', code: 'not_found' }] } },
          },
    );
    const el = await open({ what: 'this conversation', groups: [{ sessions: ['s1'] }] });
    expect(text(el.querySelector('.notice.error'))).toContain(
      'A backup, restore or transfer is running',
    );
    button(el, 'Try again')?.click();
    await settle();
    expect(text(el.querySelector('.planline'))).toBe('There is nothing to delete here.');
    expect(text(el.querySelector('.skips'))).toContain('It is not there any more.');
    expect(button(el, 'Nothing to delete')?.disabled).toBeTrue();
  });

  it('names a skipped conversation from the store, and the accounts only when several', async () => {
    (
      TestBed.inject(RealtimeService) as unknown as { replaceStore(l: SessionSummary[]): void }
    ).replaceStore([{ ...session('live1', 'Running right now'), live: true }]);
    backend.routes.set('/data/delete', () => ({
      status: 200,
      body: {
        plan: {
          sessions: [{ id: 'p1', files: 3, bytes: 300, account: 'default' }],
          skipped: [{ id: 'live1', code: 'session_live' }],
        },
      },
    }));
    const el = await open({
      what: 'this project',
      groups: [{ from: 'default', projects: ['/p'] }],
    });
    expect(text(el.querySelector('.skips .ptitle'))).toBe('Running right now');
    expect(text(el.querySelector('[aria-label="What goes"]'))).not.toContain('in Default');
  });

  it('names what it deletes', () => {
    expect(countedWhat(1, 'conversation')).toBe('this conversation');
    expect(countedWhat(3, 'project')).toBe('3 projects');
  });
});
