import { ApiError, describeError } from '../core/api.service';
import { isTransferRefusal, transferErrorText } from '../core/data-jobs';
import { computerNoun, tr } from '../core/i18n/i18n';
import { AccountView, CaptureView, CopyConsent, PlanUsage, SecretStoreKind } from '../core/models';
import { compactWindows, limitName, limitWindow, windowPct } from '../core/plan-usage';
import { Tone, capitalize, resetsIn } from './util';

/**
 * What the account views say about an account: its kind, its sign-in, its
 * plan usage in one line, and the refusals a launch or a continue may get.
 */

/** A token account's plan usage before any of its sessions reported it
 * (its token is inference-only: it can't read the plan by itself). English:
 * translated where it is shown (`tr(TOKEN_NO_USAGE)`). */
export const TOKEN_NO_USAGE =
  'Its plan usage shows once a session runs under it: a token cannot read the plan on its own.';

/** "Own directory" / "Token, shares the Default's data". */
export function accountKindLabel(a: Pick<AccountView, 'kind'>): string {
  return a.kind === 'token' ? tr("Token, shares the Default's data") : tr('Own directory');
}

/** The plans' own names (never translated). */
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
  if (a.signingIn) return { text: tr('Waiting for the sign-in in the browser…'), tone: 'blue' };
  switch (a.attention) {
    case 'tokenMissing':
      return {
        text: tr('No token on this {computer} yet', { computer: computerNoun() }),
        tone: 'amber',
      };
    case 'authFailed':
      return {
        text:
          a.kind === 'token'
            ? tr('Its token was refused: get a new one')
            : tr('A session could not sign in: sign in again'),
        tone: 'red',
      };
    case 'signedOut':
      return { text: tr('Signed out'), tone: 'amber' };
    case 'checkFailed':
      return {
        text: a.authError
          ? tr("Couldn't check the sign-in: {error}", { error: a.authError })
          : tr("Couldn't check the sign-in"),
        tone: 'amber',
      };
    default:
      break;
  }
  if (a.checking && !a.signedIn) return { text: tr('Checking the sign-in…'), tone: 'muted' };
  if (a.signedIn) {
    const parts = [a.email, subscriptionLabel(a.subscription)].filter(Boolean);
    return { text: parts.length ? parts.join(' · ') : tr('Signed in'), tone: 'green' };
  }
  return { text: tr('Sign-in not checked yet'), tone: 'muted' };
}

/** Why nothing can run under the account right now ('' when it can). */
export function cannotRunReason(a: Pick<AccountView, 'attention'>): string {
  if (a.attention === 'tokenMissing') {
    return tr('no token on this {computer} yet', { computer: computerNoun() });
  }
  if (a.attention === 'signedOut') return tr('signed out');
  return '';
}

/** Where the desktop's computer keeps account tokens, as a phrase. */
export function secretStoreText(kind: SecretStoreKind | undefined): string {
  switch (kind) {
    case 'keychain':
      return tr('the macOS Keychain');
    case 'credentialManager':
      return tr('Windows Credential Manager');
    case 'files':
      return tr('files in the app data folder that only your user can read');
    case 'memory':
      return tr('memory only (they are forgotten when Mother Claude quits)');
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
    // Capitalized as a whole: the limit's name may stand mid-sentence.
    const reached = capitalize(tr('{limit} reached', { limit: limitName(limit) }));
    return left ? `${reached} · ${left}` : reached;
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
        ? tr('The sign-in ended without a token: {error}', { error: c.error })
        : tr('The sign-in ended without a token.');
    case 'timedOut':
      return tr('No token arrived within five minutes.');
    case 'cancelled':
      return tr('The sign-in was cancelled.');
    default:
      return '';
  }
}

/** A refusal of a launch or a continue that names an account, as a
 * sentence (the server's own words where they help). */
export function accountErrorText(e: unknown, accountName?: string): string {
  if (e instanceof ApiError) {
    const name = accountName || tr('That account');
    switch (e.code) {
      // An older server, before "continue on" could copy.
      case 'account_needs_copy':
        return (
          e.message ||
          tr('That account keeps its own directory: the conversation has to be copied there first.')
        );
      case 'copy_consent_required':
        return tr(
          '{name} keeps its own directory, so the conversation is copied there first, and copies between these two accounts have to be allowed first (Accounts page, Copy for Continue on).',
          { name },
        );
      case 'account_token_missing':
        return tr(
          '{name} has no token on this {computer} yet: add it on the Accounts page of the desktop app.',
          { name, computer: computerNoun() },
        );
      case 'unknown_account':
        return tr('That account no longer exists: pick another one.');
      default:
        break;
    }
  }
  return isTransferRefusal(e) ? transferErrorText(e) : describeError(e).text;
}

/** A refusal of a sign-in, in words (`nameOf` names the account another
 * sign-in runs for: one runs at a time, docs/ACCOUNTS.md §15.3). */
export function signInErrorText(
  e: unknown,
  nameOf: (id: string) => string | undefined = () => undefined,
): string {
  if (e instanceof ApiError) {
    switch (e.code) {
      case 'sign_in_running': {
        const id = e.detail('account');
        const name = (id && nameOf(id)) || e.detail('accountName');
        return name
          ? tr(
              "{name}'s sign-in is running: one sign-in runs at a time. Finish or cancel it first.",
              { name },
            )
          : tr(
              "Another account's sign-in is running: one sign-in runs at a time. Finish or cancel it first.",
            );
      }
      case 'sign_in_not_running':
        return tr('That sign-in is not running any more.');
      case 'sign_in_no_link':
        return tr("The sign-in hasn't shown its link yet: try again in a moment.");
      case 'browser_unavailable':
        return tr(
          "Google Chrome isn't on this {computer}: open the link in another browser, or copy it.",
          { computer: computerNoun() },
        );
      case 'browser_failed':
        return tr("The browser couldn't be opened: copy the link and open it yourself.");
      case 'invalid_sign_in_url':
        return tr(
          'The sign-in showed a link Mother Claude does not open: cancel it and start again.',
        );
      case 'sign_in_unavailable':
        return tr(
          "This build can't run the sign-in itself: sign in in a terminal, or paste a token.",
        );
      default:
        break;
    }
  }
  return describeError(e).text;
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
      return tr('Hooks plugin installed');
    case 'off':
      return tr('Hooks plugin switched off');
    case 'missing':
      return tr('Hooks plugin not installed');
    case 'unknown':
      return tr("Hooks plugin: couldn't check");
    default:
      return '';
  }
}
