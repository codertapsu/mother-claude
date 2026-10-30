import { signInErrorText } from '../shared/accounts';
import { normalizeAccounts } from './accounts';
import { ApiError } from './api.service';
import { accountFixture } from './mock/fixtures';
import { SignInView } from './models';
import {
  accountSigningIn,
  activeSignIn,
  advanceSignIn,
  browserLabel,
  identityWarningText,
  isLongLivedToken,
  looksLikeToken,
  mergeSignIn,
  normalizeBrowsers,
  normalizeIdentityWarning,
  normalizeSignIn,
  safeSignInUrl,
  signInEndedLately,
  signInRunning,
  signInWaitText,
} from './sign-in';

describe('sign-in helpers', () => {
  it("offers only Claude's and Anthropic's own https pages", () => {
    expect(safeSignInUrl('https://claude.ai/oauth/authorize?code=true')).toBe(
      'https://claude.ai/oauth/authorize?code=true',
    );
    expect(safeSignInUrl('https://console.anthropic.com/oauth')).toBeDefined();
    expect(safeSignInUrl('https://auth.claude.com/x')).toBeDefined();
    expect(safeSignInUrl('http://claude.ai/oauth')).toBeUndefined();
    expect(safeSignInUrl('https://claude.ai.evil.example/x')).toBeUndefined();
    expect(safeSignInUrl('https://notclaude.ai/x')).toBeUndefined();
    expect(safeSignInUrl('https://user:pw@claude.ai/x')).toBeUndefined();
    expect(safeSignInUrl('javascript:alert(1)')).toBeUndefined();
    expect(safeSignInUrl('not a url')).toBeUndefined();
  });

  it('reads a sign-in and an identity warning tolerantly', () => {
    expect(
      normalizeSignIn({
        kind: 'login',
        state: 'open',
        url: 'https://claude.ai/oauth/authorize',
        startedAt: 5,
      }),
    ).toEqual({
      kind: 'login',
      state: 'open',
      url: 'https://claude.ai/oauth/authorize',
      startedAt: 5,
    });
    // A link elsewhere is dropped; the rest stays.
    expect(
      normalizeSignIn({ kind: 'token', state: 'code', url: 'https://evil.example', startedAt: 1 }),
    ).toEqual({ kind: 'token', state: 'code', startedAt: 1 });
    expect(normalizeSignIn({ kind: 'login', state: 'weird' })).toBeUndefined();
    expect(normalizeSignIn(null)).toBeUndefined();
    expect(normalizeIdentityWarning({ kind: 'same-as', account: 'acc_w' })).toEqual({
      kind: 'same-as',
      account: 'acc_w',
    });
    expect(normalizeIdentityWarning({ kind: 'changed', account: '' })).toEqual({ kind: 'changed' });
    expect(normalizeIdentityWarning({ kind: 'other' })).toBeUndefined();
    // And on an account from the wire.
    const view = normalizeAccounts({
      accounts: [
        {
          id: 'default',
          name: 'Default',
          kind: 'directory',
          signIn: { kind: 'login', state: 'starting', startedAt: 3 },
          identityWarning: { kind: 'changed' },
        },
      ],
    });
    expect(view?.accounts[0].signIn?.state).toBe('starting');
    expect(view?.accounts[0].identityWarning).toEqual({ kind: 'changed' });
  });

  it('reads where the link opened and when it ended, and lets the outcome go after a while', () => {
    const run = normalizeSignIn({
      kind: 'login',
      state: 'done',
      opened: 'default',
      startedAt: 1_000,
      finishedAt: 2_000,
    });
    expect(run).toEqual({
      kind: 'login',
      state: 'done',
      opened: 'default',
      startedAt: 1_000,
      finishedAt: 2_000,
    });
    expect(
      normalizeSignIn({ kind: 'login', state: 'open', opened: 'safari', startedAt: 1 }),
    ).toEqual({ kind: 'login', state: 'open', startedAt: 1 });
    expect(signInEndedLately(run, 2_000 + 60_000)).toBeTrue();
    expect(signInEndedLately(run, 2_000 + 4 * 60_000)).toBeFalse();
    expect(signInEndedLately({ kind: 'login', state: 'open', startedAt: 1 }, 2)).toBeFalse();
    // Two reads of one run: the further state, the link and the browser either has.
    expect(
      mergeSignIn(
        { kind: 'token', state: 'open', url: 'https://claude.ai/x', startedAt: 5 },
        { kind: 'token', state: 'open', opened: 'chrome-private', startedAt: 5 },
      ),
    ).toEqual({
      kind: 'token',
      state: 'open',
      url: 'https://claude.ai/x',
      opened: 'chrome-private',
      startedAt: 5,
    });
  });

  it('knows which sign-in runs, one at a time', () => {
    const work = accountFixture({
      id: 'acc_w',
      name: 'Work',
      signIn: { kind: 'login', state: 'open', startedAt: 1 },
    });
    const pro = accountFixture({ id: 'acc_p', name: 'Pro', kind: 'token' });
    expect(signInRunning(work.signIn)).toBeTrue();
    expect(signInRunning({ kind: 'login', state: 'done', startedAt: 1 })).toBeFalse();
    expect(activeSignIn([pro, work])?.id).toBe('acc_w');
    expect(signInWaitText(pro, [pro, work])).toBe("Waiting for Work's sign-in");
    expect(signInWaitText(work, [pro, work])).toBe('');
    // An older server: a browser login or a token capture that runs.
    expect(accountSigningIn(accountFixture({ id: 'x', signingIn: true }))).toBeTrue();
    expect(
      accountSigningIn(
        accountFixture({
          id: 'y',
          capture: { state: 'running', asksForCode: false, startedAt: 1 },
        }),
      ),
    ).toBeTrue();
    expect(activeSignIn([pro])).toBeUndefined();
  });

  it("keeps the link of the desktop's own read when a frame moves the state on", () => {
    const own = {
      kind: 'token' as const,
      state: 'open' as const,
      url: 'https://claude.ai/x',
      startedAt: 7,
    };
    const frame = { kind: 'token' as const, state: 'code' as const, startedAt: 7 };
    expect(mergeSignIn(own, frame)).toEqual({ ...frame, url: 'https://claude.ai/x' });
    // A newer run wins whole.
    const next = { kind: 'token' as const, state: 'starting' as const, startedAt: 9 };
    expect(mergeSignIn(own, next)).toEqual(next);
    expect(mergeSignIn(null, frame)).toEqual(frame);
  });

  it('reads whether a code went in, why one was turned down, and the run’s count', () => {
    expect(
      normalizeSignIn({ kind: 'token', state: 'open', startedAt: 1, codeSent: true, rev: 4 }),
    ).toEqual({ kind: 'token', state: 'open', startedAt: 1, codeSent: true, rev: 4 });
    expect(
      normalizeSignIn({
        kind: 'login',
        state: 'code',
        startedAt: 1,
        codeSent: 'yes',
        codeError: ' Invalid code. ',
        rev: 'x',
      }),
    ).toEqual({ kind: 'login', state: 'code', startedAt: 1, codeError: 'Invalid code.' });
  });

  it('takes the later read of one run by its count, though its state went back', () => {
    const asked = {
      kind: 'token' as const,
      state: 'code' as const,
      url: 'https://claude.ai/a',
      opened: 'chrome-private' as const,
      startedAt: 7,
      rev: 3,
    };
    // A code went in: back to `open`, the link kept, where it opened too.
    const sent = { ...asked, state: 'open' as const, codeSent: true, rev: 4 };
    const sentFrame: SignInView = { ...sent, url: undefined };
    expect(mergeSignIn(asked, sentFrame)).toEqual(sent);
    // An older frame does not win back.
    expect(mergeSignIn(sent, { ...asked, url: undefined })).toEqual(sent);
    // It started over: a new link, not opened yet (the desktop's read has it).
    const again = {
      kind: 'token' as const,
      state: 'open' as const,
      url: 'https://claude.ai/b',
      startedAt: 7,
      rev: 6,
    };
    expect(mergeSignIn(again, sentFrame)).toEqual(again);
    // Answers that cross: the earlier one does not undo the later.
    expect(advanceSignIn(again, sent)).toEqual(again);
    expect(advanceSignIn(sent, again)).toEqual(again);
    // A later answer without the link keeps ours.
    expect(advanceSignIn(sent, { ...sentFrame, rev: 5 })).toEqual({ ...sent, rev: 5 });
    // Without counts, an answer is the freshest word; another run, whole.
    const noRev: SignInView = { ...asked, rev: undefined };
    expect(advanceSignIn(noRev, { ...noRev, state: 'open', url: undefined })).toEqual({
      ...noRev,
      state: 'open',
    });
    const next = { kind: 'token' as const, state: 'starting' as const, startedAt: 9 };
    expect(advanceSignIn(sent, next)).toEqual(next);
  });

  it('tells a token from a code', () => {
    const token = 'sk-ant-oat01-abcdefghijklmnop_-0123456789';
    expect(looksLikeToken(token)).toBeTrue();
    expect(looksLikeToken(`export CLAUDE_CODE_OAUTH_TOKEN=${token}`)).toBeTrue();
    expect(looksLikeToken('SK-ANT-API03-abc')).toBeTrue();
    expect(looksLikeToken('Xq3vYz0123#Rk1yZ0123')).toBeFalse();
    expect(isLongLivedToken(` ${token}\n`)).toBeTrue();
    expect(isLongLivedToken('sk-ant-api03-abcdefghijklmnop')).toBeFalse();
    expect(isLongLivedToken(`token: ${token}`)).toBeFalse();
  });

  it('lists the browsers, the default one always, and names them', () => {
    expect(normalizeBrowsers({ browsers: ['chrome-private', 'safari', 'chrome'] })).toEqual([
      'default',
      'chrome',
      'chrome-private',
    ]);
    expect(normalizeBrowsers(null)).toEqual(['default']);
    expect(browserLabel('chrome-private')).toBe('Open in a private Chrome window');
    expect(browserLabel('default')).toBe('Open in browser');
  });

  it('says who signed in, and why a sign-in is refused', () => {
    const name = (id: string) => (id === 'acc_w' ? 'Work' : undefined);
    expect(identityWarningText({ kind: 'changed' }, name)).toBe(
      'It signed in as a different Claude account than before.',
    );
    expect(identityWarningText({ kind: 'same-as', account: 'acc_w' }, name)).toContain(
      'the same Claude account as Work',
    );
    const running = new ApiError('busy', 409, 'http', 'sign_in_running', undefined, undefined, {
      account: 'acc_w',
    });
    expect(signInErrorText(running, name)).toBe(
      "Work's sign-in is running: one sign-in runs at a time. Finish or cancel it first.",
    );
    // An account this view doesn't know yet: the name the Mac sent.
    const unknown = new ApiError('busy', 409, 'http', 'sign_in_running', undefined, undefined, {
      account: 'acc_new',
      accountName: 'Side project',
    });
    expect(signInErrorText(unknown, name)).toContain("Side project's sign-in is running");
    expect(signInErrorText(new ApiError('', 409, 'http', 'sign_in_no_link'))).toContain(
      "hasn't shown its link yet",
    );
    expect(signInErrorText(new ApiError('', 500, 'http', 'browser_failed'))).toContain(
      "The browser couldn't be opened",
    );
    expect(signInErrorText(new ApiError('', 422, 'http', 'sign_in_unavailable'))).toContain(
      "can't run the sign-in itself",
    );
  });
});
