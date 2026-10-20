import {
  accountCanRun,
  accountForCwd,
  accountNameProblem,
  dirContains,
  homeOf,
  needsCopy,
  normalizeAccounts,
  normalizeCapture,
  sharersOf,
  suggestAccountDir,
  transferTargets,
} from './accounts';
import { buildAccounts } from './mock/fixtures';
import { AccountsView } from './models';

const WORK = 'acc_5f1c9e2b7a40';
const PRO = 'acc_9b02d3e6c1f8';

describe('account helpers', () => {
  it('reads the registry tolerantly and puts the Default first', () => {
    const v = normalizeAccounts({
      accounts: [
        { id: WORK, name: ' Work ', kind: 'directory', color: '#0EA5E9', signedIn: true, x: 1 },
        { id: 'default', name: 'Default', kind: 'directory', color: 'purple', live: 2 },
        { id: PRO, name: 'Pro', kind: 'token', attention: 'whatever', tokenStored: 'yes' },
        { id: 'acc_000000000000', name: 'Odd', kind: 'mystery' },
        { id: WORK, name: 'Duplicate', kind: 'directory' },
        null,
        { name: 'No id', kind: 'token' },
      ],
      defaults: { newSession: WORK, byProject: { '/a/b': WORK, relative: PRO, '/c': 3 } },
      copyConsent: [{ from: 'default', to: WORK }, { from: 'default' }, 'nope'],
      secretStore: 'vault',
      tokenCapture: true,
    });
    expect(v?.accounts.map((a) => a.id)).toEqual(['default', WORK, PRO]);
    const [def, work, pro] = v!.accounts;
    expect(def.isDefault).toBeTrue();
    // A colour that isn't #rrggbb falls back; a good one is kept (lower case).
    expect(def.color).toBe('#8b5cf6');
    expect(def.live).toBe(2);
    expect(work.name).toBe('Work');
    expect(work.color).toBe('#0ea5e9');
    expect(work.signedIn).toBeTrue();
    expect(pro.attention).toBeUndefined();
    expect(pro.tokenStored).toBeFalse();
    expect(v?.defaults).toEqual({
      newSession: WORK,
      bridge: 'default',
      byProject: { '/a/b': WORK },
    });
    expect(v?.copyConsent).toEqual([{ from: 'default', to: WORK }]);
    expect(v?.secretStore).toBeUndefined();
    expect(v?.tokenCapture).toBeTrue();

    expect(normalizeAccounts(null)).toBeNull();
    expect(normalizeAccounts({ defaults: {} })).toBeNull();
  });

  it('keeps only an http(s) sign-in link of a capture', () => {
    expect(
      normalizeCapture({ state: 'running', url: 'javascript:alert(1)', asksForCode: true }),
    ).toEqual({ state: 'running', asksForCode: true, startedAt: 0 });
    expect(
      normalizeCapture({ state: 'running', url: 'https://claude.ai/oauth', startedAt: 5 }),
    ).toEqual({
      state: 'running',
      url: 'https://claude.ai/oauth',
      asksForCode: false,
      startedAt: 5,
    });
    expect(normalizeCapture({ state: 'exploded' })).toBeUndefined();
  });

  it('picks the account of the longest project folder, else the default for new sessions', () => {
    const view: AccountsView = {
      ...buildAccounts(0, true),
      defaults: {
        newSession: PRO,
        bridge: 'default',
        byProject: {
          '/Users/me/dev': WORK,
          '/Users/me/dev/api/': 'default',
          '/Users/me/gone': 'acc_ffffffffffff',
        },
      },
    };
    expect(accountForCwd(view, '/Users/me/dev/api/src')).toEqual({
      id: 'default',
      via: 'project',
      dir: '/Users/me/dev/api/',
    });
    expect(accountForCwd(view, '/Users/me/dev/web').id).toBe(WORK);
    // Whole path components only: /Users/me/dev2 is not inside /Users/me/dev.
    expect(accountForCwd(view, '/Users/me/dev2')).toEqual({ id: PRO, via: 'default' });
    // A rule naming a removed account is skipped.
    expect(accountForCwd(view, '/Users/me/gone/x').id).toBe(PRO);
    expect(accountForCwd(null, '/x')).toEqual({ id: 'default', via: 'default' });
    expect(dirContains('/', '/anything')).toBeTrue();
    expect(dirContains('/a/b', '/a/b')).toBeTrue();
    expect(dirContains('/a/b', '/a/bc')).toBeFalse();
  });

  it('knows which accounts share a directory', () => {
    const def = { id: 'default', kind: 'directory' };
    const work = { id: WORK, kind: 'directory' };
    const pro = { id: PRO, kind: 'token' };
    expect(homeOf(pro)).toBe('default');
    expect(needsCopy(def, pro)).toBeFalse();
    expect(needsCopy(pro, def)).toBeFalse();
    // The unregistered-token pseudo account runs in the Default's directory.
    expect(needsCopy({ id: 'unregistered', kind: 'token' }, pro)).toBeFalse();
    expect(needsCopy(def, work)).toBeTrue();
    expect(needsCopy(work, pro)).toBeTrue();
    expect(accountCanRun({ attention: 'tokenMissing' })).toBeFalse();
    expect(accountCanRun({ attention: 'signedOut' })).toBeFalse();
    expect(accountCanRun({ attention: 'authFailed' })).toBeTrue();
    expect(accountCanRun({})).toBeTrue();
  });

  it('moves and copies only into directory accounts with another directory', () => {
    const list = buildAccounts(0, true).accounts;
    const ids = (from: { id: string; kind: string }) =>
      transferTargets(list, from).map((a) => a.id);
    // From the Default's directory (the Default, or a token account in it):
    // only Work keeps another one.
    expect(ids({ id: 'default', kind: 'directory' })).toEqual([WORK]);
    expect(ids({ id: PRO, kind: 'token' })).toEqual([WORK]);
    // From Work: the Default's directory, which the token account shares.
    expect(ids({ id: WORK, kind: 'directory' })).toEqual(['default']);
    expect(sharersOf(list, 'default').map((a) => a.id)).toEqual([PRO]);
    expect(sharersOf(list, WORK)).toEqual([]);
    // With only the Default there is nowhere to go.
    expect(
      transferTargets(buildAccounts(0, false).accounts, { id: 'default', kind: 'directory' }),
    ).toEqual([]);
  });

  it('checks names by the server rules', () => {
    const list = [
      { id: 'default', name: 'Default' },
      { id: WORK, name: 'Work' },
    ];
    expect(accountNameProblem('  ', list)).toBe('Enter a name.');
    expect(accountNameProblem('x'.repeat(41), list)).toBe('At most 40 characters.');
    expect(accountNameProblem('.hidden', list)).toContain('dot');
    expect(accountNameProblem('a/b', list)).toContain("can't contain");
    expect(accountNameProblem('work', list)).toBe('Another account already has this name.');
    expect(accountNameProblem('Work', list, WORK)).toBe('');
    expect(accountNameProblem('Marcus (Pro)', list)).toBe('');
    expect(suggestAccountDir('Work Stuff!')).toBe('~/.claude-work-stuff');
    expect(suggestAccountDir('…')).toBe('~/.claude-work');
  });
});
