import { ApiError, describeError } from '../core/api.service';
import { isTransferRefusal, transferErrorText } from '../core/data-jobs';
import { AccountView, CaptureView, CopyConsent, PlanUsage, SecretStoreKind } from '../core/models';
import { compactWindows, limitName, limitWindow, windowPct } from '../core/plan-usage';
import { Tone, capitalize, resetsIn } from './util';

/**
 * What the account views say about an account: its kind, its sign-in, its
 * plan usage in one line, and the refusals a launch or a continue may get.
 */

/** A token account's plan usage before any of its sessions reported it
 * (its token is inference-only: it can't read the plan by itself). */
export const TOKEN_NO_USAGE =
  'Its plan usage shows once a session runs under it: a token cannot read the plan on its own.';

/** "Own directory" / "Token, shares the Default's data". */
export function accountKindLabel(a: Pick<AccountView, 'kind'>): string {
  return a.kind === 'token' ? "Token, shares the Default's data" : 'Own directory';
}

const SUBSCRIPTIONS: Record<string, string> = {
  max: 'Max',
  pro: 'Pro',
  team: 'Team',
  enterprise: 'Enterprise',
  free: 'Free',
};

/** `max` → "Max"; '' when unknown. */
export function subscriptionLabel(s: string | undefined): string {
  if (!s) return '';
  return SUBSCRIPTIONS[s.toLowerCase()] ?? capitalize(s);
}

export interface SignInLine {
  text: string;
  tone: Tone;
}

/** The account's sign-in as one line: who it is signed in as, or what is
 * wrong (the attention reason). */
export function signInLine(a: AccountView): SignInLine {
  if (a.signingIn) return { text: 'Waiting for the sign-in in the browser…', tone: 'blue' };
  switch (a.attention) {
    case 'tokenMissing':
      return { text: 'No token on this Mac yet', tone: 'amber' };
    case 'authFailed':
      return {
        text:
          a.kind === 'token'
            ? 'Its token was refused: get a new one'
            : 'A session could not sign in: sign in again',
        tone: 'red',
      };
    case 'signedOut':
      return { text: 'Signed out', tone: 'amber' };
    case 'checkFailed':
      return {
        text: a.authError
          ? `Couldn't check the sign-in: ${a.authError}`
          : "Couldn't check the sign-in",
        tone: 'amber',
      };
    default:
      break;
  }
  if (a.checking && !a.signedIn) return { text: 'Checking the sign-in…', tone: 'muted' };
  if (a.signedIn) {
    const parts = [a.email, subscriptionLabel(a.subscription)].filter(Boolean);
    return { text: parts.length ? parts.join(' · ') : 'Signed in', tone: 'green' };
  }
  return { text: 'Sign-in not checked yet', tone: 'muted' };
}

/** Why nothing can run under the account right now ('' when it can). */
export function cannotRunReason(a: Pick<AccountView, 'attention'>): string {
  if (a.attention === 'tokenMissing') return 'no token on this Mac yet';
  if (a.attention === 'signedOut') return 'signed out';
  return '';
}

/** Where the Mac keeps account tokens, as a phrase. */
export function secretStoreText(kind: SecretStoreKind | undefined): string {
  switch (kind) {
    case 'keychain':
      return 'the macOS Keychain';
    case 'files':
      return 'files in the app data folder that only your user can read';
    case 'memory':
      return 'memory only (they are forgotten when Mother Claude quits)';
    default:
      return '';
  }
}

/** An account's plan usage in one line for compact places: "Session 42% ·
 * Week 18%", "5-hour limit reached · resets in 2h", or '' when there is
 * nothing to show. */
export function usageLine(u: PlanUsage | null | undefined, now: number): string {
  if (!u?.available) return '';
  const limit = limitWindow(u.windows, now);
  if (limit) {
    const left = resetsIn(limit.resetsAt, now);
    return `${capitalize(limitName(limit))} reached${left ? ` · ${left}` : ''}`;
  }
  return compactWindows(u.windows, now)
    .map(({ window: w, short }) => `${short} ${windowPct(w, now)}%`)
    .join(' · ');
}

/** What a finished `setup-token` run that brought no token says. */
export function captureProblem(c: CaptureView | null | undefined): string {
  switch (c?.state) {
    case 'failed':
      return c.error
        ? `The sign-in ended without a token: ${c.error}`
        : 'The sign-in ended without a token.';
    case 'timedOut':
      return 'No token arrived within five minutes.';
    case 'cancelled':
      return 'The sign-in was cancelled.';
    default:
      return '';
  }
}

/** A refusal of a launch or a continue that names an account, as a
 * sentence (the server's own words where they help). */
export function accountErrorText(e: unknown, accountName?: string): string {
  if (e instanceof ApiError) {
    switch (e.code) {
      // An older server, before "continue on" could copy.
      case 'account_needs_copy':
        return (
          e.message ||
          'That account keeps its own directory: the conversation has to be copied there first.'
        );
      case 'copy_consent_required':
        return `${accountName || 'That account'} keeps its own directory, so the conversation is copied there first, and copies between these two accounts have to be allowed first (Accounts page, Copy for Continue on).`;
      case 'account_token_missing':
        return `${accountName || 'That account'} has no token on this Mac yet: add it on the Accounts page of the desktop app.`;
      case 'unknown_account':
        return 'That account no longer exists: pick another one.';
      default:
        break;
    }
  }
  return isTransferRefusal(e) ? transferErrorText(e) : describeError(e).text;
}

/** A session belongs to the account an account filter picked ('' = every
 * account). A session that names no account matches only "every account". */
export function inAccount(s: { account?: { id: string } } | undefined, filter: string): boolean {
  return !filter || s?.account?.id === filter;
}

/** The pair of accounts a continue-on copy waits for the desktop to allow
 * (409 `copy_consent_required` names them), or null for any other error. */
export function consentNeeded(e: unknown): CopyConsent | null {
  if (!(e instanceof ApiError) || e.code !== 'copy_consent_required') return null;
  const from = e.detail('from');
  const to = e.detail('to');
  return from && to ? { from, to } : null;
}

/** The hooks plugin of an account, in a few words ('' for none to show: a
 * token account uses the Default's). */
export function hooksPluginText(a: Pick<AccountView, 'kind' | 'hooksPlugin'>): string {
  if (a.kind === 'token') return '';
  switch (a.hooksPlugin) {
    case 'installed':
      return 'Hooks plugin installed';
    case 'off':
      return 'Hooks plugin switched off';
    case 'missing':
      return 'Hooks plugin not installed';
    case 'unknown':
      return "Hooks plugin: couldn't check";
    default:
      return '';
  }
}
