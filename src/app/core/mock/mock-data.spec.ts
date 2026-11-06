import { AccountView, SessionSummary } from '../models';
import { accountFixture } from './fixtures';
import { DataHost, MockData, WIN_HOST, WIN_MACHINE } from './mock-data';

const s = (id: string, over: Partial<SessionSummary> = {}): SessionSummary =>
  ({
    id,
    cwd: '/Users/marcus/dev/api',
    title: `Title ${id}`,
    live: false,
    status: 'idle',
    messageCount: 1,
    ...over,
  }) as SessionSummary;

describe('MockData backups (the mock backend of Backup & sync, as the server answers)', () => {
  let desktop: boolean;
  let mock: MockData;

  beforeEach(() => {
    jasmine.clock().install();
    jasmine.clock().mockDate(new Date(1_800_000_000_000));
    desktop = true;
    const accounts: AccountView[] = [
      accountFixture({ id: 'default', name: 'Default', isDefault: true, dir: '/u/.claude' }),
    ];
    const sessions = [s('a1'), s('a2', { live: true })];
    const host: DataHost = {
      get desktop() {
        return desktop;
      },
      sessions: () => sessions,
      accounts: () => accounts,
      accountOf: () => 'default',
      reassign: () => undefined,
    };
    mock = new MockData(host);
  });

  afterEach(() => jasmine.clock().uninstall());

  const call = async (path: string, method: string, body?: unknown, q = '') => {
    const res = await mock.route(path, method, body, new URLSearchParams(q));
    return res ? { status: res.status, body: await res.json() } : null;
  };
  const job = async (id: string) => (await call(`/data/jobs/${id}`, 'GET'))?.body;
  const ready = async () => {
    await call('/backup/settings', 'PUT', { folder: '/Volumes/NAS/MotherClaude' });
    await call('/backup/passphrase', 'POST', { passphrase: 'correct horse battery staple' });
    const first = await call('/backup/push', 'POST', {});
    jasmine.clock().tick(2_000);
    return first;
  };

  it('refuses a force from a device without the dangerous right', async () => {
    await ready();
    desktop = false;
    for (const [path, body] of [
      ['/backup/push', { force: true }],
      ['/backup/pull', { account: 'default', force: true }],
      ['/backup/restore', { account: 'default', ids: ['a1'], force: true }],
    ] as const) {
      const r = await call(path, 'POST', body);
      expect([r?.status, r?.body.error.code]).toEqual([403, 'dangerous_blocked']);
    }
    // Without force a device may, as on the server.
    expect((await call('/backup/push', 'POST', {}))?.status).toBe(202);
  });

  it('force-pulls: what differs here is replaced, a running session is skipped', async () => {
    await ready();
    const plan = (await call('/backup/index', 'GET', undefined, 'account=default'))?.body;
    const a1 = plan.sessions.find((x: { id: string }) => x.id === 'a1');
    expect([a1.action, a1.differs]).toEqual(['localNewer', true]);
    const res = await call('/backup/pull', 'POST', { account: 'default', force: true });
    expect(res?.status).toBe(202);
    jasmine.clock().tick(2_000);
    const done = await job(res?.body.jobs[0]);
    expect(done.state).toBe('done');
    expect(done.result.force).toBeTrue();
    expect(done.result.replaced).toEqual(['a1']);
    expect(done.result.conflicts).toEqual([]);
    expect(done.result.skipped.map((x: { id: string; code: string }) => [x.id, x.code])).toEqual([
      ['a2', 'session_live'],
    ]);
    expect(done.undoUntil).toBeGreaterThan(Date.now());
  });

  it('force-backs up as its own job, beside a backup that runs', async () => {
    await ready();
    const plain = await call('/backup/push', 'POST', {});
    const forced = await call('/backup/push', 'POST', { force: true });
    expect(forced?.body.jobs[0]).not.toBe(plain?.body.jobs[0]);
    // A second force answers the first.
    const again = await call('/backup/push', 'POST', { force: true });
    expect(again?.body.jobs[0]).toBe(forced?.body.jobs[0]);
    jasmine.clock().tick(2_000);
    const done = await job(forced?.body.jobs[0]);
    expect(done.result.force).toBeTrue();
  });

  it('keeps folder pairs as the server does: checked, listed, never twice, removed', async () => {
    const pair = (body: unknown) => call('/backup/pairs', 'POST', body);
    // Nothing to keep them in before a backup.
    await call('/backup/settings', 'PUT', { folder: '/Volumes/NAS/MotherClaude' });
    await call('/backup/passphrase', 'POST', { passphrase: 'correct horse battery staple' });
    const none = await pair({
      folder: '/Users/m/dev',
      machine: WIN_MACHINE,
      otherFolder: 'D:\\dev',
    });
    expect([none?.status, none?.body.error.code]).toEqual([422, 'invalid_pair']);
    await ready();
    // A machine the backups do not know.
    let r = await pair({ folder: '/Users/m/dev', machine: WIN_MACHINE, otherFolder: 'D:\\dev' });
    expect(r?.body.error.code).toBe('invalid_pair');
    mock.noteMachine('default', WIN_MACHINE, WIN_HOST);
    const listed = (await call('/backup/pairs', 'GET'))?.body;
    expect(listed.machines).toEqual([{ id: WIN_MACHINE, name: WIN_HOST, os: 'windows' }]);
    // Each folder the way its machine writes paths.
    r = await pair({ folder: 'dev', machine: WIN_MACHINE, otherFolder: 'D:\\dev' });
    expect(r?.body.error.code).toBe('invalid_pair');
    r = await pair({ folder: '/Users/m/dev', machine: WIN_MACHINE, otherFolder: '/Users/m/dev' });
    expect(r?.body.error.message).toContain('is not a Windows path');
    r = await pair({ folder: '/Users/m/dev', machine: WIN_MACHINE, otherFolder: 'D:\\dev', x: 1 });
    expect(r?.body.error.code).toBe('invalid_body');
    // A device may (as with the backup settings).
    desktop = false;
    r = await pair({ folder: '/Users/m/dev/', machine: WIN_MACHINE, otherFolder: 'D:\\dev\\' });
    expect(r?.status).toBe(201);
    expect(r?.body.pairs[0].folders.map((f: { folder: string }) => f.folder)).toEqual([
      '/Users/m/dev',
      'D:\\dev',
    ]);
    r = await pair({ folder: '/Users/m/dev', machine: WIN_MACHINE, otherFolder: 'd:/DEV' });
    expect(r?.body.error.message).toBe('these two folders are paired already');
    expect((await call('/backup', 'GET'))?.body.folderPairs.pairs.length).toBe(1);
    const id = (await call('/backup/pairs', 'GET'))?.body.pairs[0].id;
    expect((await call(`/backup/pairs/${id}`, 'DELETE'))?.body.pairs).toEqual([]);
    expect((await call(`/backup/pairs/${id}`, 'DELETE'))?.body.error.code).toBe('pair_not_found');
  });

  it("pairs a Windows desktop's folder its way, the other machine's in its form", async () => {
    localStorage.setItem('mc_mock_os', 'windows');
    try {
      await call('/backup/settings', 'PUT', { folder: 'D:\\Backups\\Mother Claude' });
      await call('/backup/passphrase', 'POST', { passphrase: 'correct horse battery staple' });
      await call('/backup/push', 'POST', {});
      jasmine.clock().tick(2_000);
      // A machine that has not said its system yet.
      const box = 'ab'.repeat(16);
      mock.noteMachine('default', box, 'linux-box');
      const pair = (body: unknown) => call('/backup/pairs', 'POST', body);
      let r = await pair({ folder: '/Users/m/dev', machine: box, otherFolder: '/home/m/dev' });
      expect(r?.body.error.message).toBe(
        '/Users/m/dev is not a full path on this computer: start it with a drive like C:\\ or a share like \\\\server\\share (or ~ for your home folder)',
      );
      // `~\` is the home folder here; the other side is written plainly.
      r = await pair({ folder: '~\\dev', machine: box, otherFolder: '/home/m//dev/' });
      expect(r?.status).toBe(201);
      expect(r?.body.os).toBe('windows');
      expect(r?.body.pairs[0].folders.map((f: { folder: string }) => f.folder)).toEqual([
        'C:\\Users\\marcus\\dev',
        '/home/m/dev',
      ]);
      // Its form is its folder's own while it has not said.
      r = await pair({ folder: 'd:/work', machine: box, otherFolder: 'e:/work/' });
      expect(r?.body.pairs[1].folders.map((f: { folder: string }) => f.folder)).toEqual([
        'D:\\work',
        'E:\\work',
      ]);
      r = await pair({ folder: 'D:\\WORK', machine: box, otherFolder: 'E:\\Work' });
      expect(r?.body.error.message).toBe('these two folders are paired already');
    } finally {
      localStorage.removeItem('mc_mock_os');
    }
  });

  it('force-restores over a copy here, and skips one as the backup has it', async () => {
    await ready();
    const res = await call('/backup/restore', 'POST', {
      account: 'default',
      ids: ['a1'],
      force: true,
    });
    jasmine.clock().tick(2_000);
    const done = await job(res?.body.job);
    expect(done.result.replaced).toEqual(['a1']);
    expect(done.result.force).toBeTrue();
    expect(done.sessions).toEqual(['a1']);
  });
});
