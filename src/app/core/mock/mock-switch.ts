// Dev-only: switching a session's account and signing accounts in, for
// `?mock=1` (docs/ACCOUNTS.md §15). Never bundled in production builds.
//
// - `POST /sessions/{id}/account`: a session Mother Claude runs switches at
//   once when idle; busy with a turn it waits for the turn to end
//   (`switchingTo`, a few seconds here) unless asked `now`; one another app
//   runs is refused (`session_not_owned`); one that doesn't run switches for
//   its next continue. Another directory needs the copy consent, like
//   Continue on. `DELETE` cancels a waiting switch.
// - Sign-ins: a login (`POST /accounts/{id}/login`) shows its link, then
//   lands with a changed-identity warning; a token sign-in (`POST
//   /accounts/{id}/token {}`) shows its link and waits for the code the
//   browser shows (`?mockcapture=fail`: it fails instead). One runs at a
//   time. `GET|POST /accounts/{id}/sign-in/open`, `POST …/sign-in/code`,
//   `DELETE …/sign-in`, `POST /accounts/{id}/identity/accept`.

import { needsCopy } from '../accounts';
import { AccountView, SessionSummary, SignInView } from '../models';
import { urlParam } from '../platform';
import { MockRole } from './mock-api';

/** `?mockcapture=fail`: a token sign-in ends without a token. */
const CAPTURE_MODE = urlParam('mockcapture');
/** How long a busy session's turn goes on before a waiting switch lands. */
const TURN_ENDS_MS = 4_000;

/** What the switch and sign-in mock needs from the fixture backend. */
export interface SwitchHost {
  readonly role: MockRole;
  /** Every session the mock knows (live list and history). */
  sessions(): SessionSummary[];
  accounts(): AccountView[];
  /** The account a session runs under (the Default when none). */
  accountOf(sessionId: string): string;
  /** The session now runs under `account` (its chip follows). */
  reassign(sessionId: string, account: string): void;
  /** Change a session of the live list (broadcast). */
  patchSession(id: string, change: Partial<SessionSummary>): void;
  hasCopyConsent(from: string, to: string): boolean;
  updateAccount(id: string, change: Partial<AccountView>): void;
  /** Tell every client the accounts changed. */
  publishAccounts(): void;
  /** The account as signed in after a sign-in landed. */
  signedIn(a: AccountView): Partial<AccountView>;
  /** One account as the asker sees it (the link only for the desktop). */
  accountJson(id: string, status?: number): Response;
}

export class MockSwitch {
  /** The timers of each running sign-in, by account. */
  private readonly runs = new Map<string, ReturnType<typeof setTimeout>[]>();
  /** The timer of each switch waiting for its turn, by session. */
  private readonly waiting = new Map<string, ReturnType<typeof setTimeout>>();

  constructor(private readonly host: SwitchHost) {}

  /** Answers a route of §15, or null to let others answer. */
  route(path: string, method: string, body: unknown): Response | null {
    const b = obj(body) ?? {};
    const sw = /^\/sessions\/([^/]+)\/account$/.exec(path);
    if (sw) return this.sessionAccount(decodeURIComponent(sw[1]), method, b);
    const m = /^\/accounts\/([^/]+)(?:\/(.+))?$/.exec(path);
    if (!m) return null;
    const a = this.host.accounts().find((x) => x.id === decodeURIComponent(m[1]));
    if (!a) return null;
    const sub = m[2] ?? '';
    if (!sub && method === 'GET') return this.host.accountJson(a.id);
    const desktop = this.host.role === 'desktop';
    const signInRoute =
      sub === 'login' ||
      sub.startsWith('sign-in') ||
      sub === 'identity/accept' ||
      (sub === 'token' && (method === 'DELETE' || (method === 'POST' && !str(b['token']))));
    if (!signInRoute) return null;
    if (!desktop) {
      return fail(403, 'desktop_only', 'Only the desktop app can do this.');
    }
    if (sub === 'login' && method === 'POST') return this.start(a, 'login');
    if (sub === 'token' && method === 'POST') {
      const code = str(b['code']);
      return code ? this.code(a, code) : this.start(a, 'token');
    }
    if ((sub === 'token' || sub === 'sign-in') && method === 'DELETE') return this.cancel(a);
    if (sub === 'sign-in/code' && method === 'POST') return this.code(a, str(b['code']) ?? '');
    if (sub === 'sign-in/open' && method === 'GET') {
      return json({ browsers: ['default', 'chrome', 'chrome-private'] });
    }
    if (sub === 'sign-in/open' && method === 'POST') {
      const browser = str(b['browser']);
      if (browser !== 'default' && browser !== 'chrome' && browser !== 'chrome-private') {
        return fail(422, 'invalid_body', 'browser: default, chrome or chrome-private');
      }
      return a.signIn?.url && running(a.signIn)
        ? new Response(null, { status: 204 })
        : fail(409, 'sign_in_not_running', 'No sign-in of this account runs.');
    }
    if (sub === 'identity/accept' && method === 'POST') {
      this.host.updateAccount(a.id, { identityWarning: undefined });
      this.host.publishAccounts();
      return this.host.accountJson(a.id);
    }
    return null;
  }

  // ─────────────────────────────────────────────────────── Switching

  private sessionAccount(id: string, method: string, b: Record<string, unknown>): Response {
    const s = this.host.sessions().find((x) => x.id === id);
    if (!s) return fail(404, 'session_not_found', `no session ${id}`);
    if (method === 'DELETE') {
      const timer = this.waiting.get(id);
      if (!timer) return fail(409, 'switch_not_pending', 'No switch is waiting.');
      clearTimeout(timer);
      this.waiting.delete(id);
      this.host.patchSession(id, { switchingTo: undefined });
      return new Response(null, { status: 204 });
    }
    if (method !== 'POST') return fail(405, 'method_not_allowed', 'POST or DELETE');
    const asked = str(b['account']);
    const target = this.host.accounts().find((a) => a.id === asked);
    if (!target) return fail(422, 'unknown_account', `no account ${JSON.stringify(asked)}`);
    if (target.kind === 'token' && !target.tokenStored) {
      return fail(409, 'account_token_missing', `${target.name} has no token on this Mac yet.`);
    }
    const from = this.host.accountOf(id);
    if (from === target.id) return json({ id, account: target.id, moved: false });
    if (s.live && !s.owned) {
      return fail(
        409,
        'session_not_owned',
        "Another app runs this session: its account can't change while it runs there.",
      );
    }
    const source = this.host.accounts().find((a) => a.id === from) ?? {
      id: from,
      kind: 'directory' as const,
    };
    const moved = needsCopy(source, target);
    if (moved && !this.host.hasCopyConsent(from, target.id)) {
      return json(
        {
          error: {
            code: 'copy_consent_required',
            message:
              'That account keeps its own directory: the conversation moves there first. Allow copies between these two accounts, then try again.',
            from,
            to: target.id,
          },
        },
        409,
      );
    }
    const busy = s.live && (s.status === 'working' || s.status === 'starting');
    if (busy && b['when'] !== 'now') {
      // Once the turn ends.
      clearTimeout(this.waiting.get(id));
      this.host.patchSession(id, { switchingTo: target.id });
      this.waiting.set(
        id,
        setTimeout(() => {
          this.waiting.delete(id);
          this.host.patchSession(id, {
            switchingTo: undefined,
            status: 'idle',
            activity: { kind: 'idle', label: 'Idle' },
          });
          this.host.reassign(id, target.id);
        }, TURN_ENDS_MS),
      );
      return json({ pending: true }, 202);
    }
    if (busy) {
      // Now: the turn is interrupted first.
      this.host.patchSession(id, {
        status: 'idle',
        activity: { kind: 'idle', label: 'Interrupted' },
      });
    }
    this.host.reassign(id, target.id);
    return json({ id, account: target.id, moved });
  }

  // ─────────────────────────────────────────────────────── Signing in

  private start(a: AccountView, kind: SignInView['kind']): Response {
    if (kind === 'login' && a.kind === 'token') {
      return fail(422, 'login_failed', 'A token account signs in with its token.');
    }
    if (kind === 'token' && a.kind !== 'token') {
      return fail(422, 'not_a_token_account', 'this is not a token account');
    }
    // One sign-in at a time, across every account.
    const other = this.host.accounts().find((x) => x.id !== a.id && running(x.signIn));
    if (other) {
      return json(
        {
          error: {
            code: 'sign_in_running',
            message: `${other.name}'s sign-in is running: finish or cancel it first.`,
            account: other.id,
          },
        },
        409,
      );
    }
    this.stop(a.id);
    const startedAt = Date.now();
    const url =
      kind === 'login'
        ? `https://claude.ai/oauth/authorize?client_id=mock-client&response_type=code&scope=user%3Aprofile${a.email ? `&login_hint=${encodeURIComponent(a.email)}` : ''}`
        : 'https://claude.ai/oauth/authorize?code=true&client_id=mock-client&response_type=code&scope=user%3Ainference';
    this.set(a.id, { kind, state: 'starting', startedAt });
    const timers = [setTimeout(() => this.set(a.id, { kind, state: 'open', url, startedAt }), 700)];
    if (kind === 'login') {
      // The browser opened by itself; the sign-in lands — as someone else
      // than before, here.
      timers.push(
        setTimeout(() => {
          const cur = this.account(a.id);
          if (!cur || !running(cur.signIn)) return;
          this.host.updateAccount(a.id, {
            ...this.host.signedIn(cur),
            signingIn: false,
            signIn: { kind, state: 'done', startedAt },
            identityWarning: { kind: 'changed' },
          });
          this.host.publishAccounts();
        }, 5_000),
      );
    } else if (CAPTURE_MODE === 'fail') {
      timers.push(
        setTimeout(
          () =>
            this.set(a.id, {
              kind,
              state: 'failed',
              startedAt,
              error: 'setup-token exited before it showed a token',
            }),
          3_000,
        ),
      );
    } else {
      // The CLI asks for the code the browser shows.
      timers.push(setTimeout(() => this.set(a.id, { kind, state: 'code', url, startedAt }), 2_000));
    }
    this.runs.set(a.id, timers);
    return this.host.accountJson(a.id, 202);
  }

  private code(a: AccountView, code: string): Response {
    const run = a.signIn;
    if (!run || run.state !== 'code') {
      return fail(409, 'sign_in_not_running', 'No sign-in is waiting for a code.');
    }
    if (!code) return fail(422, 'invalid_body', 'Send the code the browser shows.');
    this.stop(a.id);
    this.set(a.id, { ...run, state: 'open' });
    this.runs.set(a.id, [
      setTimeout(() => {
        const cur = this.account(a.id);
        if (!cur) return;
        this.host.updateAccount(a.id, {
          ...this.host.signedIn(cur),
          tokenStored: true,
          signIn: { kind: run.kind, state: 'done', startedAt: run.startedAt },
        });
        this.host.publishAccounts();
      }, 1_200),
    ]);
    return this.host.accountJson(a.id);
  }

  private cancel(a: AccountView): Response {
    this.stop(a.id);
    if (running(a.signIn) && a.signIn) {
      this.set(a.id, { kind: a.signIn.kind, state: 'cancelled', startedAt: a.signIn.startedAt });
    }
    return new Response(null, { status: 204 });
  }

  /** The run as the account shows it (an older client reads a running
   * login as `signingIn`). */
  private set(id: string, run: SignInView): void {
    this.host.updateAccount(id, { signIn: run, signingIn: run.kind === 'login' && running(run) });
    this.host.publishAccounts();
  }

  private stop(id: string): void {
    for (const t of this.runs.get(id) ?? []) clearTimeout(t);
    this.runs.delete(id);
  }

  private account(id: string): AccountView | undefined {
    return this.host.accounts().find((a) => a.id === id);
  }
}

/** The sign-in still runs. */
function running(s: SignInView | undefined): boolean {
  return !!s && (s.state === 'starting' || s.state === 'open' || s.state === 'code');
}

function obj(v: unknown): Record<string, unknown> | null {
  return v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : null;
}

function str(v: unknown): string | undefined {
  return typeof v === 'string' && v.trim() ? v.trim() : undefined;
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function fail(status: number, code: string, message: string): Response {
  return json({ error: { code, message } }, status);
}
