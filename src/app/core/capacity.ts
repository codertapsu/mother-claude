import { PlanWindow } from './models';

/**
 * The scheduler side's API (docs/ROUTING.md): host metrics, admission
 * control, budgets, the routing policy and the reports, as the server
 * answers `GET /api/host`, `/api/capacity`, `/api/routing/policy` and
 * `/api/routing/report`. Pure helpers for the Capacity page live here too.
 */

export interface ModelChoice {
  model: string;
  effort?: string;
}

export interface SessionMemory {
  sessionId: string;
  pid: number;
  rssBytes: number;
}

/** `GET /api/host`: the last sample and the per-session estimate. */
export interface HostView {
  /** 0 until the first sample. */
  sampledAt: number;
  totalMemory: number;
  /** What a new process can get without swapping (shown as "free"). */
  availableMemory: number;
  usedMemory: number;
  cpus: number;
  /** 1, 5 and 15 minutes; absent on Windows. */
  load?: [number, number, number];
  sessions: SessionMemory[];
  estimateBytes: number;
  measured: boolean;
  reserveBytes: number;
  headroom?: number;
}

export interface CapsView {
  userCap: number;
  projectCap: number;
  workersRunning: number;
  leadsRunning: number;
  hardwareCap?: number;
}

export type Billing = 'perToken' | 'plan';

export interface BillingFacts {
  billing?: Billing;
  source: string;
}

export interface RateLimitNote {
  at: number;
  account: string;
  /** `allowed`, `allowed_warning` or `rejected`. */
  status: string;
  limit?: string;
  resetsAt?: number;
  pct?: number;
  source: string;
}

export interface AccountSignals {
  id: string;
  name: string;
  billing: BillingFacts;
  planUsage: boolean;
  windows: PlanWindow[];
  leadOnly: boolean;
  stopped: boolean;
  opusGated: boolean;
  rateLimit?: RateLimitNote;
}

export interface RoleSession {
  sessionId: string;
  project: string;
  role: string;
  model: string;
  lead: boolean;
  firstSeen: number;
  lastSeen: number;
}

export interface RunningView extends RoleSession {
  live?: boolean;
  spentUsd?: number;
  rssBytes?: number;
}

/** The last admission decision for a (project, role). */
export interface QueueEntry {
  project: string;
  role: string;
  model: string;
  priority: number;
  lead: boolean;
  account?: string;
  admitted: boolean;
  reasons: string[];
  waitingSince?: number;
  at: number;
  asks: number;
}

export type BudgetScope = 'project' | 'session' | 'task';
export type BudgetLevel = 'ok' | 'near' | 'over';

export interface BudgetState {
  scope: BudgetScope;
  key: string;
  label: string;
  project?: string;
  role?: string;
  sessionId?: string;
  spentUsd: number;
  limitUsd: number;
  pct: number;
  level: BudgetLevel;
  applies: boolean;
  note?: string;
}

export interface RouteNote {
  at: number;
  project: string;
  role?: string;
  class?: string;
  choice: ModelChoice;
  reason: string;
}

export interface Budgets {
  projectDailyUsd: number | null;
  projects: Record<string, number>;
  taskUsd: number | null;
  sessionUsd: number | null;
  roles: Record<string, number>;
  nearPct: number;
}

/** The scheduler's settings (`settings` of the Capacity view). */
export interface CapacitySettings {
  userCap: number;
  projectCap: number;
  projectCaps: Record<string, number>;
  reserveMb: number;
  sessionMb: number | null;
  leadOnlyPct: number;
  stopPct: number;
  opusGatePct: number;
  budgets: Budgets;
  alerts: { desktop: boolean; push: boolean };
  haikuClassifier: boolean;
  billing: Record<string, Billing>;
}

export interface ClassifierStats {
  calls: number;
  failures: number;
  inputTokens: number;
  outputTokens: number;
  costUsd: number;
  lastError?: string;
}

export interface ClassifierView {
  enabled: boolean;
  ready: boolean;
  whyNot?: string;
  model: string;
  stats: ClassifierStats;
}

/** `GET /api/capacity`. */
export interface CapacityView {
  now: number;
  host: HostView;
  caps: CapsView;
  accounts: AccountSignals[];
  rateLimits: RateLimitNote[];
  running: RunningView[];
  queue: QueueEntry[];
  budgets: BudgetState[];
  routes: RouteNote[];
  settings: CapacitySettings;
  classifier: ClassifierView;
  canManage: boolean;
  canEditPolicy: boolean;
}

/** A partial settings update: `null` turns an optional cap off; maps are
 * replaced whole. */
export interface SettingsPatch {
  userCap?: number;
  projectCap?: number;
  projectCaps?: Record<string, number>;
  reserveMb?: number;
  sessionMb?: number | null;
  leadOnlyPct?: number;
  stopPct?: number;
  opusGatePct?: number;
  budgets?: Partial<Budgets>;
  alerts?: Partial<CapacitySettings['alerts']>;
  haikuClassifier?: boolean;
  billing?: Record<string, Billing>;
}

export interface RoutingPolicy {
  classes: Record<string, ModelChoice>;
  ladder: ModelChoice[];
  fableOnEscalation: boolean;
  gatesPerStep: number;
  cleanToStepDown: number;
  gateChoice: ModelChoice;
  fallbacks: Record<string, string>;
}

export interface ClassView {
  project: string;
  class: string;
  level: number;
  clean: number;
}

export interface EscalationNote {
  at: number;
  taskId: string;
  project: string;
  class?: string;
  direction: 'up' | 'down';
  steps: number;
  reason: string;
}

/** `GET /api/routing/policy`. */
export interface PolicyView {
  policy: RoutingPolicy;
  defaults: RoutingPolicy;
  isDefault: boolean;
  classes: { class: string; label: string }[];
  models: string[];
  efforts: string[];
  learned: ClassView[];
  escalations: EscalationNote[];
  canEdit: boolean;
}

export interface ReportRow {
  class: string;
  model: string;
  effort: string;
  tasks: number;
  completed: number;
  failed: number;
  escalated: number;
  cancelled: number;
  withUsage: number;
  costUsd: number;
  costPerCompletedUsd?: number;
  tokens: number;
  tokensPerCompleted?: number;
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  cacheHitRatio?: number;
  gates: number;
  gatesPassed: number;
  gatePassRate?: number;
  escalationRate?: number;
  windowPct?: number;
  windowPctPerCompleted?: number;
  minutesPerCompleted?: number;
  perToken: number;
}

/** `GET /api/routing/report`. */
export interface RoutingReport {
  generatedAt: number;
  since: number;
  days: number;
  project?: string;
  rows: ReportRow[];
  classes: ReportRow[];
  totals: ReportRow;
  escalations: EscalationNote[];
  learned: ClassView[];
  classifier: ClassifierStats;
  pending: number;
  estimateNote: string;
}

export interface Classification {
  class: string;
  source: 'rules' | 'haiku';
  reason: string;
}

// ──────────────────────────────────────────────────────────── Helpers

export const EFFORTS = ['low', 'medium', 'high', 'xhigh', 'max'] as const;
export const MODELS = ['haiku', 'sonnet', 'opus', 'fable'] as const;
export const REPORT_DAYS = [7, 30, 90] as const;

const MIB = 1024 * 1024;
const GIB = 1024 * MIB;

/** "1.1 GB", "600 MB". */
export function bytesLabel(b: number | undefined): string {
  if (b === undefined || !Number.isFinite(b) || b < 0) return '—';
  return b >= GIB ? `${(b / GIB).toFixed(1)} GB` : `${Math.round(b / MIB)} MB`;
}

/** "91 %", "95.5 %". */
export function pctLabel(p: number | undefined): string {
  if (p === undefined || !Number.isFinite(p)) return '—';
  return `${Number.isInteger(p) ? p : p.toFixed(1)} %`;
}

/** A ratio (0–1) as a percentage: "83 %". */
export function ratioLabel(r: number | undefined): string {
  if (r === undefined || !Number.isFinite(r)) return '—';
  return `${Math.round(r * 100)} %`;
}

export function budgetTone(level: BudgetLevel): 'green' | 'amber' | 'red' {
  return level === 'over' ? 'red' : level === 'near' ? 'amber' : 'green';
}

/** The family of an alias or a full id. */
export function family(model: string): string | undefined {
  const m = model.trim().toLowerCase();
  if (m.includes('fable') || m.includes('mythos')) return 'fable';
  if (m.includes('opus')) return 'opus';
  if (m.includes('sonnet')) return 'sonnet';
  if (m.includes('haiku')) return 'haiku';
  return undefined;
}

/** Tier of a model: haiku 0, sonnet 1, opus 2, fable 3 (unknown: 1). */
export function tier(model: string): number {
  const f = family(model);
  return f === 'haiku' ? 0 : f === 'opus' ? 2 : f === 'fable' ? 3 : 1;
}

/** Rank of an effort (none: the model's default, ranked as medium). */
export function effortRank(effort: string | undefined): number {
  const i = effort ? (EFFORTS as readonly string[]).indexOf(effort) : -1;
  return i < 0 ? 1 : i;
}

/** Tier first, then effort, as one number. */
export function rank(c: ModelChoice): number {
  return tier(c.model) * 10 + effortRank(c.effort);
}

/** "Opus · high", "Sonnet". */
export function choiceLabel(c: ModelChoice | undefined): string {
  if (!c) return '—';
  const alias = (MODELS as readonly string[]).includes(c.model);
  const model = alias ? c.model[0].toUpperCase() + c.model.slice(1) : c.model;
  return c.effort ? `${model} · ${c.effort}` : model;
}

/** A model Claude Code takes: an alias or a full `claude-…` id. */
export function validModel(m: string): boolean {
  if ((MODELS as readonly string[]).includes(m)) return true;
  const core = m.endsWith('[1m]') ? m.slice(0, -4) : m;
  return core.length > 'claude-'.length && core.length <= 64 && /^claude-[a-z0-9.-]+$/.test(core);
}

/** What is wrong with a policy, by field (`classes.plan`, `ladder`, …);
 * empty when the server would take it. */
export function policyProblems(p: RoutingPolicy): Record<string, string> {
  const out: Record<string, string> = {};
  const choiceProblem = (c: ModelChoice): string | null => {
    if (!validModel(c.model)) return `${c.model || 'Nothing'} is not a model.`;
    if (c.effort && !(EFFORTS as readonly string[]).includes(c.effort)) {
      return `${c.effort} is not an effort.`;
    }
    return null;
  };
  for (const [k, c] of Object.entries(p.classes)) {
    const problem = choiceProblem(c);
    if (problem) out[`classes.${k}`] = problem;
  }
  if (p.ladder.length < 2 || p.ladder.length > 12) {
    out['ladder'] = 'The ladder has 2 to 12 rungs.';
  } else if (p.ladder.some((r) => choiceProblem(r))) {
    out['ladder'] = 'Every rung needs a model and an effort Claude Code takes.';
  } else if (p.ladder.some((r, i) => i > 0 && rank(r) <= rank(p.ladder[i - 1]))) {
    out['ladder'] =
      'Each rung must be above the one before it (a stronger model, or the same model at a higher effort).';
  }
  if (!Number.isInteger(p.gatesPerStep) || p.gatesPerStep < 1 || p.gatesPerStep > 5) {
    out['gatesPerStep'] = 'Between 1 and 5 failed gates per step.';
  }
  if (!Number.isInteger(p.cleanToStepDown) || p.cleanToStepDown < 1 || p.cleanToStepDown > 10) {
    out['cleanToStepDown'] = 'Between 1 and 10 clean completions.';
  }
  const gate = choiceProblem(p.gateChoice);
  if (gate) out['gateChoice'] = gate;
  else if (tier(p.gateChoice.model) >= 2) {
    out['gateChoice'] = 'The budget gate must send Opus work to a cheaper model.';
  }
  return out;
}

/** A dollar amount typed by the user: `null` for empty (no budget),
 * `undefined` when it is not a usable amount. */
export function parseUsd(text: string): number | null | undefined {
  const t = text.trim().replace(/^\$/, '').replace(/,/g, '');
  if (!t) return null;
  if (!/^\d+(\.\d{1,2})?$/.test(t)) return undefined;
  const n = Number(t);
  return n > 0 && n <= 1_000_000 ? n : undefined;
}

/** The last folder of a project path. */
export function projectName(path: string): string {
  const parts = path.split(/[\\/]+/).filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

/** What a rate-limit report says, in words. */
export function rateLimitText(n: RateLimitNote): string {
  const limit =
    n.limit === 'five_hour'
      ? 'the 5-hour limit'
      : n.limit === 'seven_day'
        ? 'the weekly limit'
        : n.limit === 'seven_day_opus'
          ? 'the Opus weekly limit'
          : n.limit === 'seven_day_sonnet'
            ? 'the Sonnet weekly limit'
            : 'a usage limit';
  if (n.status === 'rejected') return `Claude refused a request on ${limit}`;
  if (n.status === 'allowed_warning') return `Claude warned that ${limit} is near`;
  return `Claude reported ${limit}`;
}

/** The plan windows that matter for scheduling, 5-hour first. */
export function schedulingWindows(windows: PlanWindow[]): PlanWindow[] {
  const order = ['five_hour', 'seven_day', 'seven_day_opus', 'seven_day_sonnet'];
  return windows
    .filter((w) => order.includes(w.key))
    .sort((a, b) => order.indexOf(a.key) - order.indexOf(b.key));
}

/** The download name of a report. */
export function csvName(days: number, rows: 'groups' | 'outcomes'): string {
  return rows === 'outcomes'
    ? `mother-claude-task-outcomes-${days}d.csv`
    : `mother-claude-routing-${days}d.csv`;
}

/** A Capacity view read tolerantly (an older server, a partial answer):
 * `null` when it is not one at all. */
export function normalizeCapacity(raw: unknown): CapacityView | null {
  if (!raw || typeof raw !== 'object') return null;
  const v = raw as Partial<CapacityView>;
  if (!v.caps || !v.settings || !v.host) return null;
  return {
    now: typeof v.now === 'number' ? v.now : Date.now(),
    host: { ...v.host, sessions: Array.isArray(v.host.sessions) ? v.host.sessions : [] },
    caps: v.caps,
    accounts: Array.isArray(v.accounts) ? v.accounts : [],
    rateLimits: Array.isArray(v.rateLimits) ? v.rateLimits : [],
    running: Array.isArray(v.running) ? v.running : [],
    queue: Array.isArray(v.queue) ? v.queue : [],
    budgets: Array.isArray(v.budgets) ? v.budgets : [],
    routes: Array.isArray(v.routes) ? v.routes : [],
    settings: v.settings,
    classifier: v.classifier ?? {
      enabled: false,
      ready: false,
      model: 'claude-haiku-5-5',
      stats: { calls: 0, failures: 0, inputTokens: 0, outputTokens: 0, costUsd: 0 },
    },
    canManage: v.canManage === true,
    canEditPolicy: v.canEditPolicy === true,
  };
}
