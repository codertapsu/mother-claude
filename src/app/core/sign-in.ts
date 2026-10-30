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
  const codeError = str(r['codeError']);
  const startedAt = typeof r['startedAt'] === 'number' ? r['startedAt'] : 0;
  const finishedAt = typeof r['finishedAt'] === 'number' ? r['finishedAt'] : undefined;
  const rev = r['rev'];
  return {
    kind,
    state,
    ...(url ? { url } : {}),
    ...(opened ? { opened } : {}),
    startedAt,
    ...(finishedAt !== undefined ? { finishedAt } : {}),
    ...(error ? { error } : {}),
    ...(r['codeSent'] === true ? { codeSent: true } : {}),
    ...(codeError ? { codeError } : {}),
    ...(typeof rev === 'number' && Number.isFinite(rev) ? { rev } : {}),
  };
}

/** Text holding a token or a key (`sk-ant-…`): never sent as a sign-in's
 * code (docs/ACCOUNTS.md §15.2). */
export function looksLikeToken(text: string): boolean {
  return /sk-ant-/i.test(text);
}

/** A long-lived token from `claude setup-token`, alone: what a token
 * account keeps. */
export function isLongLivedToken(text: string): boolean {
  return /^sk-ant-oat\d\d-[\w-]+$/.test(text.trim());
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
 * link). The later run wins; within one run, the later read (the higher
 * `rev`: a run goes back from `code` to `open` while a code is checked, or
 * when it starts over with a new link), else the further state — and the
 * later read's link, else the other's. */
export function mergeSignIn(
  own: SignInView | null | undefined,
  frame: SignInView | null | undefined,
): SignInView | undefined {
  if (!own) return frame ?? undefined;
  if (!frame) return own;
  if (own.startedAt !== frame.startedAt) return own.startedAt > frame.startedAt ? own : frame;
  const byRev = own.rev !== undefined && frame.rev !== undefined && own.rev !== frame.rev;
  const later = byRev
    ? (frame.rev ?? 0) > (own.rev ?? 0)
      ? frame
      : own
    : STATES.indexOf(frame.state) > STATES.indexOf(own.state)
      ? frame
      : own;
  const other = later === own ? frame : own;
  const url = later.url ?? other.url;
  // A later read says where its link was opened (a new link: nowhere yet).
  const opened = byRev ? later.opened : (later.opened ?? other.opened);
  const merged: SignInView = { ...later, ...(url ? { url } : {}) };
  if (opened) merged.opened = opened;
  return merged;
}

/** Our read of a sign-in moved on by `next` (an answer, or a read of
 * ours): `next`, the freshest word there is — unless it is an earlier read
 * of the same run (a lower `rev`: answers can cross). The link stays if it
 * left it out. */
export function advanceSignIn(own: SignInView | null | undefined, next: SignInView): SignInView {
  if (!own || own.startedAt !== next.startedAt) return next;
  if (own.rev !== undefined && next.rev !== undefined && own.rev > next.rev) return own;
  const url = next.url ?? own.url;
  return { ...next, ...(url ? { url } : {}) };
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
