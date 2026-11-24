import { tr } from './i18n/i18n';

/**
 * The usage guard (Settings › Usage limits; `claude::usage_guard`): when an
 * account reaches the threshold of a plan window, Mother Claude pauses its
 * sessions' work, saves where each was, and resumes it a minute after the
 * window resets. `GET /api/usage-guard` answers this; `{t:'usageGuard'}`
 * says it changed.
 */

export interface UsageGuardSettings {
  enabled: boolean;
  /** The percentage of a window that pauses (50–99). */
  thresholdPct: number;
  /** Resume by itself after the reset. */
  autoResume: boolean;
  /** Seconds after the reset before resuming. */
  bufferSecs: number;
  /** Also sessions started elsewhere (through the plugin's hooks or tmux). */
  includeElsewhere: boolean;
}

export type UsageGuardPatch = Partial<UsageGuardSettings>;

/** A session a pause stopped. */
export interface PausedSession {
  id: string;
  name: string;
  owned: boolean;
  cwd?: string;
  /** A turn or background agents were stopped: it gets the resume message. */
  stopped: boolean;
  /** What stopped: the turn, agents, workflows. */
  doing: string[];
  /** Its todos then: [done, total]. */
  todos?: [number, number];
  /** Someone sent it a message while paused: left alone. */
  released: boolean;
  resumedAt?: number;
  resumeError?: string;
}

export type PauseEnded = 'resumed' | 'resumedByHand' | 'dismissed';

/** One window of one account paused. */
export interface UsagePause {
  id: string;
  account: string;
  accountName: string;
  /** `five_hour`, `seven_day`, `seven_day_<model>`. */
  window: string;
  label: string;
  pct: number;
  thresholdPct: number;
  pausedAt: number;
  resetsAt: number;
  /** When it resumes by itself (the reset plus the buffer). */
  resumeAt: number;
  /** A per-model window: only sessions on this model. */
  model?: string;
  sessions: PausedSession[];
  /** It does not resume by itself. */
  hold: boolean;
  endedAt?: number;
  ended?: PauseEnded;
}

export interface UsageGuardView {
  settings: UsageGuardSettings;
  /** Active pauses first, then the last few that ended. */
  pauses: UsagePause[];
}

/** `{t:'usageGuard'}`: it changed — refetch. */
export interface UsageGuardFrame {
  t: 'usageGuard';
}

export const THRESHOLD_MIN = 50;
export const THRESHOLD_MAX = 99;

export const DEFAULT_GUARD_SETTINGS: UsageGuardSettings = {
  enabled: true,
  thresholdPct: 90,
  autoResume: true,
  bufferSecs: 60,
  includeElsewhere: true,
};

const num = (v: unknown, fallback: number): number =>
  typeof v === 'number' && Number.isFinite(v) ? v : fallback;
const str = (v: unknown): string => (typeof v === 'string' ? v : '');
const list = (v: unknown): Record<string, unknown>[] =>
  Array.isArray(v)
    ? (v.filter((x) => x && typeof x === 'object') as Record<string, unknown>[])
    : [];

/** Read the server's answer tolerantly (a newer server may add more). */
export function normalizeUsageGuard(raw: unknown): UsageGuardView {
  const r = (raw && typeof raw === 'object' ? raw : {}) as Record<string, unknown>;
  const s = (r['settings'] && typeof r['settings'] === 'object' ? r['settings'] : {}) as Record<
    string,
    unknown
  >;
  const d = DEFAULT_GUARD_SETTINGS;
  return {
    settings: {
      enabled: typeof s['enabled'] === 'boolean' ? s['enabled'] : d.enabled,
      thresholdPct: num(s['thresholdPct'], d.thresholdPct),
      autoResume: typeof s['autoResume'] === 'boolean' ? s['autoResume'] : d.autoResume,
      bufferSecs: num(s['bufferSecs'], d.bufferSecs),
      includeElsewhere:
        typeof s['includeElsewhere'] === 'boolean' ? s['includeElsewhere'] : d.includeElsewhere,
    },
    pauses: list(r['pauses']).map((p) => ({
      id: str(p['id']),
      account: str(p['account']),
      accountName: str(p['accountName']) || str(p['account']),
      window: str(p['window']),
      label: str(p['label']),
      pct: num(p['pct'], 0),
      thresholdPct: num(p['thresholdPct'], d.thresholdPct),
      pausedAt: num(p['pausedAt'], 0),
      resetsAt: num(p['resetsAt'], 0),
      resumeAt: num(p['resumeAt'], 0),
      ...(str(p['model']) ? { model: str(p['model']) } : {}),
      sessions: list(p['sessions']).map((x) => ({
        id: str(x['id']),
        name: str(x['name']) || str(x['id']).slice(0, 8),
        owned: x['owned'] === true,
        ...(str(x['cwd']) ? { cwd: str(x['cwd']) } : {}),
        stopped: x['stopped'] === true,
        doing: Array.isArray(x['doing'])
          ? (x['doing'] as unknown[]).filter((d): d is string => typeof d === 'string')
          : [],
        ...(Array.isArray(x['todos']) && x['todos'].length === 2
          ? { todos: [num(x['todos'][0], 0), num(x['todos'][1], 0)] as [number, number] }
          : {}),
        released: x['released'] === true,
        ...(typeof x['resumedAt'] === 'number' ? { resumedAt: x['resumedAt'] } : {}),
        ...(str(x['resumeError']) ? { resumeError: str(x['resumeError']) } : {}),
      })),
      hold: p['hold'] === true,
      ...(typeof p['endedAt'] === 'number' ? { endedAt: p['endedAt'] } : {}),
      ...(['resumed', 'resumedByHand', 'dismissed'].includes(str(p['ended']))
        ? { ended: str(p['ended']) as PauseEnded }
        : {}),
    })),
  };
}

/** The window as people say it: "the 5-hour limit", "the weekly Opus limit". */
export function windowText(key: string, label: string): string {
  if (key === 'five_hour') return tr('the 5-hour limit');
  if (key === 'seven_day') return tr('the weekly limit');
  const model = label.split('·')[1]?.trim();
  return model ? tr('the weekly {model} limit', { model }) : tr('its limit');
}

/** How many sessions a pause stopped. */
export function stoppedCount(p: UsagePause): number {
  return p.sessions.filter((s) => s.stopped).length;
}
