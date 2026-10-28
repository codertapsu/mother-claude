import { AccountView, SessionSummary } from '../models';
import { accountFixture } from './fixtures';
import { MockRole } from './mock-api';
import { MockSwitch, SwitchHost } from './mock-switch';

const session = (id: string, over: Partial<SessionSummary>): SessionSummary =>
  ({
    id,
    live: true,
    owned: true,
    status: 'idle',
    surface: 'mother-claude',
    ...over,
  }) as SessionSummary;

describe('MockSwitch (switching and sign-ins, as the server answers)', () => {
  let role: MockRole;
  let sessions: SessionSummary[];
  let accounts: AccountView[];
  let owners: Record<string, string>;
  let consent: boolean;
  let mock: MockSwitch;

  beforeEach(() => {
    jasmine.clock().install();
    role = 'desktop';
    consent = false;
    sessions = [
      session('busy', { status: 'working' }),
      session('idle', {}),
      session('vscode', { owned: false, surface: 'vscode' }),
      session('ended', { live: false, status: 'ended' }),
    ];
    owners = {};
    accounts = [
      accountFixture({ id: 'default', name: 'Default', isDefault: true, email: 'me@example.com' }),
      accountFixture({ id: 'acc_w', name: 'Work', dir: '/u/.claude-work' }),
      accountFixture({ id: 'acc_p', name: 'Pro', kind: 'token', tokenStored: true }),
    ];
    const host: SwitchHost = {
      get role() {
        return role;
      },
      sessions: () => sessions,
      accounts: () => accounts,
      accountOf: (id) => owners[id] ?? 'default',
      reassign: (id, account) => (owners[id] = account),
      patchSession: (id, change) =>
        (sessions = sessions.map((s) => (s.id === id ? { ...s, ...change } : s))),
      hasCopyConsent: () => consent,
      updateAccount: (id, change) =>
        (accounts = accounts.map((a) => (a.id === id ? { ...a, ...change } : a))),
      publishAccounts: () => undefined,
      signedIn: () => ({ signedIn: true }),
      accountJson: (id, status = 200) =>
        new Response(JSON.stringify(accounts.find((a) => a.id === id)), { status }),
      moveJob: () => 'job-move-1',
    };
    mock = new MockSwitch(host);
  });

  afterEach(() => jasmine.clock().uninstall());

  const call = async (path: string, method: string, body?: unknown) => {
    const res = mock.route(path, method, body);
    if (!res) return null;
    const text = await res.text();
    return { status: res.status, body: text ? JSON.parse(text) : null };
  };
  const of = (id: string) => accounts.find((a) => a.id === id)!;

  it('switches a session Mother Claude runs, once its turn ends unless asked now', async () => {
    expect(await call('/sessions/idle/account', 'POST', { account: 'acc_p' })).toEqual({
      status: 200,
      body: { id: 'idle', account: 'acc_p', moved: false },
    });
    expect(owners['idle']).toBe('acc_p');

    const pending = await call('/sessions/busy/account', 'POST', { account: 'acc_p' });
    expect(pending).toEqual({ status: 202, body: { id: 'busy', account: 'acc_p', pending: true } });
    expect(sessions[0].switchingTo).toBe('acc_p');
    jasmine.clock().tick(4_100);
    expect(owners['busy']).toBe('acc_p');
    expect(sessions[0].switchingTo).toBeUndefined();

    // Cancelled before the turn ends: nothing switches.
    owners['busy'] = 'default';
    sessions[0] = { ...sessions[0], status: 'working' };
    await call('/sessions/busy/account', 'POST', { account: 'acc_p' });
    expect(await call('/sessions/busy/account', 'DELETE')).toEqual({
      status: 200,
      body: { id: 'busy', account: 'default', cancelled: true },
    });
    jasmine.clock().tick(5_000);
    expect(owners['busy']).toBe('default');
    // Now: the turn is interrupted, the switch is done.
    expect(
      (await call('/sessions/busy/account', 'POST', { account: 'acc_p', when: 'now' }))?.status,
    ).toBe(200);
    expect(owners['busy']).toBe('acc_p');
  });

  it('refuses one another app runs, and asks the copy consent for another directory', async () => {
    const refused = await call('/sessions/vscode/account', 'POST', { account: 'acc_p' });
    expect([refused?.status, refused?.body.error.code]).toEqual([409, 'session_not_owned']);
    const ask = await call('/sessions/ended/account', 'POST', { account: 'acc_w' });
    expect(ask?.body.error).toEqual(
      jasmine.objectContaining({ code: 'copy_consent_required', from: 'default', to: 'acc_w' }),
    );
    consent = true;
    expect((await call('/sessions/ended/account', 'POST', { account: 'acc_w' }))?.body).toEqual({
      id: 'ended',
      account: 'acc_w',
      moved: true,
      job: 'job-move-1',
    });
    const when = await call('/sessions/idle/account', 'POST', { account: 'acc_w', when: 'later' });
    expect(when?.body.error.code).toBe('invalid_when');
  });

  it('signs in with a link, one at a time, and lands with a changed identity', async () => {
    expect((await call('/accounts/acc_w/login', 'POST', {}))?.status).toBe(202);
    expect(of('acc_w').signIn?.state).toBe('starting');
    jasmine.clock().tick(800);
    expect(of('acc_w').signIn?.state).toBe('open');
    expect(of('acc_w').signIn?.url).toContain('https://claude.ai/');
    // Another account waits.
    const busy = await call('/accounts/acc_p/token', 'POST', {});
    expect([busy?.status, busy?.body.error.code, busy?.body.error.accountName]).toEqual([
      409,
      'sign_in_running',
      'Work',
    ]);
    // The Mac opened it in the default browser; a private window on asking.
    expect(of('acc_w').signIn?.opened).toBe('default');
    const opened = await call('/accounts/acc_w/sign-in/open', 'POST', {
      browser: 'chrome-private',
    });
    expect([opened?.status, opened?.body.signIn.opened]).toEqual([200, 'chrome-private']);
    // Asked again, the same run comes back.
    expect((await call('/accounts/acc_w/login', 'POST', {}))?.body.signIn.startedAt).toBe(
      of('acc_w').signIn?.startedAt,
    );
    jasmine.clock().tick(5_000);
    expect(of('acc_w').signIn?.state).toBe('done');
    expect(of('acc_w').signIn?.finishedAt).toBeDefined();
    expect(of('acc_w').identityWarning).toEqual({ kind: 'changed' });
    await call('/accounts/acc_w/identity/accept', 'POST', {});
    expect(of('acc_w').identityWarning).toBeUndefined();
  });

  it('waits for the code of a token sign-in, and keeps it to the desktop', async () => {
    const start = await call('/accounts/acc_p/token', 'POST', {});
    expect([start?.status, start?.body.capture.state, start?.body.signIn.kind]).toEqual([
      202,
      'running',
      'token',
    ]);
    jasmine.clock().tick(2_800);
    expect(of('acc_p').signIn?.state).toBe('code');
    // Nothing opened by itself.
    expect(of('acc_p').signIn?.opened).toBeUndefined();
    const typed = await call('/accounts/acc_p/sign-in/code', 'POST', { code: 'abc#123' });
    expect(typed?.body.signIn.state).toBe('open');
    jasmine.clock().tick(1_300);
    expect(of('acc_p').signIn?.state).toBe('done');
    expect(of('acc_p').tokenStored).toBeTrue();
    // A pasted token is the backend's; a phone may not sign in.
    expect(mock.route('/accounts/acc_p/token', 'POST', { token: 'sk-ant-oat01-x' })).toBeNull();
    role = 'device';
    expect((await call('/accounts/acc_w/login', 'POST', {}))?.status).toBe(403);
  });
});
