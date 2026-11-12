import { FixItem, fixItems } from './attention';
import {
  PEER_ID,
  conflicted,
  failingSetup,
  job,
  keptCopy,
  machineAsk,
  machinesView,
  peer,
  permission,
  sources,
  syncPair,
} from './mock/attention.testing';
import { accountFixture } from './mock/fixtures';

const H = 3_600_000;
const ids = (items: FixItem[]) => items.map((i) => i.id);

describe('what to fix (core/attention.ts)', () => {
  it('finds nothing while everything works', () => {
    expect(fixItems(sources())).toEqual([]);
  });

  it('a required macOS permission missing, on the desktop only', () => {
    const s = sources({ permissions: [permission(), permission({ id: 'ok', granted: true })] });
    expect(fixItems(s)).toEqual([
      expect.objectContaining({
        kind: 'permission',
        id: 'permission:full-disk-access',
        tone: 'danger',
      }),
    ]);
    expect(fixItems({ ...s, desktop: false })).toEqual([]);
    // An optional one is no blocker.
    expect(fixItems(sources({ permissions: [permission({ required: false })] }))).toEqual([]);
  });

  it("a computer's failing required checks, here and there, while Machines' dot shows", () => {
    const { view, setup } = failingSetup();
    const items = fixItems(sources({ machines: view, setup }));
    expect(items.map((i) => [i.kind, i.id, i.machine, i.machineName ?? ''])).toEqual([
      ['check', 'check:self:keepAwake', null, ''],
      ['check', `check:${PEER_ID}:firewall`, PEER_ID, 'DESKTOP-PC'],
    ]);
    expect(items.every((i) => i.tone === 'danger')).toBe(true);
    // Before the checks are read: one item says so.
    expect(fixItems(sources({ machines: view }))).toEqual([
      expect.objectContaining({ kind: 'health', id: 'health', blockers: 2 }),
    ]);
    // Machines off and never set up: no dot, nothing to fix.
    expect(fixItems(sources({ machines: { ...view, enabled: false }, setup }))).toEqual([]);
  });

  it("leaves this computer's account checks to the accounts", () => {
    const { view, setup } = failingSetup();
    setup.self.checks = [
      { ...setup.self.checks[0], id: 'account:acc_5f1c9e2b7a40', scope: 'account' },
    ];
    setup.peers = [];
    expect(fixItems(sources({ machines: view, setup }))).toEqual([]);
  });

  it('an account signed out, refused, without its token, or signed in as someone else', () => {
    const accounts = [
      accountFixture({ id: 'default', name: 'Default', isDefault: true }),
      accountFixture({ id: 'a1', name: 'Work', attention: 'signedOut', signedIn: false }),
      accountFixture({ id: 'a2', name: 'Pro', kind: 'token', attention: 'tokenMissing' }),
      accountFixture({ id: 'a3', name: 'Client X', identityWarning: { kind: 'changed' } }),
      accountFixture({ id: 'a4', name: 'Old', attention: 'authFailed' }),
      // A check that failed to run is no sign-in lost; a sign-in that runs
      // is on its way.
      accountFixture({ id: 'a5', name: 'Flaky', attention: 'checkFailed' }),
      accountFixture({ id: 'a6', name: 'Busy', attention: 'signedOut', signingIn: true }),
    ];
    const items = fixItems(sources({ accounts }));
    expect(items.map((i) => (i.kind === 'signIn' ? [i.account, i.problem, i.tone] : []))).toEqual([
      ['a1', 'signedOut', 'danger'],
      ['a2', 'tokenMissing', 'danger'],
      ['a3', 'identity', 'warn'],
      ['a4', 'authFailed', 'danger'],
    ]);
  });

  it("a paired computer's versions apart from this one's", () => {
    const machines = machinesView({
      enabled: true,
      peers: [
        peer({ skew: { app: 'older', claude: 'newer' } }),
        peer({ id: 'e'.repeat(32), name: 'BUILD-BOX', problem: 'olderApp' }),
      ],
    });
    expect(
      fixItems(sources({ machines })).map((i) =>
        i.kind === 'skew' ? [i.machineName, i.what, i.order] : [],
      ),
    ).toEqual([
      ['DESKTOP-PC', 'app', 'older'],
      ['DESKTOP-PC', 'claude', 'newer'],
      ['BUILD-BOX', 'app', 'older'],
    ]);
  });

  it('a project changed on both computers', () => {
    const items = fixItems(sources({ sync: [syncPair(), conflicted()] }));
    expect(items).toEqual([
      expect.objectContaining({ kind: 'sync', machine: PEER_ID, machineName: 'DESKTOP-PC' }),
    ]);
  });

  it('a copy paused for a reason someone can end', () => {
    const kept = ['noGrant', 'machinesOff', 'tooLarge', 'unsupported', 'offline', 'changed'].map(
      (r) => keptCopy(r, { id: `kept.${r}` }),
    );
    expect(ids(fixItems(sources({ kept })))).toEqual([
      'copy:kept.noGrant',
      'copy:kept.machinesOff',
      'copy:kept.tooLarge',
      'copy:kept.unsupported',
    ]);
  });

  it('a job that failed in the last day, unless the same ran through since or it was put away', () => {
    const failed = job();
    expect(ids(fixItems(sources({ jobs: [failed] })))).toEqual(['job:job_1']);
    // Older than a day.
    expect(fixItems(sources({ jobs: [job({ finishedAt: Date.now() - 25 * H })] }))).toEqual([]);
    // The same backup ran through afterwards.
    const done = job({ id: 'job_2', state: 'done', startedAt: Date.now() - 60_000 });
    expect(fixItems(sources({ jobs: [failed, done] }))).toEqual([]);
    // Another account's backup did not.
    const other = { ...done, account: 'default' };
    expect(ids(fixItems(sources({ jobs: [failed, other] })))).toEqual(['job:job_1']);
    // Put away on this device.
    expect(fixItems(sources({ jobs: [failed], dismissed: new Set(['job:job_1']) }))).toEqual([]);
    expect(fixItems(sources({ jobs: [job({ state: 'done' })] }))).toEqual([]);
  });

  it("another computer's ask that ran here and failed", () => {
    const machines = machinesView({
      enabled: true,
      asks: [
        machineAsk({ id: 'a_open' }),
        machineAsk({ id: 'a_failed', state: 'failed', answeredAt: Date.now() - H }),
        machineAsk({ id: 'a_old', state: 'failed', answeredAt: Date.now() - 30 * H }),
      ],
    });
    expect(ids(fixItems(sources({ machines })))).toEqual(['ask:a_failed']);
  });

  it('an update ready to install, on the desktop', () => {
    expect(fixItems(sources({ update: '0.12.0' }))).toEqual([
      expect.objectContaining({ kind: 'update', version: '0.12.0', tone: 'info' }),
    ]);
    expect(fixItems(sources({ update: '0.12.0', desktop: false }))).toEqual([]);
  });

  it('puts what blocks first', () => {
    const { view, setup } = failingSetup();
    const items = fixItems(
      sources({
        update: '0.12.0',
        jobs: [job()],
        sync: [conflicted()],
        machines: view,
        setup,
        permissions: [permission()],
      }),
    );
    expect(items.map((i) => i.kind)).toEqual([
      'permission',
      'check',
      'check',
      'sync',
      'job',
      'update',
    ]);
  });
});
