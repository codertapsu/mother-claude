import { ApiError } from './api.service';
import {
  actionText,
  backupErrorText,
  jobSummary,
  normalizeBackupIndex,
  normalizeBackupView,
  normalizePullResult,
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
          machines: [{ host: 'marcus-air', lastPush: 3 }, { nohost: 1 }],
        },
        { name: 'No account' },
      ],
      suggestions: [{ label: 'iCloud Drive', path: '/i' }, { label: 'broken' }],
      jobs: [{ id: 'j', kind: 'backup', state: 'running', startedAt: 1 }, { id: 'bad' }],
    });
    expect(v?.settings).toEqual({
      folder: '/Users/me/iCloud/Mother Claude',
      everyHours: 6,
      pullOnLaunch: false,
    });
    expect(v?.sets.map((x) => x.account)).toEqual(['default']);
    expect(v?.sets[0].machines).toEqual([{ host: 'marcus-air', lastPush: 3 }]);
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
        { id: 'a', action: 'new', remoteBytes: 10, host: 'air', pushedAt: 2, newestMtime: 1 },
        { id: 'b', action: 'teleported' },
        { action: 'new' },
      ],
    });
    expect(ix?.sessions.map((x) => x.id)).toEqual(['a']);
    expect(ix?.config).toEqual([]);
    expect(normalizeBackupIndex({ account: 'default' })).toBeNull();
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
  });
});
