import { ApiError } from './api.service';
import {
  actionText,
  andList,
  backupErrorText,
  jobSummary,
  normalizeBackupIndex,
  normalizeBackupView,
  normalizePassphraseChange,
  normalizePullResult,
  normalizePullSkips,
  normalizePushResult,
  otherMachines,
  ownSets,
  passphraseFailure,
  preselect,
  prunedOf,
  scheduleLabel,
} from './backup';
import { BackupIndexSession, JobView } from './models';

const s = (id: string, action: BackupIndexSession['action'], pruneWarning?: string) =>
  ({
    id,
    action,
    remoteBytes: 1,
    host: 'mac',
    pushedAt: 1,
    newestMtime: 1,
    ...(pruneWarning ? { pruneWarning } : {}),
  }) as BackupIndexSession;

const done = (kind: JobView['kind'], result: unknown): JobView => ({
  id: 'j',
  kind,
  state: 'done',
  startedAt: 1,
  filesDone: 0,
  filesTotal: 0,
  bytesDone: 0,
  bytesTotal: 0,
  sessions: [],
  result,
});

describe('backup helpers', () => {
  it('reads the backup view tolerantly', () => {
    const v = normalizeBackupView({
      settings: { folder: '/Users/me/iCloud/Mother Claude', everyHours: 6.2, pullOnLaunch: 'yes' },
      passphraseSet: true,
      host: 'marcus-mbp',
      sets: [
        {
          account: 'default',
          name: 'Default',
          dir: '/x/Default',
          exists: true,
          lastPush: 5,
          machines: [{ id: 'm1', host: 'marcus-air', lastPush: 3 }, { nohost: 1 }, { host: 'old' }],
        },
        {
          account: 'acc_9b02d3e6c1f8',
          name: 'Marcus (Pro)',
          dir: '/x/Default',
          exists: true,
          sharedWith: 'default',
          keyMissing: 'yes',
          machines: [],
        },
        { account: 'acc_5f1c9e2b7a40', name: 'Work', dir: '/x/Work', keyMissing: true },
        { name: 'No account' },
      ],
      passphraseChangeUnfinished: true,
      suggestions: [{ label: 'iCloud Drive', path: '/i' }, { label: 'broken' }],
      jobs: [{ id: 'j', kind: 'backup', state: 'running', startedAt: 1 }, { id: 'bad' }],
    });
    expect(v?.settings).toEqual({
      folder: '/Users/me/iCloud/Mother Claude',
      everyHours: 6,
      pullOnLaunch: false,
    });
    expect(v?.sets.map((x) => x.account)).toEqual([
      'default',
      'acc_9b02d3e6c1f8',
      'acc_5f1c9e2b7a40',
    ]);
    // Machines by their id when they have one (an older set's have none).
    expect(v?.sets[0].machines).toEqual([
      { id: 'm1', host: 'marcus-air', lastPush: 3 },
      { host: 'old' },
    ]);
    expect(v?.sets.map((x) => [x.sharedWith, x.keyMissing])).toEqual([
      [undefined, false],
      ['default', false],
      [undefined, true],
    ]);
    expect(ownSets(v!.sets).map((x) => x.account)).toEqual(['default', 'acc_5f1c9e2b7a40']);
    expect(v?.passphraseChangeUnfinished).toBeTrue();
    expect(normalizeBackupView({ settings: {} })?.passphraseChangeUnfinished).toBeFalse();
    expect(v?.suggestions).toEqual([{ label: 'iCloud Drive', path: '/i' }]);
    expect(v?.jobs.map((j) => j.id)).toEqual(['j']);
    // No folder: null, not an empty string.
    expect(normalizeBackupView({ settings: { folder: '' } })?.settings.folder).toBeNull();
    expect(normalizeBackupView({})).toBeNull();
  });

  it("reads a set's index, dropping what it can't use", () => {
    const ix = normalizeBackupIndex({
      account: 'default',
      name: 'Default',
      sessions: [
        {
          id: 'a',
          action: 'new',
          title: '  Refactor auth ',
          remoteBytes: 10,
          host: 'air',
          pushedAt: 2,
          newestMtime: 1,
        },
        { id: 'c', action: 'same', title: '   ' },
        { id: 'b', action: 'teleported' },
        { action: 'new' },
      ],
      config: [
        {
          name: 'config/2026-10-21T08-00-00Z-3f9a1c2b7d4e.tar.gz.age',
          createdAt: 4,
          host: 'air',
          machine: 'm1',
          fingerprint: 'x',
          archiveSha256: 'ab12',
          archiveBytes: 48213,
        },
        { name: 'config/old.tar.gz.age', createdAt: 1, host: 'mbp' },
      ],
    });
    expect(ix?.sessions.map((x) => [x.id, x.title])).toEqual([
      ['a', 'Refactor auth'],
      ['c', undefined],
    ]);
    expect(ix?.config).toEqual([
      {
        name: 'config/2026-10-21T08-00-00Z-3f9a1c2b7d4e.tar.gz.age',
        createdAt: 4,
        host: 'air',
        machine: 'm1',
        archiveSha256: 'ab12',
        archiveBytes: 48213,
      },
      {
        name: 'config/old.tar.gz.age',
        createdAt: 1,
        host: 'mbp',
        machine: '',
        archiveSha256: '',
        archiveBytes: 0,
      },
    ]);
    expect(normalizeBackupIndex({ account: 'default' })).toBeNull();
  });

  it('reads the index of a storage set no account is named after', () => {
    const ix = normalizeBackupIndex({
      set: 'Shared',
      sessions: [
        { id: 'a', action: 'new', archived: { at: 5, machine: 'm1' } },
        { id: 'b', action: 'new', archived: false },
      ],
    });
    expect(ix?.account).toBe('');
    expect(ix?.name).toBe('Shared');
    expect(ix?.sessions.map((x) => [x.id, x.archived])).toEqual([
      ['a', true],
      ['b', undefined],
    ]);
    expect(normalizeBackupIndex({ sessions: [] })).toBeNull();
  });

  it('preselects new sessions for a restore, and all a pull brings for a pull', () => {
    const list = [
      s('n', 'new'),
      s('u', 'update'),
      s('c', 'conflict'),
      s('same', 'same'),
      s('l', 'live'),
      s('ln', 'localNewer'),
    ];
    expect([...preselect(list, 'restore')]).toEqual(['n']);
    expect([...preselect(list, 'pull')]).toEqual(['n', 'u', 'c']);
  });

  it('finds the selected sessions Claude Code would prune again', () => {
    const list = [s('a', 'new', 'old'), s('b', 'new', 'old'), s('c', 'new')];
    expect(prunedOf(list, new Set(['a', 'c'])).map((x) => x.id)).toEqual(['a']);
    expect(prunedOf(list, new Set(['c']))).toEqual([]);
  });

  it('says what a pull does with each session', () => {
    expect(actionText('new', 'pull')).toBe('New here');
    expect(actionText('update', 'pull')).toBe(
      "Newer in the backup — replaces this Mac's copy, kept for undo",
    );
    expect(actionText('conflict', 'pull')).toBe('Changed on both — kept side by side');
    expect(actionText('live', 'pull')).toBe('Running here — skipped');
    expect(actionText('localNewer', 'pull')).toBe('Newer here');
    expect(scheduleLabel(0)).toBe('By hand only');
    expect(scheduleLabel(6)).toBe('Every 6 hours');
    expect(scheduleLabel(48)).toBe('Every 48 hours');
  });

  it('sums up finished jobs in one line', () => {
    expect(
      jobSummary(
        done('backup', {
          archived: ['a', 'b'],
          unchanged: 40,
          skippedLive: ['c'],
          failed: [],
          config: 'config/x',
          bytes: 1,
        }),
      ),
    ).toBe('2 sessions backed up · 40 unchanged · 1 running, skipped · settings saved');
    expect(
      jobSummary(
        done('backup', {
          archived: [],
          unchanged: 12,
          skippedLive: [],
          failed: [],
          conflicts: ['d'],
          newerInSet: ['e', 'f'],
          warnings: ['agents/x.md could not be read'],
          removed: 3,
          bytes: 0,
        }),
      ),
    ).toBe(
      'Nothing new to back up · 12 unchanged · 1 conflict · 2 newer in the backup · 1 warning',
    );
    expect(
      jobSummary(done('restore', { restored: ['a'], skipped: [{ id: 'b', code: 'x' }] })),
    ).toBe('1 session restored · 1 skipped');
    expect(
      jobSummary(
        done('pull', {
          restored: ['a'],
          updated: ['b', 'c'],
          conflicts: [['d', '/x/d.conflict.jsonl']],
          historyLines: 12,
        }),
      ),
    ).toBe('1 new · 2 updated · 1 conflict · 12 prompt-history lines added');
    expect(jobSummary(done('pull', {}))).toBe('Already up to date.');
    expect(jobSummary({ ...done('copy', null), sessions: ['a'] })).toBe('Copied 1 session.');
    expect(jobSummary({ ...done('move', null), state: 'failed', error: 'disk full' })).toBe(
      'Disk full.',
    );
  });

  it("reads a pull's conflicts as pairs of session and file", () => {
    const r = normalizePullResult({
      conflicts: [['s1', '/p/s1.conflict-air-1.jsonl'], ['bad'], 'x'],
      warnings: [{ id: '', text: 'Prompt history was not merged' }, { id: 'x' }],
    });
    expect(r.conflicts).toEqual([['s1', '/p/s1.conflict-air-1.jsonl']]);
    expect(r.warnings).toEqual([{ id: '', text: 'Prompt history was not merged' }]);
    expect(r.restored).toEqual([]);
  });

  it("reads a backup's conflicts, newer sessions, warnings and removed files", () => {
    expect(
      normalizePushResult({
        archived: ['a'],
        conflicts: ['d', 3, ''],
        newerInSet: ['e'],
        warnings: ['left out: x', ''],
        removed: 2,
      }),
    ).toEqual({
      archived: ['a'],
      unchanged: 0,
      skippedLive: [],
      failed: [],
      conflicts: ['d'],
      newerInSet: ['e'],
      config: null,
      bytes: 0,
      warnings: ['left out: x'],
      removed: 2,
    });
    // An older server's result has none of them.
    expect(normalizePushResult({ archived: [] })).toEqual(
      jasmine.objectContaining({ conflicts: [], newerInSet: [], warnings: [], removed: 0 }),
    );
  });

  it("reads a passphrase change's answer and what a refused one left behind", () => {
    expect(
      normalizePassphraseChange({
        passphrase: 'AB-CD',
        rekeyed: ['Default', 2],
        left: ['Personal'],
      }),
    ).toEqual({ passphrase: 'AB-CD', rekeyed: ['Default'], left: ['Personal'] });
    expect(normalizePassphraseChange({ passphrase: null })).toEqual({
      passphrase: null,
      rekeyed: [],
      left: [],
    });
    const err = (extra: Record<string, unknown>) =>
      new ApiError(
        'the Keychain refused it',
        500,
        'http',
        'backup_io',
        undefined,
        undefined,
        extra,
      );
    expect(passphraseFailure(err({ applied: true, passphrase: 'AB-CD' }))).toEqual({
      applied: true,
      passphrase: 'AB-CD',
    });
    expect(passphraseFailure(err({ applied: true }))).toEqual({ applied: true, passphrase: '' });
    expect(passphraseFailure(err({ applied: 'yes' }))).toEqual({ applied: false, passphrase: '' });
    expect(passphraseFailure(new Error('x'))).toEqual({ applied: false, passphrase: '' });
  });

  it('reads the sets a pull passed over, as objects or as sentences', () => {
    expect(
      normalizePullSkips([
        { account: 'acc_w', code: 'set_key_missing', message: 'no key' },
        'an older answer',
        { account: 'acc_x' },
        '  ',
        7,
      ]),
    ).toEqual([
      { account: 'acc_w', code: 'set_key_missing', message: 'no key' },
      { account: '', code: '', message: 'an older answer' },
    ]);
    expect(normalizePullSkips(undefined)).toEqual([]);
  });

  it('tells the other machines apart from this one as far as the view can', () => {
    const m = (id: string, host: string) => ({ id, host });
    expect(otherMachines([m('a', 'mbp'), m('b', 'air')], 'mbp')).toEqual({
      others: [m('b', 'air')],
      sameName: false,
    });
    // Two entries with this Mac's name: another Mac has it too.
    expect(otherMachines([m('a', 'mbp'), m('c', 'mbp')], 'mbp').sameName).toBeTrue();
    // With this Mac's id, the other Mac of the same name is listed itself.
    expect(otherMachines([m('a', 'mbp'), m('c', 'mbp'), m('b', 'air')], 'mbp', 'a')).toEqual({
      others: [m('c', 'mbp'), m('b', 'air')],
      sameName: false,
    });
    expect(andList([])).toBe('');
    expect(andList(['Default'])).toBe('Default');
    expect(andList(['Default', 'Work', 'Personal'])).toBe('Default, Work and Personal');
  });

  it('says what each refusal means', () => {
    const e = (status: number, code: string, message = '') =>
      new ApiError(message, status, 'http', code);
    expect(backupErrorText(e(409, 'no_backup_folder', 'no backup folder is set'))).toBe(
      'Choose a backup folder first.',
    );
    expect(backupErrorText(e(409, 'no_backup_passphrase'))).toBe('Set a passphrase first.');
    expect(backupErrorText(e(403, 'wrong_passphrase'))).toContain('does not open this backup');
    expect(backupErrorText(e(422, 'invalid_backup', 'there is no backup of Work in /x'))).toBe(
      'There is no backup of Work in /x.',
    );
    expect(backupErrorText(e(409, 'session_live', 'session a is running; stop it first'))).toBe(
      'Session a is running; stop it first.',
    );
    // A set that fails its check, or whose key isn't here: our words, not
    // the server's paths and file formats.
    expect(
      backupErrorText(
        e(409, 'backup_unauthenticated', 'the index of the backup set in /x is not authentic'),
      ),
    ).toBe(
      "This backup failed its check: its index was not written with this passphrase's key — someone changed it, or an older build of Mother Claude made it. Nothing was read from it.",
    );
    expect(backupErrorText(e(409, 'set_key_missing', 'the backup set in /x has no key'))).toContain(
      'choose Keep Downloaded for the Mother Claude folder',
    );
    // The same about a job that ended with them.
    expect(
      jobSummary({
        ...done('backup', null),
        state: 'failed',
        error: 'the backup set in /x has no key',
        errorCode: 'set_key_missing',
      }),
    ).toContain("The backup's key (identity.age) isn't in the folder on this Mac");
  });
});
