import { ApiError } from '../core/api.service';
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
  { pending: true } | { pending: false; id: string; account: string; moved: boolean };

export function switchMode(s: Pick<SessionSummary, 'live' | 'owned'>): SwitchMode {
  if (!s.live) return 'later';
  return s.owned ? 'here' : 'elsewhere';
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
  return {
    pending: false,
    id: typeof r['id'] === 'string' ? r['id'] : '',
    account: typeof r['account'] === 'string' ? r['account'] : asked,
    moved: r['moved'] === true,
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
  const where = s.surface && s.surface !== 'unknown' ? surfaceLabel(s.surface) : 'another app';
  return `It runs in ${where}: its account can't change while it runs there.`;
}

/** What a switch that went through says. */
export function switchedText(name: string, moved: boolean): string {
  return moved ? `Now on ${name}: the conversation moved to its folder.` : `Now on ${name}.`;
}

/** A refusal of a switch, in words. */
export function switchErrorText(e: unknown, accountName?: string): string {
  if (e instanceof ApiError) {
    switch (e.code) {
      case 'session_not_owned':
        return "Another app runs this session, so its account can't change while it runs there: continue it on the other account instead.";
      case 'turn_running':
        return 'This API conversation is in the middle of a turn: its account changes between turns. Try again once the turn is over.';
      case 'account_not_served':
        return 'Its API server runs only as its own account.';
      case 'switch_not_pending':
        return 'No switch is waiting any more.';
      default:
        break;
    }
  }
  return accountErrorText(e, accountName);
}
