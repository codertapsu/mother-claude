import { ApiError } from './api.service';
import { jobTitle, normalizeJob } from './data-jobs';
import {
  DELETE_DANGEROUS_REASON,
  SKIP_OPEN_IN_API,
  SKIP_RUNNING,
  counted,
  deleteBody,
  deleteButtonLabel,
  deleteErrorText,
  deletePlanLine,
  deleteResultSummary,
  deleteSkipReason,
  desktopOs,
  mergeDeletePlans,
  normalizeDataProjects,
  normalizeDeletePlan,
  normalizeSessionFiles,
  parentFolder,
  projectGroups,
  projectKey,
  projectMatches,
  revealLabel,
  sessionGroups,
} from './delete-data';
import { JobView } from './models';

const job = (over: Partial<JobView>): JobView => ({
  id: 'j',
  kind: 'delete',
  state: 'done',
  startedAt: 1,
  filesDone: 0,
  filesTotal: 0,
  bytesDone: 0,
  bytesTotal: 0,
  sessions: [],
  ...over,
});

describe('delete data helpers', () => {
  it("reads where a conversation's files are", () => {
    expect(
      normalizeSessionFiles({
        transcript: '/Users/me/.claude/projects/-Users-me/e5ec.jsonl',
        folder: '/Users/me/.claude/projects/-Users-me/e5ec',
        fileHistory: '',
        account: { account: 'acc_w' },
      }),
    ).toEqual({
      transcript: '/Users/me/.claude/projects/-Users-me/e5ec.jsonl',
      folder: '/Users/me/.claude/projects/-Users-me/e5ec',
      account: 'acc_w',
    });
    expect(normalizeSessionFiles({ folder: '/x' })).toBeNull();
    expect(normalizeSessionFiles(undefined)).toBeNull();
    expect(parentFolder('/Users/me/.claude/projects/-Users-me/e5ec.jsonl')).toBe(
      '/Users/me/.claude/projects/-Users-me',
    );
    expect(parentFolder('C:\\Users\\me\\.claude\\projects\\x\\a.jsonl')).toBe(
      'C:\\Users\\me\\.claude\\projects\\x',
    );
  });

  it("names the reveal after the desktop's file manager", () => {
    const mac = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15';
    const win = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Edg/131.0';
    const linux = 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15';
    expect(revealLabel(desktopOs(mac))).toBe('Show in Finder');
    expect(revealLabel(desktopOs(win))).toBe('Show in File Explorer');
    expect(revealLabel(desktopOs(linux))).toBe('Show in folder');
  });

  it('reads every project on disk, newest first', () => {
    const list = normalizeDataProjects({
      projects: [
        {
          account: 'default',
          cwd: '/Users/me/dev/api',
          name: 'api',
          folder: '/Users/me/.claude/projects/-Users-me-dev-api',
          sessions: 12,
          live: 1,
          bytes: 4096,
          lastActivityAt: 10,
        },
        {
          account: { account: 'acc_w' },
          cwd: '/Users/me/dev/web/',
          sessions: 2,
          lastActivityAt: 20,
        },
        { account: 'default', cwd: '/Users/me/dev/api', sessions: 1 },
        { cwd: '/no/account' },
        { account: 'default' },
      ],
    });
    expect(list.map((p) => [p.account, p.cwd, p.name, p.sessions])).toEqual([
      ['acc_w', '/Users/me/dev/web/', 'web', 2],
      ['default', '/Users/me/dev/api', 'api', 12],
    ]);
    expect(list[1]).toEqual({
      account: 'default',
      cwd: '/Users/me/dev/api',
      name: 'api',
      folder: '/Users/me/.claude/projects/-Users-me-dev-api',
      sessions: 12,
      live: 1,
      bytes: 4096,
      lastActivityAt: 10,
    });
    expect(projectKey(list[1])).toBe('default\u0000/Users/me/dev/api');
    expect(projectMatches(list[1], 'dev/ap')).toBeTrue();
    expect(projectMatches(list[1], 'web')).toBeFalse();
    expect(normalizeDataProjects(null)).toEqual([]);
  });

  it('builds one request per account', () => {
    expect(
      sessionGroups([
        { id: 'a', account: 'default' },
        { id: 'b', account: 'acc_w' },
        { id: 'c', account: 'default' },
        { id: 'd' },
      ]),
    ).toEqual([
      { from: 'default', sessions: ['a', 'c'] },
      { from: 'acc_w', sessions: ['b'] },
      { sessions: ['d'] },
    ]);
    expect(
      projectGroups([
        { account: 'default', cwd: '/p' },
        { account: 'acc_w', cwd: '/p' },
        { account: 'default', cwd: '/q' },
      ]),
    ).toEqual([
      { from: 'default', projects: ['/p', '/q'] },
      { from: 'acc_w', projects: ['/p'] },
    ]);
    expect(deleteBody({ from: 'acc_w', sessions: ['a'] }, true)).toEqual({
      from: 'acc_w',
      items: { sessions: ['a'] },
      undo: true,
    });
    expect(deleteBody({ projects: ['/p'] }, false)).toEqual({
      items: { projects: ['/p'] },
      undo: false,
    });
  });

  it('reads a dry run tolerantly and adds several up', () => {
    const a = normalizeDeletePlan({
      sessions: [
        {
          id: 's1',
          title: ' Fix it ',
          cwd: '/p',
          account: { account: 'default' },
          files: 3,
          bytes: 300,
        },
        { id: 's2', files: [{}, {}], bytes: 20, account: 'default' },
        { title: 'no id' },
      ],
      skipped: [
        {
          id: 's3',
          title: 'Ship it',
          code: 'session_live',
          message: 'session s3 is running; stop it first',
        },
        'something else',
        {},
      ],
      projects: ['/p'],
      folders: [],
      sessionsTotal: 1240,
      skippedTotal: 12,
      projectsTotal: 1,
      foldersTotal: 0,
      files: 5,
      bytes: 320,
      jobs: 1,
      truncated: true,
    });
    expect(a).toEqual({
      sessions: [
        { id: 's1', title: 'Fix it', cwd: '/p', account: 'default', files: 3, bytes: 300 },
        { id: 's2', account: 'default', files: 2, bytes: 20 },
      ],
      skipped: [
        {
          id: 's3',
          title: 'Ship it',
          code: 'session_live',
          message: 'session s3 is running; stop it first',
        },
        { code: '', message: 'something else' },
      ],
      sessionsTotal: 1240,
      skippedTotal: 12,
      projectsTotal: 1,
      foldersTotal: 0,
      files: 5,
      bytes: 320,
      jobs: 1,
      truncated: true,
    });
    // An older plan without totals: its lists are the totals.
    const b = normalizeDeletePlan({
      sessions: [{ id: 'w1', files: 1, bytes: 10 }],
      skipped: [],
      folders: ['/x'],
    })!;
    expect([b.files, b.jobs, b.sessionsTotal, b.skippedTotal, b.foldersTotal]).toEqual([
      1, 1, 1, 0, 1,
    ]);
    const both = mergeDeletePlans([a!, b]);
    expect(both.sessions.map((s) => s.id)).toEqual(['s1', 's2', 'w1']);
    expect([both.files, both.bytes, both.jobs, both.truncated]).toEqual([6, 330, 2, true]);
    expect([both.sessionsTotal, both.skippedTotal, both.projectsTotal, both.foldersTotal]).toEqual([
      1241, 12, 1, 1,
    ]);
    expect(normalizeDeletePlan({ files: 3 })).toBeNull();
  });

  it('says what a dry run takes, and names the count on the button', () => {
    const p = normalizeDeletePlan({
      sessions: [
        { id: 'a', files: 3, bytes: 2048 },
        { id: 'b', files: 2, bytes: 1024 },
      ],
      skipped: [],
    })!;
    expect(deletePlanLine(p)).toBe('Deletes 2 conversations (5 files, 3.0 KB).');
    expect(deleteButtonLabel(p)).toBe('Delete 2 conversations');
    // The totals count everything; the lists only the first 1000.
    const big = {
      ...p,
      sessionsTotal: 1240,
      projectsTotal: 2,
      foldersTotal: 2,
      files: 3720,
      truncated: true,
    };
    expect(deletePlanLine(big)).toBe(
      'Deletes 1,240 conversations of 2 projects (3,720 files, 3.0 KB). The 2 project folders left empty go too. The lists show the first 1,000.',
    );
    expect(deleteButtonLabel(big)).toBe('Delete 1,240 conversations');
    expect(deletePlanLine({ ...p, projectsTotal: 1, foldersTotal: 1 })).toBe(
      'Deletes 2 conversations of 1 project (5 files, 3.0 KB). The project folder left empty goes too.',
    );
    const none = normalizeDeletePlan({ sessions: [], skipped: [{ code: 'not_found' }] })!;
    expect(deletePlanLine(none)).toBe('There is nothing to delete here.');
    expect(deleteButtonLabel(none)).toBe('Nothing to delete');
    expect(counted(1, 'file')).toBe('1 file');
  });

  it('says why something is skipped', () => {
    // The server names a running one by its id: the row has its title.
    const live = {
      id: 's3',
      code: 'session_live',
      message: 'session s3 is running; stop it first',
    };
    expect(deleteSkipReason(live)).toBe(SKIP_RUNNING);
    expect(deleteSkipReason(live, new Set(['s3']))).toBe(SKIP_OPEN_IN_API);
    expect(
      deleteSkipReason({
        id: 's4',
        code: 'not_found',
        message: "it is Work's conversation; delete it from there",
      }),
    ).toBe("It is Work's conversation; delete it from there.");
    expect(deleteSkipReason({ code: 'transfer_io', message: '' })).toContain('put back whole');
    expect(deleteSkipReason({ code: 'x', message: '' })).toBe('Skipped.');
  });

  it('sums up a delete job and titles it after its directory', () => {
    expect(
      deleteResultSummary(
        job({ result: { deleted: ['a', 'b'], skipped: [{ id: 'c', code: 'session_live' }] } }),
      ),
    ).toBe('Deleted 2 conversations · 1 skipped');
    expect(deleteResultSummary(job({ result: { deleted: [], skipped: [] } }))).toBe(
      'Nothing was deleted',
    );
    expect(
      deleteResultSummary(
        job({
          result: {
            deleted: ['a'],
            skipped: [],
            warnings: ['Some deleted files could not be removed from /u/j (busy); they go later.'],
          },
        }),
      ),
    ).toBe(
      'Deleted 1 conversation. Some deleted files could not be removed from /u/j (busy); they go later.',
    );
    expect(deleteResultSummary(job({ kind: 'move' }))).toBeNull();
    const view = normalizeJob({ id: 'j', kind: 'delete', state: 'done', account: 'acc_w' })!;
    expect(view.kind).toBe('delete');
    expect(jobTitle(view, (id) => (id === 'acc_w' ? 'Work' : undefined))).toBe('Delete from Work');
    expect(jobTitle({ ...view, account: undefined }, () => undefined)).toBe('Delete');
  });

  it('puts the refusals in words', () => {
    expect(deleteErrorText(new ApiError('no', 403, 'http', 'dangerous_blocked'))).toBe(
      DELETE_DANGEROUS_REASON,
    );
    expect(deleteErrorText(new ApiError('', 404, 'http', 'transcript_not_found'))).toContain(
      "isn't on this Mac",
    );
    expect(deleteErrorText(new ApiError('busy', 409, 'http', 'data_job_running'))).toBe('Busy.');
    expect(deleteErrorText(new ApiError('', 404, 'http', 'project_not_found'))).toContain(
      'no conversations of this folder',
    );
    expect(deleteErrorText(new ApiError('', 500, 'http', 'reveal_failed'))).toBe(
      "The Mac's file manager did not open.",
    );
  });
});
