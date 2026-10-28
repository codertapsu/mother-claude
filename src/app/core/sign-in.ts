import {
  AccountView,
  IdentityWarning,
  SignInBrowser,
  SignInKind,
  SignInState,
  SignInView,
} from './models';

/**
 * Signing an account in (docs/ACCOUNTS.md §15.2–15.3): the sign-in as the
 * wire says it — read tolerantly, its link only when it is Claude's own
 * https page — one sign-in at a time across the accounts, the browsers the
 * link may open in, who signed in, and the words for all of it.
 */

const KINDS: readonly SignInKind[] = ['login', 'token'];
const STATES: readonly SignInState[] = ['starting', 'open', 'code', 'done', 'failed', 'cancelled'];
/** The states of a sign-in that still runs. */
const RUNNING: ReadonlySet<SignInState> = new Set(['starting', 'open', 'code']);
const BROWSERS: readonly SignInBrowser[] = ['default', 'chrome', 'chrome-private'];
/** The only places a sign-in link may lead (and their subdomains). */
const SIGN_IN_HOSTS = ['claude.ai', 'claude.com', 'anthropic.com'];

function str(v: unknown): string | undefined {
  return typeof v === 'string' && v.trim() ? v.trim() : undefined;
}

function obj(v: unknown): Record<string, unknown> | null {
  return v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : null;
}

/** An `https` link to Claude's or Anthropic's own pages, else undefined: no
 * other link is ever offered to open or copy. */
export function safeSignInUrl(raw: unknown): string | undefined {
  const text = str(raw);
  if (!text) return undefined;
  let url: URL;
  try {
    url = new URL(text);
  } catch {
    return undefined;
  }
  if (url.protocol !== 'https:' || url.username || url.password) return undefined;
  const host = url.hostname.toLowerCase();
  return SIGN_IN_HOSTS.some((h) => host === h || host.endsWith(`.${h}`)) ? text : undefined;
}

/** An account's sign-in from the wire, or undefined when it isn't one. */
export function normalizeSignIn(raw: unknown): SignInView | undefined {
  const r = obj(raw);
  const kind = KINDS.find((k) => k === r?.['kind']);
  const state = STATES.find((x) => x === r?.['state']);
  if (!r || !kind || !state) return undefined;
  const url = safeSignInUrl(r['url']);
  const opened = BROWSERS.find((b) => b === r['opened']);
  const error = str(r['error']);
  const startedAt = typeof r['startedAt'] === 'number' ? r['startedAt'] : 0;
  const finishedAt = typeof r['finishedAt'] === 'number' ? r['finishedAt'] : undefined;
  return {
    kind,
    state,
    ...(url ? { url } : {}),
    ...(opened ? { opened } : {}),
    startedAt,
    ...(finishedAt !== undefined ? { finishedAt } : {}),
    ...(error ? { error } : {}),
  };
}

/** How long a finished sign-in's outcome stays in view: the run stays on
 * the account, its news goes quietly. */
export const SIGN_IN_NEWS_MS = 3 * 60_000;

/** A sign-in that ended lately (its outcome is still news). */
export function signInEndedLately(s: SignInView | null | undefined, now: number): boolean {
  if (!s || signInRunning(s)) return false;
  return now - (s.finishedAt ?? s.startedAt) < SIGN_IN_NEWS_MS;
}

/** An account's identity warning from the wire, or undefined. */
export function normalizeIdentityWarning(raw: unknown): IdentityWarning | undefined {
  const r = obj(raw);
  const kind = r?.['kind'];
  if (kind !== 'changed' && kind !== 'same-as') return undefined;
  const account = str(r?.['account']);
  return { kind, ...(account ? { account } : {}) };
}

/** The sign-in still runs. */
export function signInRunning(s: SignInView | null | undefined): boolean {
  return !!s && RUNNING.has(s.state);
}

/** The account's sign-in runs: the new one, or an older server's browser
 * login or token capture. */
export function accountSigningIn(
  a: Pick<AccountView, 'signIn' | 'signingIn' | 'capture'>,
): boolean {
  if (a.signIn) return signInRunning(a.signIn);
  return a.signingIn || a.capture?.state === 'running';
}

/** The account whose sign-in runs (only one at a time runs, §15.3). */
export function activeSignIn(accounts: readonly AccountView[]): AccountView | undefined {
  return accounts.find(accountSigningIn);
}

/** What the sign-in buttons of `a` say while another account's runs ('' when
 * none does, or it is `a`'s own). */
export function signInWaitText(
  a: Pick<AccountView, 'id'>,
  accounts: readonly AccountView[],
): string {
  const other = activeSignIn(accounts);
  return other && other.id !== a.id ? `Waiting for ${other.name}'s sign-in` : '';
}

/** The same sign-in as two reads say it: ours (with its link, from the
 * desktop's own read) and the frame's (the latest state, without the
 * link). The later run wins; within one run, the further state, and the
 * link either has. */
export function mergeSignIn(
  own: SignInView | null | undefined,
  frame: SignInView | null | undefined,
): SignInView | undefined {
  if (!own) return frame ?? undefined;
  if (!frame) return own;
  if (own.startedAt !== frame.startedAt) return own.startedAt > frame.startedAt ? own : frame;
  const later = STATES.indexOf(frame.state) > STATES.indexOf(own.state) ? frame : own;
  const other = later === own ? frame : own;
  const url = own.url ?? frame.url;
  const opened = later.opened ?? other.opened;
  return { ...later, ...(url ? { url } : {}), ...(opened ? { opened } : {}) };
}

/** The browsers a sign-in link may open in, as the Mac lists them (the
 * default one always). */
export function normalizeBrowsers(raw: unknown): SignInBrowser[] {
  const list = Array.isArray(raw) ? raw : obj(raw)?.['browsers'];
  const found = new Set<SignInBrowser>(['default']);
  if (Array.isArray(list)) {
    for (const b of list) {
      const known = BROWSERS.find((x) => x === b);
      if (known) found.add(known);
    }
  }
  return BROWSERS.filter((b) => found.has(b));
}

/** "Open in browser", "Open in Chrome", "Open in a private Chrome window". */
export function browserLabel(b: SignInBrowser): string {
  switch (b) {
    case 'chrome':
      return 'Open in Chrome';
    case 'chrome-private':
      return 'Open in a private Chrome window';
    default:
      return 'Open in browser';
  }
}

/** What an identity warning says (`nameOf` names the other account). */
export function identityWarningText(
  w: IdentityWarning,
  nameOf: (id: string) => string | undefined,
): string {
  if (w.kind === 'same-as') {
    const other = (w.account && nameOf(w.account)) || 'another account';
    return `It signed in as the same Claude account as ${other}: each account here should be a different Claude account.`;
  }
  return 'It signed in as a different Claude account than before.';
}
