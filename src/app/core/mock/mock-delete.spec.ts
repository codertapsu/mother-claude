import { AccountView, JobView, SessionSummary } from '../models';
import { accountFixture } from './fixtures';
import { MockRole } from './mock-api';
import { DeleteHost, MockDelete } from './mock-delete';
import { JobEnd, JobNames, MockData } from './mock-data';

const s = (id: string, cwd: string, over: Partial<SessionSummary> = {}): SessionSummary =>
  ({ id, cwd, title: `Title ${id}`, live: false, messageCount: 1, ...over }) as SessionSummary;

describe('MockDelete (the mock backend of deleting, as the server answers)', () => {
  let role: MockRole;
  let sessions: SessionSummary[];
  let owners: Record<string, string>;
  let started: { kind: string; names: JobNames; sessions: string[]; done: () => JobEnd }[];
  let mock: MockDelete;

  beforeEach(() => {
    role = 'desktop';
    sessions = [
      s('a1', '/p/api'),
      s('a2', '/p/api', { live: true }),
      s('w1', '/p/web'),
      s('t1', '/p/api'),
    ];
    owners = { w1: 'acc_work', t1: 'acc_pro' };
    started = [];
    const accounts: AccountView[] = [
      accountFixture({ id: 'default', name: 'Default', isDefault: true, dir: '/u/.claude' }),
      accountFixture({ id: 'acc_work', name: 'Work', dir: '/u/.claude-work' }),
      accountFixture({ id: 'acc_pro', name: 'Pro', kind: 'token' }),
    ];
    const host: DeleteHost = {
      get role() {
        return role;
      },
      sessions: () => sessions,
      accounts: () => accounts,
      accountOf: (id) => owners[id] ?? 'default',
      removeSessions: () => () => undefined,
      openThreads: () => ['b1'],
    };
    const data = {
      runJob: (
        kind: string,
        names: JobNames,
        _files: number,
        _bytes: number,
        ids: string[],
        _ms: number,
        done: () => JobEnd,
      ): JobView => {
        started.push({ kind, names, sessions: ids, done });
        return { id: `j${started.length}`, kind, state: 'running', ...names } as JobView;
      },
    } as unknown as MockData;
    mock = new MockDelete(host, data);
  });

  const call = async (path: string, method: string, body?: unknown) => {
    const res = mock.route(path, method, body);
    return res ? { status: res.status, body: res.status === 204 ? null : await res.json() } : null;
  };

  it('refuses a request without items, or with nothing named, as the server does', async () => {
    expect((await call('/data/delete', 'POST', { undo: true }))?.body.error.code).toBe(
      'invalid_body',
    );
    const none = await call('/data/delete', 'POST', { items: {}, dryRun: true });
    expect([none?.status, none?.body.error.code]).toEqual([422, 'invalid_transfer']);
    const nobody = await call('/data/delete', 'POST', {
      from: 'acc_x',
      items: { sessions: ['a1'] },
    });
    expect([nobody?.status, nobody?.body.error.code]).toEqual([404, 'not_found']);
    role = 'device';
    const device = await call('/data/delete', 'POST', {
      items: { sessions: ['a1'] },
      dryRun: true,
    });
    expect(device?.body.error.code).toBe('dangerous_blocked');
  });

  it('plans with plain ids, titled skips and the totals', async () => {
    const r = await call('/data/delete', 'POST', {
      from: 'default',
      items: { projects: ['/p/api/'], sessions: ['b1', 'zz'] },
      dryRun: true,
    });
    const plan = r?.body.plan;
    expect(plan.from).toBe('default');
    expect(plan.sessions).toEqual([
      jasmine.objectContaining({ id: 'a1', account: 'default', title: 'Title a1' }),
    ]);
    expect(plan.skipped).toEqual([
      { id: 'b1', code: 'session_live', message: 'session b1 is running; stop it first' },
      jasmine.objectContaining({ id: 'zz', code: 'not_found' }),
      {
        id: 'a2',
        title: 'Title a2',
        code: 'session_live',
        message: 'session a2 is running; stop it first',
      },
    ]);
    expect([plan.sessionsTotal, plan.skippedTotal, plan.projectsTotal]).toEqual([1, 3, 1]);
    // a2 stays (it runs), so the folder is not left empty.
    expect(plan.foldersTotal).toBe(0);
  });

  it("names each job after the account asked for, else the directory's", async () => {
    await call('/data/delete', 'POST', { from: 'acc_pro', items: { sessions: ['t1'] } });
    expect(started.map((j) => [j.names.account, j.sessions])).toEqual([['acc_pro', ['t1']]]);
    started = [];
    await call('/data/delete', 'POST', { items: { sessions: ['a1', 'w1', 't1'] } });
    expect(started.map((j) => [j.names.account, j.sessions])).toEqual([
      ['default', ['a1', 't1']],
      ['acc_work', ['w1']],
    ]);
    // Nothing to delete still makes one job, with what it skipped.
    started = [];
    await call('/data/delete', 'POST', { items: { sessions: ['a2'] } });
    expect(started.length).toBe(1);
    expect(started[0].done().result).toEqual({
      deleted: [],
      skipped: [jasmine.objectContaining({ id: 'a2', code: 'session_live' })],
    });
  });

  it("reveals a project's folder, and says when the account has none of it", async () => {
    const ok = await call('/data/projects/reveal', 'POST', { account: 'default', cwd: '/p/api/' });
    expect(ok?.status).toBe(204);
    const gone = await call('/data/projects/reveal', 'POST', {
      account: 'acc_work',
      cwd: '/p/api',
    });
    expect([gone?.status, gone?.body.error.code]).toEqual([404, 'project_not_found']);
  });
});
