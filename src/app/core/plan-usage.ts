import { PlanExtraUsage, PlanSeverity, PlanUsage, PlanWindow } from './models';

/**
 * Pure helpers for the Claude plan's usage (`GET /usage`, `planUsage`
 * frames). The numbers come from an experimental Claude Code request, so
 * everything read from the wire is checked here before the UI sees it.
 */

/** Refresh intervals the server accepts (seconds), as Settings offers them. */
export const PLAN_INTERVALS: readonly { secs: number; label: string }[] = [
  { secs: 180, label: 'Every 3 min' },
  { secs: 300, label: 'Every 5 min (recommended)' },
  { secs: 600, label: 'Every 10 min' },
  { secs: 0, label: 'Manually only' },
];

export const DEFAULT_PLAN_INTERVAL = 300;

/** "Every 5 min", "Manually only", or "Every 7 min" for a value this app
 * doesn't offer. */
export function planIntervalLabel(secs: number | undefined): string {
  const s = typeof secs === 'number' && Number.isFinite(secs) ? secs : DEFAULT_PLAN_INTERVAL;
  const known = PLAN_INTERVALS.find((o) => o.secs === s);
  if (known) return known.label.replace(' (recommended)', '');
  return s <= 0 ? 'Manually only' : `Every ${Math.max(1, Math.round(s / 60))} min`;
}

const SEVERITIES: readonly PlanSeverity[] = ['normal', 'warning', 'critical'];
const SOURCES: readonly NonNullable<PlanUsage['source']>[] = ['live', 'stream', 'cache'];

function num(v: unknown): number | undefined {
  return typeof v === 'number' && Number.isFinite(v) ? v : undefined;
}

function str(v: unknown): string | undefined {
  return typeof v === 'string' && v.trim() ? v : undefined;
}

function normalizeWindow(raw: unknown): PlanWindow | null {
  if (!raw || typeof raw !== 'object') return null;
  const r = raw as Record<string, unknown>;
  const pct = num(r['pct']);
  const key = str(r['key']);
  if (pct === undefined || !key) return null;
  const severity = SEVERITIES.find((s) => s === r['severity']);
  const resetsAt = num(r['resetsAt']);
  return {
    key,
    label: str(r['label']) ?? key,
    pct: Math.max(0, pct),
    ...(resetsAt ? { resetsAt } : {}),
    ...(severity ? { severity } : {}),
  };
}

function normalizeExtra(raw: unknown): PlanExtraUsage | undefined {
  if (!raw || typeof raw !== 'object') return undefined;
  const r = raw as Record<string, unknown>;
  const decimals = num(r['decimals']);
  const limitMinor = num(r['limitMinor']);
  const utilization = num(r['utilization']);
  // No currency is no currency: never guess one for an amount.
  const currency = str(r['currency']);
  return {
    enabled: r['enabled'] === true,
    usedMinor: Math.max(0, num(r['usedMinor']) ?? 0),
    ...(limitMinor !== undefined ? { limitMinor } : {}),
    ...(currency ? { currency } : {}),
    // Amounts are divided by 10^decimals: a value outside what the server
    // accepts (a whole 0–6) is dropped, never clamped.
    decimals:
      decimals !== undefined && Number.isInteger(decimals) && decimals >= 0 && decimals <= 6
        ? decimals
        : 2,
    ...(utilization !== undefined ? { utilization } : {}),
  };
}

/** A plan usage from the wire made safe to render, or null when it isn't
 * one at all. Malformed windows are dropped rather than shown as 0 %. */
export function normalizePlanUsage(raw: unknown): PlanUsage | null {
  if (!raw || typeof raw !== 'object') return null;
  const r = raw as Record<string, unknown>;
  const windows = Array.isArray(r['windows'])
    ? r['windows'].map(normalizeWindow).filter((w): w is PlanWindow => !!w)
    : [];
  const interval = num(r['intervalSecs']);
  const extraUsage = normalizeExtra(r['extraUsage']);
  const fetchedAt = num(r['fetchedAt']);
  const streamAt = num(r['streamAt']);
  const nextRefreshAt = num(r['nextRefreshAt']);
  const source = SOURCES.find((s) => s === r['source']);
  const error = str(r['error']);
  return {
    available: r['available'] === true,
    windows,
    ...(extraUsage ? { extraUsage } : {}),
    ...(fetchedAt ? { fetchedAt } : {}),
    ...(source ? { source } : {}),
    ...(streamAt ? { streamAt } : {}),
    refreshing: r['refreshing'] === true,
    ...(nextRefreshAt ? { nextRefreshAt } : {}),
    intervalSecs: interval !== undefined && interval >= 0 ? interval : DEFAULT_PLAN_INTERVAL,
    ...(error ? { error } : {}),
  };
}

/** The window's reset time has passed: a new window starts with the next
 * message, so it counts as 0 % until the server says otherwise. */
export function hasReset(w: PlanWindow, now: number): boolean {
  return !!w.resetsAt && w.resetsAt <= now;
}

/** Whole percent used right now (0 after a reset; may pass 100). */
export function windowPct(w: PlanWindow, now: number): number {
  if (hasReset(w, now)) return 0;
  return Math.max(0, Math.round(w.pct));
}

/** How close to the limit: the server's severity or the percentage
 * (warning from 75 %, critical from 90 %), whichever is worse. */
export function windowSeverity(w: PlanWindow, now: number): PlanSeverity {
  const pct = windowPct(w, now);
  const reset = hasReset(w, now);
  if ((!reset && w.severity === 'critical') || pct >= 90) return 'critical';
  if ((!reset && w.severity === 'warning') || pct >= 75) return 'warning';
  return 'normal';
}

/** The worst severity across windows. */
export function worstSeverity(windows: readonly PlanWindow[], now: number): PlanSeverity {
  let worst: PlanSeverity = 'normal';
  for (const w of windows) {
    const s = windowSeverity(w, now);
    if (s === 'critical') return s;
    if (s === 'warning') worst = s;
  }
  return worst;
}

function isSession(w: PlanWindow): boolean {
  return /^(session|five_hour)/i.test(w.key) || /\bsession\b|5.hour/i.test(w.label);
}

function isWeeklyAll(w: PlanWindow): boolean {
  return /^(weekly_all|seven_day)$/i.test(w.key) || /^weekly( · all( models)?)?$/i.test(w.label);
}

function isWeekly(w: PlanWindow): boolean {
  return /week|seven_day/i.test(w.key) || /week/i.test(w.label);
}

export interface PrimaryWindow {
  window: PlanWindow;
  /** "Session", "Week", or a shortened label. */
  short: string;
}

/** The two windows a compact view shows: the current session (5-hour) and
 * the weekly all-models window; else the first two the server listed. */
export function primaryWindows(windows: readonly PlanWindow[]): PrimaryWindow[] {
  const session = windows.find(isSession);
  const week =
    windows.find((w) => w !== session && isWeeklyAll(w)) ??
    windows.find((w) => w !== session && isWeekly(w));
  const out: PrimaryWindow[] = [];
  if (session) out.push({ window: session, short: 'Session' });
  if (week) out.push({ window: week, short: 'Week' });
  if (out.length) return out;
  return windows.slice(0, 2).map((w) => ({ window: w, short: shortWindowLabel(w.label) }));
}

/** What a compact view shows: the primary windows, plus any other window
 * near or at its limit (a busy model-specific week must not hide behind a
 * calm "Week"), named by its scope ("Weekly · Fable" → "Fable"). */
export function compactWindows(windows: readonly PlanWindow[], now: number): PrimaryWindow[] {
  const primary = primaryWindows(windows);
  const shown = new Set(primary.map((p) => p.window));
  const urgent = windows
    .filter((w) => !shown.has(w) && windowSeverity(w, now) !== 'normal')
    .map((w) => ({ window: w, short: scopeLabel(w.label) }));
  return [...primary, ...urgent];
}

function shortWindowLabel(label: string): string {
  return clip(label.split('·')[0].trim() || label);
}

function scopeLabel(label: string): string {
  const parts = label
    .split('·')
    .map((p) => p.trim())
    .filter(Boolean);
  return clip(parts[parts.length - 1] ?? label);
}

function clip(s: string): string {
  return s.length > 14 ? `${s.slice(0, 13)}…` : s;
}
