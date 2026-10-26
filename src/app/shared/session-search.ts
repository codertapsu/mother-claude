import { SessionsQuery, SessionSummary } from '../core/models';
import { inAccount } from './accounts';
import { thinkingChip } from './session-settings';
import { effortLabel, effortShortLabel } from './util';

/**
 * The session list's filters — the search box, the surface, project and
 * account pickers, and the All / Live / Needs you switch — as one value the
 * page applies to what it holds and sends to `GET /sessions` for the rest
 * (earlier sessions it has not read; *Select all*). The server matches the
 * same fields (`SessionSummary::matches_search`).
 */
export interface SessionFilters {
  /** The search, as typed (matched trimmed, in any case). */
  q: string;
  surface: string;
  project: string;
  /** An account id ('' = every account). */
  account: string;
  /** Only running sessions, or only those waiting on someone. */
  only: '' | 'live' | 'needs';
}

/** Waiting on a person: a question or an approval ("Needs you"). */
export function needsYou(s: SessionSummary): boolean {
  return s.status === 'waiting' || (s.pending?.length ?? 0) > 0;
}

/** The search box: `q` (lower case) appears in anything the card shows
 * about the session, or in its id, folder or last prompt. */
export function sessionMatches(s: SessionSummary, q: string): boolean {
  return [
    s.title,
    s.name,
    s.projectName,
    s.cwd,
    s.gitBranch,
    s.lastPrompt,
    s.model?.label,
    s.model?.id,
    s.effort,
    effortLabel(s.effort),
    // What the card's chips say ("X-high", "Thinking off").
    effortShortLabel(s.effort),
    thinkingChip(s)?.text,
    s.ultracode === 'on' ? 'ultracode' : '',
    s.fast ? 'fast' : '',
    s.id,
  ]
    .filter(Boolean)
    .some((v) => String(v).toLowerCase().includes(q));
}

/** Whether a session passes every filter. */
export function sessionPasses(s: SessionSummary, f: SessionFilters): boolean {
  if (f.surface && s.surface !== f.surface) return false;
  if (f.project && s.projectName !== f.project) return false;
  if (!inAccount(s, f.account)) return false;
  if (f.only === 'live' && !s.live) return false;
  if (f.only === 'needs' && !needsYou(s)) return false;
  const q = f.q.trim().toLowerCase();
  return !q || sessionMatches(s, q);
}

/** Any filter narrows the list (Select all then says "matching"). */
export function filtersActive(f: SessionFilters): boolean {
  return !!(f.q.trim() || f.surface || f.project || f.account || f.only);
}

/** The filters as `GET /sessions` fields (blank ones are left out). */
export function filterQuery(f: SessionFilters): SessionsQuery {
  return {
    q: f.q.trim() || undefined,
    surface: f.surface || undefined,
    project: f.project || undefined,
    account: f.account || undefined,
    only: f.only || undefined,
  };
}
