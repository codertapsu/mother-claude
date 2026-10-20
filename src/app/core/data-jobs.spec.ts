import { ApiError } from './api.service';
import {
  canUndo,
  formatBytes,
  jobFraction,
  jobIdOf,
  jobTitle,
  normalizeJob,
  normalizePlan,
  sentence,
  transferErrorText,
} from './data-jobs';
import { JobView } from './models';

const job = (over: Partial<JobView> = {}): JobView => ({
  id: 'job1',
  kind: 'copy',
  state: 'running',
  startedAt: 1,
  filesDone: 0,
  filesTotal: 0,
  bytesDone: 0,
  bytesTotal: 0,
  sessions: [],
  ...over,
});

describe('data job helpers', () => {
  it('reads a job tolerantly', () => {
    expect(
      normalizeJob({
        id: 'j',
        kind: 'pull',
        state: 'done',
        startedAt: 5,
        finishedAt: 9,
        filesDone: 3,
        filesTotal: -1,
        bytesDone: 'x',
        undoUntil: 99,
        sessions: ['a', 2, 'b'],
        result: { restored: [] },
        extra: true,
      }),
    ).toEqual({
      id: 'j',
      kind: 'pull',
      state: 'done',
      startedAt: 5,
      finishedAt: 9,
      filesDone: 3,
      filesTotal: 0,
      bytesDone: 0,
      bytesTotal: 0,
      undoUntil: 99,
      sessions: ['a', 'b'],
      result: { restored: [] },
    });
    expect(normalizeJob({ id: 'j', kind: 'explode', state: 'done' })).toBeNull();
    expect(normalizeJob({ kind: 'copy', state: 'done' })).toBeNull();
    expect(normalizeJob(null)).toBeNull();
    // Routes answer a job or just its id.
    expect(jobIdOf('job_9 ')).toBe('job_9');
    expect(jobIdOf(job({ id: 'job_7' }))).toBe('job_7');
    expect(jobIdOf(42)).toBe('');
  });

  it('offers undo only for a finished job whose undo has not run out', () => {
    expect(canUndo(job({ state: 'done', undoUntil: 100 }), 50)).toBeTrue();
    expect(canUndo(job({ state: 'done', undoUntil: 100 }), 100)).toBeFalse();
    expect(canUndo(job({ state: 'done' }), 50)).toBeFalse();
    expect(canUndo(job({ state: 'undone', undoUntil: 100 }), 50)).toBeFalse();
    // A failed transfer that could not put everything back keeps its undo.
    expect(
      canUndo(job({ state: 'failed', errorCode: 'transfer_incomplete', undoUntil: 100 }), 50),
    ).toBeTrue();
    expect(canUndo(job({ state: 'failed' }), 50)).toBeFalse();
    expect(canUndo(job({ state: 'running', undoUntil: 100 }), 50)).toBeFalse();
    expect(canUndo(null, 0)).toBeFalse();
  });

  it('measures progress by bytes, else files, else not at all', () => {
    expect(jobFraction(job({ bytesDone: 50, bytesTotal: 200, filesDone: 9, filesTotal: 10 }))).toBe(
      0.25,
    );
    expect(jobFraction(job({ filesDone: 1, filesTotal: 4 }))).toBe(0.25);
    expect(jobFraction(job())).toBeNull();
    expect(jobFraction(job({ state: 'done' }))).toBe(1);
  });

  it('writes sizes for people', () => {
    expect(formatBytes(0)).toBe('0 B');
    expect(formatBytes(512)).toBe('512 B');
    expect(formatBytes(1536)).toBe('1.5 KB');
    expect(formatBytes(3_400_000)).toBe('3.2 MB');
    expect(formatBytes(48 * 1024 * 1024)).toBe('48 MB');
  });

  it("puts a transfer's refusals in words: the server's, as a sentence", () => {
    const refusal = (code: string, message: string) => new ApiError(message, 409, 'http', code);
    expect(
      transferErrorText(refusal('session_live', 'session 9b2 is running; stop it first')),
    ).toBe('Session 9b2 is running; stop it first.');
    expect(transferErrorText(refusal('undo_conflict', ''))).toContain(
      'the copied conversation was continued',
    );
    expect(transferErrorText(refusal('directory_in_use', ''))).toContain('quit its sessions first');
    // Anything else goes through the app's one error sentence.
    expect(transferErrorText(new ApiError('nope', 422, 'http', 'other'))).toBe('nope');
    expect(sentence('  done already!  ')).toBe('Done already!');
  });

  it('reads a dry run tolerantly', () => {
    const p = normalizePlan({
      what: { what: 'project', cwd: '/Users/me/dev/api' },
      mode: 'move',
      sessions: [
        { id: 's1', cwd: '/Users/me/dev/api', files: [{ from: '/a', to: '/b', size: 10 }, {}] },
        { cwd: 'no id' },
      ],
      files: 1,
      bytes: 10,
      addsProjectEntry: '/Users/me/dev/api',
      historyLines: 4,
      writesShared: true,
    });
    expect(p).toEqual({
      what: { what: 'project', cwd: '/Users/me/dev/api' },
      mode: 'move',
      sessions: [
        { id: 's1', cwd: '/Users/me/dev/api', files: [{ from: '/a', to: '/b', size: 10 }] },
      ],
      files: 1,
      bytes: 10,
      addsProjectEntry: '/Users/me/dev/api',
      removesProjectEntry: null,
      historyLines: 4,
      writesShared: true,
    });
    expect(normalizePlan({ what: { what: 'session' }, mode: 'copy' })).toBeNull();
    expect(normalizePlan({ what: { what: 'session', id: 'x' }, mode: 'teleport' })).toBeNull();
  });

  it('names the accounts a job moved between, when they are known', () => {
    const names: Record<string, string> = { default: 'Default', acc_w: 'Work', acc_p: 'Personal' };
    const nameOf = (id: string) => names[id];
    const job = (extra: Record<string, unknown>) =>
      normalizeJob({ id: 'j', kind: 'move', state: 'done', ...extra })!;
    expect(job({ from: 'default', to: 'acc_w' }).from).toBe('default');
    expect(jobTitle(job({ from: 'default', to: 'acc_w' }), nameOf)).toBe(
      'Move from Default to Work',
    );
    expect(jobTitle(job({ to: 'acc_w' }), nameOf)).toBe('Move to Work');
    expect(jobTitle(job({ from: 'gone', to: 'acc_gone' }), nameOf)).toBe('Move');
    expect(jobTitle(job({ kind: 'backup', account: 'acc_w' }), nameOf)).toBe('Backup of Work');
    expect(jobTitle(job({ kind: 'restore', account: 'acc_w', to: 'acc_p' }), nameOf)).toBe(
      'Restore of Work into Personal',
    );
    expect(jobTitle(job({ kind: 'restore', account: 'acc_w', to: 'acc_w' }), nameOf)).toBe(
      'Restore of Work',
    );
    expect(jobTitle(job({ kind: 'pull' }), nameOf)).toBe('Pull');
  });
});
