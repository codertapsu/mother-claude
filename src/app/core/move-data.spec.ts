import { ApiError } from './api.service';
import { JobView } from './models';
import {
  AccountLike,
  StorageView,
  accountLocation,
  defaultTarget,
  filterRows,
  indexRow,
  locationFromKey,
  locationGroups,
  locationHint,
  locationKey,
  locationName,
  modeHint,
  modeProblem,
  moveBody,
  moveDirection,
  moveErrorText,
  moveResultSummary,
  newSetKey,
  newSetStorage,
  normalizeMovePlan,
  normalizeStorages,
  planLine,
  projectRows,
  readLocation,
  sameLocation,
  sessionRow,
  setLocation,
  setNameProblem,
  skipName,
  skipReason,
  storagePathProblem,
  trustApplies,
  whatProblem,
  wireLocation,
} from './move-data';

const DEFAULT: AccountLike = {
  id: 'default',
  name: 'Default',
  kind: 'directory',
  isDefault: true,
  dir: '/Users/me/.claude',
};
const WORK: AccountLike = {
  id: 'acc_w',
  name: 'Work',
  kind: 'directory',
  dir: '/Users/me/.claude-work',
};
const PRO: AccountLike = { id: 'acc_p', name: 'Pro', kind: 'token' };
const ACCOUNTS = [DEFAULT, WORK, PRO];

const ICLOUD: StorageView = {
  id: 'backup',
  label: 'iCloud Drive',
  path: '/Users/me/Library/Mobile Documents/com~apple~CloudDocs/Mother Claude',
  kind: 'icloud',
  primary: true,
  available: true,
  sets: [
    { name: 'Default', account: 'default', exists: true, keyMissing: false, sessions: 12 },
    { name: 'Work', account: 'acc_w', exists: false, keyMissing: true },
  ],
};
const GDRIVE: StorageView = {
  id: 'stor_0123456789ab',
  label: 'Google Drive',
  path: '/Users/me/Library/CloudStorage/GoogleDrive-me/My Drive/MC',
  kind: 'gdrive',
  primary: false,
  available: true,
  sets: [{ name: 'Shared', exists: true, keyMissing: false, sessions: 3, archived: 1 }],
};
const STORAGES = [ICLOUD, GDRIVE];

const job = (over: Partial<JobView>): JobView => ({
  id: 'j',
  kind: 'move',
  state: 'done',
  startedAt: 1,
  filesDone: 0,
  filesTotal: 0,
  bytesDone: 0,
  bytesTotal: 0,
  sessions: [],
  ...over,
});

describe('move data helpers', () => {
  it('reads the storages, the backup folder first', () => {
    const list = normalizeStorages({
      storage: [
        {
          id: 'stor_0123456789ab',
          label: ' Google Drive ',
          path: '/x/GoogleDrive-me/MC',
          kind: 'teleport',
          available: false,
          sets: [
            { name: 'Shared', exists: true, sessions: 3, archived: 1 },
            { name: 'Shared', exists: false },
            { exists: true },
          ],
        },
        { id: 'backup', label: 'iCloud Drive', path: '/y', kind: 'icloud', primary: true },
        { id: 'backup', path: '/z' },
        { label: 'no id', path: '/w' },
      ],
    });
    expect(list.map((s) => s.id)).toEqual(['backup', 'stor_0123456789ab']);
    expect(list[1]).toEqual({
      id: 'stor_0123456789ab',
      label: 'Google Drive',
      path: '/x/GoogleDrive-me/MC',
      kind: 'gdrive',
      primary: false,
      available: false,
      sets: [{ name: 'Shared', exists: true, keyMissing: false, sessions: 3, archived: 1 }],
    });
    expect(list[0].available).toBeTrue();
  });

  it('makes the backup folder the only storage when the server lists none', () => {
    // Goal 7: only the Default and the backup folder answer as before.
    const list = normalizeStorages({
      settings: { folder: '/Users/me/Library/Mobile Documents/com~apple~CloudDocs/MC' },
      sets: [
        { account: 'default', name: 'Default', exists: true, sessions: 4 },
        { account: 'acc_p', name: 'Pro', exists: true, sharedWith: 'default' },
        { account: 'acc_w', name: 'Work', exists: false, keyMissing: true },
      ],
    });
    expect(list).toEqual([
      {
        id: 'backup',
        label: 'Backup folder',
        path: '/Users/me/Library/Mobile Documents/com~apple~CloudDocs/MC',
        kind: 'icloud',
        primary: true,
        available: true,
        sets: [
          { name: 'Default', account: 'default', exists: true, keyMissing: false, sessions: 4 },
          { name: 'Work', account: 'acc_w', exists: false, keyMissing: true },
        ],
      },
    ]);
    expect(normalizeStorages({ settings: { folder: null }, sets: [] })).toEqual([]);
    expect(normalizeStorages(null)).toEqual([]);
  });

  it('reads and writes locations in every wire form', () => {
    expect(readLocation('acc_w')).toEqual(accountLocation('acc_w'));
    expect(readLocation({ account: 'acc_w' })).toEqual(accountLocation('acc_w'));
    expect(readLocation({ storage: 'backup', set: 'Work' })).toEqual(setLocation('backup', 'Work'));
    expect(readLocation({ storage: 'backup' })).toEqual(setLocation('backup', ''));
    expect(readLocation({ set: 'x' })).toBeNull();
    expect(readLocation('')).toBeNull();
    expect(wireLocation(accountLocation('acc_w'))).toBe('acc_w');
    expect(wireLocation(setLocation('stor_1', 'Shared'))).toEqual({
      storage: 'stor_1',
      set: 'Shared',
    });

    const set = setLocation('stor_1', 'A: b');
    expect(locationFromKey(locationKey(set))).toEqual(set);
    expect(locationFromKey(locationKey(accountLocation('default')))).toEqual(
      accountLocation('default'),
    );
    expect(locationKey(null)).toBe('');
    expect(locationFromKey('')).toBeNull();
    expect(newSetStorage(newSetKey('stor_1'))).toBe('stor_1');
    expect(newSetStorage('a:default')).toBe('');
    expect(sameLocation(set, setLocation('stor_1', 'A: b'))).toBeTrue();
    expect(sameLocation(set, accountLocation('stor_1'))).toBeFalse();
    expect(sameLocation(null, null)).toBeTrue();
  });

  it('knows which way data goes', () => {
    const d = (from: ReturnType<typeof readLocation>, to: ReturnType<typeof readLocation>) =>
      moveDirection(from, to, ACCOUNTS);
    expect(d(accountLocation('default'), accountLocation('acc_w'))).toBe('between');
    // A token account uses the Default's directory: nothing moves.
    expect(d(accountLocation('default'), accountLocation('acc_p'))).toBe('assign');
    expect(d(accountLocation('acc_w'), setLocation('backup', 'Work'))).toBe('toStorage');
    expect(d(setLocation('stor_1', 'Shared'), accountLocation('acc_p'))).toBe('fromStorage');
    expect(d(setLocation('stor_1', 'Shared'), setLocation('backup', 'Default'))).toBe('storages');
    expect(d(null, accountLocation('acc_w'))).toBe('mixed');
    expect(d(null, setLocation('backup', 'Default'))).toBe('toStorage');
    expect(d(accountLocation('acc_w'), accountLocation('acc_w'))).toBe('same');
    expect(d(accountLocation('acc_w'), null)).toBe('same');
  });

  it('says which modes and items each way takes', () => {
    expect(modeProblem('copy', 'assign', 'sessions')).toContain('would be the same conversation');
    expect(modeProblem('move', 'assign', 'sessions')).toBe('');
    expect(modeProblem('move', 'between', 'customizations')).toBe(
      'Customizations are only ever copied.',
    );
    expect(modeProblem('copy', 'same', 'all')).toBe('Pick another place for it to go.');
    expect(whatProblem('customizations', 'between')).toBe('');
    expect(whatProblem('customizations', 'toStorage')).toContain('only between accounts');
    expect(whatProblem('customizations', 'assign')).toContain('only between accounts');
    expect(whatProblem('projects', 'mixed', true)).toBe('Choose where they come from first.');
    expect(whatProblem('sessions', 'mixed', true)).toBe('');
    expect(trustApplies('between', 'projects')).toBeTrue();
    expect(trustApplies('fromStorage', 'all')).toBeTrue();
    expect(trustApplies('toStorage', 'sessions')).toBeFalse();
    expect(trustApplies('assign', 'sessions')).toBeFalse();
    expect(trustApplies('between', 'customizations')).toBeFalse();
    expect(modeHint('move', 'toStorage', 'sessions', 'Work', 'Google Drive · Shared')).toContain(
      'in storage only',
    );
    expect(modeHint('move', 'assign', 'sessions', 'Default', 'Pro')).toBe(
      "Nothing moves: it is recorded as Pro's, so it shows, counts and continues under Pro.",
    );
    expect(modeHint('copy', 'between', 'customizations', 'Default', 'Work', true)).toContain(
      'replaces what it has',
    );
  });

  it('builds the request', () => {
    const base = {
      from: accountLocation('acc_w') as ReturnType<typeof accountLocation> | null,
      to: setLocation('stor_1', 'Shared'),
      what: 'sessions' as const,
      sessions: ['s1', 's2'],
      projects: ['/p'],
      customizations: ['skills' as const],
      mode: 'move' as const,
      trust: true,
      replaceExisting: true,
    };
    expect(moveBody(base)).toEqual({
      from: 'acc_w',
      to: { storage: 'stor_1', set: 'Shared' },
      items: { sessions: ['s1', 's2'] },
      mode: 'move',
      trust: true,
      replaceExisting: false,
    });
    expect(moveBody({ ...base, from: null, what: 'sessions' }).from).toBeUndefined();
    expect(moveBody({ ...base, what: 'all' }).items).toEqual({ all: true });
    expect(moveBody({ ...base, what: 'projects' }).items).toEqual({ projects: ['/p'] });
    // Customizations are only copied, and carry no trust.
    expect(moveBody({ ...base, to: accountLocation('default'), what: 'customizations' })).toEqual({
      from: 'acc_w',
      to: 'default',
      items: { customizations: ['skills'] },
      mode: 'copy',
      trust: false,
      replaceExisting: true,
    });
  });

  it('reads a dry run tolerantly', () => {
    const p = normalizeMovePlan({
      from: 'acc_w',
      to: { storage: 'stor_1', set: 'Shared' },
      mode: 'move',
      jobs: 2,
      sessions: [
        { id: 's1', cwd: '/p', title: ' Fix it ', files: 3, bytes: 300, action: 'archive' },
        { id: 's2', files: [{ size: 10 }, { size: 5 }], from: { account: 'default' } },
        { id: 's3', action: 'teleport' },
        { title: 'no id' },
      ],
      skipped: [
        { id: 's4', code: 'session_live', message: 'session s4 is running; stop it first' },
        'something else',
        {},
      ],
      projects: ['/p', 3],
      historyLines: 7,
      customizations: [
        { item: 'skills', name: 'review', action: 'add' },
        { item: 'settings', action: 'skip', reason: 'the target has its own' },
        { item: 'agents', name: 'x', action: 'explode' },
      ],
      warnings: ['settings.json runs commands', ''],
      files: 5,
      bytes: 315,
      writesShared: true,
      truncated: true,
    });
    expect(p).toEqual({
      from: accountLocation('acc_w'),
      to: setLocation('stor_1', 'Shared'),
      mode: 'move',
      jobs: 2,
      sessions: [
        { id: 's1', cwd: '/p', title: 'Fix it', files: 3, bytes: 300, action: 'archive' },
        { id: 's2', files: 2, bytes: 15, from: accountLocation('default'), action: 'move' },
        { id: 's3', files: 0, bytes: 0, action: 'move' },
      ],
      skipped: [
        { id: 's4', code: 'session_live', message: 'session s4 is running; stop it first' },
        { code: '', message: 'something else' },
      ],
      projects: ['/p'],
      addsProjectEntries: [],
      removesProjectEntries: [],
      historyLines: 7,
      customizations: [
        { item: 'skills', name: 'review', action: 'add' },
        { item: 'settings', name: 'settings', action: 'skip', reason: 'the target has its own' },
      ],
      warnings: ['settings.json runs commands'],
      files: 5,
      bytes: 315,
      writesShared: true,
      truncated: true,
    });
    expect(
      normalizeMovePlan({ mode: 'copy', truncated: { sessions: true } })?.truncated,
    ).toBeTrue();
    expect(normalizeMovePlan({ mode: 'copy' })?.jobs).toBe(1);
    expect(normalizeMovePlan({ mode: 'teleport' })).toBeNull();
  });

  it('says what a dry run does in one line', () => {
    const plan = normalizeMovePlan({
      mode: 'move',
      sessions: [
        { id: 'a', action: 'archive' },
        { id: 'b', action: 'archive' },
      ],
      files: 12,
      bytes: 2048,
    })!;
    expect(planLine(plan, 'Google Drive · Shared')).toBe(
      'Archives 2 conversations (12 files, 2.0 KB) into Google Drive · Shared.',
    );
    const mixed = normalizeMovePlan({
      mode: 'move',
      sessions: [{ id: 'a' }, { id: 'b', action: 'assign' }],
      truncated: true,
    })!;
    expect(planLine(mixed, 'Pro')).toBe(
      'Moves 1 conversation, assigns 1 conversation to Pro (the first 1000 are listed).',
    );
    expect(planLine(normalizeMovePlan({ mode: 'copy' })!, 'Work')).toBe(
      'There is nothing to move or copy here.',
    );
    expect(skipReason({ code: 'session_live', message: '' })).toBe('It is running; stop it first.');
    expect(skipReason({ code: 'x', message: 'the target has it' })).toBe('The target has it.');
    // A copy within the shared folder, said our way whatever the server says.
    expect(
      skipReason({ code: 'same_directory', message: 'both accounts use the same directory' }),
    ).toContain('a copy there would be the same conversation');
    expect(skipName({ id: 'abcdef123456', code: '', message: '' }, () => undefined)).toBe(
      'abcdef12',
    );
    expect(skipName({ id: 'a', code: '', message: '' }, () => 'Fix it')).toBe('Fix it');
    expect(skipName({ item: 'mcpServers', code: '', message: '' }, () => undefined)).toBe(
      'MCP servers',
    );
    expect(skipName({ cwd: '/Users/me/dev/x', code: '', message: '' }, () => undefined)).toBe(
      '~/dev/x',
    );
  });

  it('sums up what a move job did', () => {
    expect(
      moveResultSummary(
        job({
          result: {
            archived: ['a', 'b'],
            skipped: [{ id: 'c', code: 'session_live', message: '' }],
            customizations: [
              { item: 'skills', name: 'x', action: 'add' },
              { item: 'settings', name: 'settings.json', action: 'skip' },
            ],
          },
        }),
      ),
    ).toBe('Archived 2 conversations · 1 customization copied · 1 skipped');
    expect(moveResultSummary(job({ result: { assigned: ['a'] } }))).toBe('Assigned 1 conversation');
    expect(moveResultSummary(job({ result: { moved: [] } }))).toBe(
      'Nothing to do: everything was there already.',
    );
    // An older server's result: the caller says it its own way.
    expect(moveResultSummary(job({ result: { files: 3 } }))).toBeNull();
    expect(moveResultSummary(job({}))).toBeNull();
  });

  it('offers the accounts and the storages with their sets', () => {
    const from = locationGroups(ACCOUNTS, STORAGES, 'from');
    expect(from.map((g) => g.label)).toEqual([
      'Accounts',
      'iCloud Drive (backup folder)',
      'Google Drive',
    ]);
    expect(from[0].options.map((o) => o.label)).toEqual([
      'Default — shared folder',
      'Work — own folder',
      'Pro — shared folder',
    ]);
    // A set that does not exist yet can't be a source.
    expect(from[1].options.map((o) => [o.label, o.disabled])).toEqual([['Default', false]]);

    const to = locationGroups(ACCOUNTS, [ICLOUD, { ...GDRIVE, available: false }], 'to');
    expect(to[1].options.map((o) => [o.label, o.disabled])).toEqual([
      ['Default', false],
      ['Work — no key here', true],
      ['New set…', false],
    ]);
    expect(to[2].label).toBe('Google Drive (not available)');
    expect(to[2].options.every((o) => o.disabled)).toBeTrue();
    expect(locationGroups([DEFAULT], [], 'to')[0].options[0].label).toBe('Default');
    // The server calls the backup folder "Backup folder": said once.
    expect(locationGroups([DEFAULT], [{ ...ICLOUD, label: 'Backup folder' }], 'to')[1].label).toBe(
      'Backup folder',
    );
  });

  it('names and describes a location', () => {
    expect(locationName(accountLocation('acc_w'), ACCOUNTS, STORAGES)).toBe('Work');
    expect(locationName(setLocation('stor_0123456789ab', 'Shared'), ACCOUNTS, STORAGES)).toBe(
      'Google Drive · Shared',
    );
    expect(locationName(setLocation('stor_gone', 'X'), ACCOUNTS, STORAGES)).toBe('a storage · X');
    expect(locationName(null, ACCOUNTS, STORAGES)).toBe('where each one is');
    expect(locationHint(accountLocation('default'), ACCOUNTS, STORAGES)).toBe(
      'The shared folder, ~/.claude: Default and Pro use it.',
    );
    expect(locationHint(accountLocation('acc_p'), ACCOUNTS, STORAGES)).toContain(
      'in the shared folder (~/.claude)',
    );
    expect(locationHint(accountLocation('acc_w'), ACCOUNTS, STORAGES)).toBe(
      'Its own folder, ~/.claude-work.',
    );
    expect(locationHint(accountLocation('default'), [DEFAULT], [])).toBe('~/.claude');
    expect(locationHint(setLocation('stor_0123456789ab', 'Shared'), ACCOUNTS, STORAGES)).toBe(
      '~/Library/CloudStorage/GoogleDrive-me/My Drive/MC/Shared · 3 conversations · 1 in storage only',
    );
    expect(locationHint(setLocation('stor_0123456789ab', 'New'), ACCOUNTS, STORAGES)).toBe(
      'A new set: ~/Library/CloudStorage/GoogleDrive-me/My Drive/MC/New.',
    );
  });

  it('picks somewhere for things to go', () => {
    expect(defaultTarget(accountLocation('default'), ACCOUNTS, STORAGES)).toEqual(
      accountLocation('acc_w'),
    );
    expect(defaultTarget(accountLocation('acc_w'), ACCOUNTS, STORAGES)).toEqual(
      accountLocation('default'),
    );
    // Only the shared folder: another account in it (an assign).
    expect(defaultTarget(accountLocation('acc_p'), [DEFAULT, PRO], STORAGES)).toEqual(
      accountLocation('default'),
    );
    // Only the Default: its own set in the backup folder.
    expect(defaultTarget(accountLocation('default'), [DEFAULT], STORAGES)).toEqual(
      setLocation('backup', 'Default'),
    );
    expect(defaultTarget(accountLocation('default'), [DEFAULT], [])).toBeNull();
    // Out of a set: the account it is named after, else the Default.
    expect(defaultTarget(setLocation('backup', 'Work'), ACCOUNTS, STORAGES)).toEqual(
      accountLocation('acc_w'),
    );
    expect(defaultTarget(setLocation('stor_0123456789ab', 'Shared'), ACCOUNTS, STORAGES)).toEqual(
      accountLocation('default'),
    );
  });

  it('checks the names of new sets and storages', () => {
    expect(setNameProblem('  ', GDRIVE)).toBe('Name the new set.');
    expect(setNameProblem('shared', GDRIVE)).toContain('There is a set with this name already');
    expect(setNameProblem('a/b', GDRIVE)).toContain("can't contain");
    expect(setNameProblem('.x', GDRIVE)).toContain('dot');
    expect(setNameProblem('Archive 2026', GDRIVE)).toBe('');
    expect(storagePathProblem('')).toBe("Enter the folder's path.");
    expect(storagePathProblem('Drive/MC')).toContain('full path');
    expect(storagePathProblem('/a/../b')).toContain('without . or ..');
    expect(storagePathProblem('~/Dropbox/MC')).toBe('');
  });

  it('turns conversations and projects into checklist rows', () => {
    const s = sessionRow({
      id: 'abcdef1234',
      title: '',
      name: 'auth',
      cwd: '/Users/me/dev/api',
      projectName: 'api',
      live: true,
    });
    expect(s).toEqual({
      id: 'abcdef1234',
      title: 'auth',
      sub: 'abcdef12 · ~/dev/api',
      tag: 'Running',
      haystack: 'auth /users/me/dev/api abcdef1234',
    });
    const i = indexRow({
      id: 'fedcba9876',
      action: 'new',
      cwd: '/Users/me/dev/old',
      title: 'Old notes',
      remoteBytes: 2048,
      host: 'mbp',
      pushedAt: 1,
      newestMtime: 1,
      archived: true,
    });
    expect(i.sub).toBe('fedcba98 · ~/dev/old · 2.0 KB');
    expect(i.tag).toBe('In storage only');
    const p = projectRows([{ cwd: '/a' }, { cwd: '/b' }, { cwd: '/b' }, {}], ['/c']);
    expect(p.map((r) => [r.id, r.sub])).toEqual([
      ['/c', ''],
      ['/b', '2 conversations'],
      ['/a', '1 conversation'],
    ]);
    expect(filterRows([s, i], 'NOTES').map((r) => r.id)).toEqual(['fedcba9876']);
    expect(filterRows([s, i], ' ').length).toBe(2);
  });

  it('puts the refusals in words', () => {
    expect(moveErrorText(new ApiError('no passphrase', 409, 'http', 'no_passphrase'))).toBe(
      'Set a backup passphrase first: every storage opens with it.',
    );
    expect(moveErrorText(new ApiError('x', 409, 'http', 'backup_unauthenticated'))).toContain(
      'failed its check',
    );
    expect(
      moveErrorText(
        new ApiError('copying within the shared folder is refused', 422, 'http', 'same_directory'),
      ),
    ).toBe('Copying within the shared folder is refused.');
    expect(moveErrorText(new ApiError('nope', 403, 'http', 'dangerous_blocked'))).toContain(
      'Remote › Devices',
    );
    expect(
      moveErrorText(
        new ApiError(
          '/x/work/store is inside (or around) the directory of account "Work"',
          422,
          'http',
          'invalid_storage',
        ),
      ),
    ).toBe(
      'That folder can\'t be a storage: /x/work/store is inside (or around) the directory of account "Work".',
    );
    expect(
      moveErrorText(
        new ApiError(
          'the storage Drive is not available: its folder is missing',
          409,
          'http',
          'storage_unavailable',
        ),
      ),
    ).toBe('The storage Drive is not available: its folder is missing.');
    expect(moveErrorText(new ApiError('', 404, 'http', 'storage_not_found'))).toBe(
      'That storage is no longer there.',
    );
  });
});
