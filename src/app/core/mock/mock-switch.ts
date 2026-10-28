// Dev-only: switching a session's account and signing accounts in, for
// `?mock=1` (docs/ACCOUNTS.md §15). Never bundled in production builds.
//
// - `POST /sessions/{id}/account`: a session Mother Claude runs switches at
//   once when idle; busy with a turn it waits for the turn to end
//   (`switchingTo`, a few seconds here) unless asked `now`; one another app
//   runs is refused (`session_not_owned`); one that doesn't run switches for
//   its next continue. Another directory needs the copy consent, like
//   Continue on. `DELETE` cancels a waiting switch. A running session's
//   switch is announced to every client (a `notice`), as the Mac does.
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
  /** A switch that moved a conversation: its undoable move job's id. */
  moveJob(sessionId: string, from: string, to: string): string;
  /** A notice to every client (the `notice` frame). */
  notice(level: 'info' | 'success' | 'warning' | 'error', text: string, session?: string): void;
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
    if (sub === 'login' && method === 'POST') {
      const email = b['email'];
      if (email !== undefined && (typeof email !== 'string' || !/^[^\s@]+@[^\s@]+$/.test(email))) {
        return fail(422, 'invalid_body', 'email: an email address');
      }
      return this.start(a, 'login');
    }
    // The token routes answer `{capture, signIn}`; the sign-in ones `{signIn}`.
    if (sub === 'token' && method === 'POST') {
      const code = str(b['code']);
      return code ? this.code(a, code, true) : this.start(a, 'token');
    }
    if (sub === 'token' && method === 'DELETE') return this.cancel(a, true);
    if (sub === 'sign-in' && method === 'DELETE') return this.cancel(a, false);
    if (sub === 'sign-in/code' && method === 'POST') {
      return this.code(a, str(b['code']) ?? '', false);
    }
    if (sub === 'sign-in/open' && method === 'GET') {
      return json({ browsers: ['default', 'chrome', 'chrome-private'] });
    }
    if (sub === 'sign-in/open' && method === 'POST') {
      const browser = str(b['browser']);
      if (browser !== 'default' && browser !== 'chrome' && browser !== 'chrome-private') {
        return fail(422, 'invalid_body', 'browser: default, chrome or chrome-private');
      }
      if (!running(a.signIn)) {
        return fail(409, 'sign_in_not_running', 'No sign-in of this account runs.');
      }
      if (!a.signIn?.url) return fail(409, 'sign_in_no_link', 'The sign-in has no link yet.');
      this.set(a.id, { ...a.signIn, opened: browser });
      return this.signInJson(a.id, false);
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
      return json({ id, account: this.host.accountOf(id), cancelled: true });
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
    if (b['when'] !== undefined && b['when'] !== 'idle' && b['when'] !== 'now') {
      return fail(422, 'invalid_when', 'when: idle or now');
    }
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
          this.announce(s, target);
        }, TURN_ENDS_MS),
      );
      return json({ id, account: target.id, pending: true }, 202);
    }
    if (busy) {
      // Now: the turn is interrupted first.
      this.host.patchSession(id, {
        status: 'idle',
        activity: { kind: 'idle', label: 'Interrupted' },
      });
    }
    const job = moved ? this.host.moveJob(id, from, target.id) : undefined;
    this.host.reassign(id, target.id);
    if (s.live) this.announce(s, target);
    return json({ id, account: target.id, moved, ...(job ? { job } : {}) });
  }

  /** Like the Mac: every client hears how a running session's switch
   * ended (one that doesn't run, only its asker). */
  private announce(s: SessionSummary, target: AccountView): void {
    const name = s.name?.trim() || `Session ${s.id.slice(0, 8)}`;
    this.host.notice('success', `${name} goes on under ${target.name}`, s.id);
  }

  // ─────────────────────────────────────────────────────── Signing in

  private start(a: AccountView, kind: SignInView['kind']): Response {
    if (kind === 'login' && a.kind === 'token') {
      return fail(422, 'login_failed', 'A token account signs in with its token.');
    }
    if (kind === 'token' && a.kind !== 'token') {
      return fail(422, 'not_a_token_account', 'this is not a token account');
    }
    // One sign-in at a time, across every account; the same account's
    // run is handed back (a double click starts nothing twice).
    if (running(a.signIn)) return this.started(a.id, kind);
    const other = this.host.accounts().find((x) => x.id !== a.id && running(x.signIn));
    if (other) {
      return json(
        {
          error: {
            code: 'sign_in_running',
            message: `${other.name}'s sign-in is running: finish or cancel it first.`,
            account: other.id,
            accountName: other.name,
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
    // A login's link opens in the default browser by itself (the Mac does
    // it); a token sign-in's opens only when asked.
    const opened = kind === 'login' ? { opened: 'default' as const } : {};
    const timers = [
      setTimeout(() => this.set(a.id, { kind, state: 'open', url, startedAt, ...opened }), 700),
    ];
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
            signIn: { ...cur.signIn, kind, state: 'done', startedAt, finishedAt: Date.now() },
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
              finishedAt: Date.now(),
              error: 'setup-token exited before it showed a token',
            }),
          3_000,
        ),
      );
    } else {
      // The CLI asks for the code the browser shows.
      timers.push(
        setTimeout(() => {
          const cur = this.account(a.id)?.signIn;
          if (cur && running(cur)) this.set(a.id, { ...cur, state: 'code' });
        }, 2_000),
      );
    }
    this.runs.set(a.id, timers);
    return this.started(a.id, kind);
  }

  /** A start's answer: the account view (a login), `{capture, signIn}` (a
   * token sign-in). */
  private started(id: string, kind: SignInView['kind']): Response {
    return kind === 'login' ? this.host.accountJson(id, 202) : this.signInJson(id, true, 202);
  }

  /** `{signIn}`, and the token routes' old `capture` view beside it. */
  private signInJson(id: string, withCapture: boolean, status = 200): Response {
    const run = this.account(id)?.signIn ?? null;
    return json({ ...(withCapture ? { capture: captureOf(run) } : {}), signIn: run }, status);
  }

  private code(a: AccountView, code: string, tokenRoute: boolean): Response {
    const run = a.signIn;
    if (!run || run.state !== 'code') {
      return fail(409, 'sign_in_not_running', 'No sign-in is waiting for a code.');
    }
    if (!code) return fail(422, 'invalid_body', 'Send the code the browser shows.');
    this.stop(a.id);
    // Back to `open` once a code is typed; it finishes by itself.
    this.set(a.id, { ...run, state: 'open' });
    this.runs.set(a.id, [
      setTimeout(() => {
        const cur = this.account(a.id);
        if (!cur) return;
        this.host.updateAccount(a.id, {
          ...this.host.signedIn(cur),
          tokenStored: true,
          signIn: { ...run, state: 'done', finishedAt: Date.now() },
        });
        this.host.publishAccounts();
      }, 1_200),
    ]);
    return this.signInJson(a.id, tokenRoute);
  }

  private cancel(a: AccountView, tokenRoute: boolean): Response {
    if (!running(a.signIn) || !a.signIn) {
      return fail(409, 'sign_in_not_running', 'No sign-in of this account runs.');
    }
    this.stop(a.id);
    this.set(a.id, { ...a.signIn, state: 'cancelled', finishedAt: Date.now() });
    return this.signInJson(a.id, tokenRoute);
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

/** A token run in the token routes' old shape. */
function captureOf(run: SignInView | null): Record<string, unknown> | null {
  if (!run || run.kind !== 'token') return null;
  const state = running(run)
    ? 'running'
    : run.state === 'done'
      ? 'captured'
      : run.state === 'failed'
        ? 'failed'
        : 'cancelled';
  return {
    state,
    asksForCode: run.state === 'code',
    startedAt: run.startedAt,
    ...(run.finishedAt ? { finishedAt: run.finishedAt } : {}),
    ...(run.error ? { error: run.error } : {}),
  };
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
