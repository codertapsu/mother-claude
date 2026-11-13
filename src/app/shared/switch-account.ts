import { ApiError } from '../core/api.service';
import { computerNoun, tr } from '../core/i18n/i18n';
import { SessionDetail, SessionSummary } from '../core/models';
import { accountErrorText } from './accounts';
import { surfaceLabel } from './util';

/**
 * Switching a session's account (docs/ACCOUNTS.md §15.1): what a switch can
 * do for a session, the request and its answer, a switch waiting for the
 * end of a turn, the background tasks a switch stops, and the words.
 */

/** `here`: a session Mother Claude runs goes on under the other account;
 * `elsewhere`: another process runs it (a terminal, VS Code…), whose
 * account can't change; `later`: it doesn't run — its next continue runs
 * under the other account. */
export type SwitchMode = 'here' | 'elsewhere' | 'later';

/** `idle`: once the turn ends (the default); `now`: interrupt it first. */
export type SwitchWhen = 'idle' | 'now';

/** `POST /sessions/{id}/account`. */
export interface SwitchBody {
  account: string;
  when?: SwitchWhen;
}

/** Its answer: switched (`200 {id, account, moved}`), or waiting for the
 * turn to end (`202 {pending: true}`). */
export type SwitchReply =
  { pending: true } | { pending: false; id: string; account: string; moved: boolean; job?: string };

export function switchMode(s: Pick<SessionSummary, 'live' | 'owned'>): SwitchMode {
  if (!s.live) return 'later';
  return s.owned ? 'here' : 'elsewhere';
}

/** A conversation of the API (its summary's limit `bridge_thread`): its
 * account changes between turns only — no after-this-turn or now. */
export function isBridgeThread(s: Pick<SessionSummary, 'control'> | undefined): boolean {
  return !!s?.control?.limits?.some((l) => l?.code === 'bridge_thread');
}

/** In the middle of a turn: a switch then waits for it to end, or stops it. */
export function switchBusy(s: Pick<SessionSummary, 'live' | 'status' | 'activity'>): boolean {
  if (!s.live) return false;
  if (s.status === 'working' || s.status === 'starting') return true;
  return ['thinking', 'tool', 'delegating', 'compacting', 'retrying', 'starting'].includes(
    s.activity?.kind ?? '',
  );
}

/** The account a session switches to once its turn ends (an id, or an
 * account object), or null. */
export function switchingToOf(s: Pick<SessionSummary, 'switchingTo'> | undefined): string | null {
  const v = s?.switchingTo;
  if (typeof v === 'string') return v.trim() || null;
  if (v && typeof v === 'object') {
    const r = v as Record<string, unknown>;
    const id = r['id'] ?? r['account'];
    return typeof id === 'string' && id ? id : null;
  }
  return null;
}

/** The answer of `POST /sessions/{id}/account`, read tolerantly. */
export function normalizeSwitchReply(raw: unknown, asked: string): SwitchReply {
  const r = raw && typeof raw === 'object' ? (raw as Record<string, unknown>) : {};
  if (r['pending'] === true) return { pending: true };
  const job = typeof r['job'] === 'string' && r['job'] ? r['job'] : undefined;
  return {
    pending: false,
    id: typeof r['id'] === 'string' ? r['id'] : '',
    account: typeof r['account'] === 'string' ? r['account'] : asked,
    moved: r['moved'] === true,
    ...(job ? { job } : {}),
  };
}

/** The background tasks a switch stops with the process: the detail's
 * running ones when it is read, else the summary's count. */
export function runningTasks(
  s: Pick<SessionSummary, 'agents'>,
  detail?: Pick<SessionDetail, 'tasks'> | null,
): number {
  if (detail?.tasks) return detail.tasks.filter((t) => t.status === 'running').length;
  return s.agents?.backgroundRunning ?? 0;
}

/** "It runs in VS Code: its account can't change while it runs there." */
export function elsewhereText(s: Pick<SessionSummary, 'surface'>): string {
  return s.surface && s.surface !== 'unknown'
    ? tr("It runs in {app}: its account can't change while it runs there.", {
        app: surfaceLabel(s.surface),
      })
    : tr("It runs in another app: its account can't change while it runs there.");
}

/** What a switch that went through says. */
export function switchedText(name: string, moved: boolean): string {
  return moved
    ? tr('Now on {name}: the conversation moved to its folder.', { name })
    : tr('Now on {name}.', { name });
}

/** What everything the session is asked answers while its account
 * changes (`409 switching_account`). English: translated where it is shown
 * (`tr(SWITCHING_TEXT)`). */
export const SWITCHING_TEXT = 'Switching accounts — try again in a moment.';

/** A refusal of a switch, in words. */
export function switchErrorText(e: unknown, accountName?: string): string {
  if (e instanceof ApiError) {
    const name = accountName || tr('That account');
    switch (e.code) {
      case 'session_not_owned':
        return tr(
          "Another app runs this session, so its account can't change while it runs there: continue it on the other account instead.",
        );
      case 'turn_running':
        return tr(
          'This API conversation is in the middle of a turn: its account changes between turns. Try again once the turn is over.',
        );
      case 'account_not_served':
        return tr('Its API server runs only as its own account.');
      case 'switch_not_pending':
        return tr('No switch is waiting any more.');
      case 'switch_underway':
        return tr("The switch has started already: it can't be cancelled now.");
      case 'switching_account':
        return tr(SWITCHING_TEXT);
      case 'already_exists':
        return tr(
          '{name} already has a copy of this conversation: continue that copy there instead.',
          { name },
        );
      case 'data_job_running':
        return tr('A move, copy, backup or restore is running: try again once it has finished.');
      case 'dangerous_blocked':
        return tr(
          'This session runs without permission prompts: switching it needs dangerous actions for this device, granted on the computer in Settings › Phone & devices.',
        );
      case 'untrusted_cwd':
        return (
          e.message ||
          tr(
            '{name} has never opened this folder with Claude Code: open it there once, or switch from the {computer}.',
            { name, computer: computerNoun() },
          )
        );
      case 'not_supported':
        return e.message || tr("A fork that hasn't run yet can't move to another folder.");
      case 'switch_failed':
      case 'session_live':
      case 'invalid_when':
        return e.message || tr('The switch did not happen.');
      default:
        break;
    }
  }
  return accountErrorText(e, accountName);
}
