// Dev-only: the scheduler side for `?mock=1` (docs/ROUTING.md) — `/host`,
// `/capacity`, `/capacity/settings`, `/routing/policy`, `/routing/report`
// (and `.csv`) and `/routing/classify`, with the server's refusals: a phone
// changes budgets and caps but not the Haiku classifier or billing, the
// policy is edited on the desktop only, and a bad value is a 422 naming its
// field. The 5-hour window sits at 91 %, so workers wait while the lead runs.
// Never bundled in production builds.

import {
  BudgetState,
  CapacitySettings,
  CapacityView,
  Classification,
  EscalationNote,
  PolicyView,
  QueueEntry,
  ReportRow,
  RoutingPolicy,
  RoutingReport,
  RunningView,
  csvName,
  policyProblems,
} from '../capacity';

const M = 60_000;
const H = 60 * M;
const MIB = 1024 * 1024;
const GIB = 1024 * MIB;
const PROJECT = '/Users/marcus/dev/mother-claude';

/** What the capacity mock needs from the fixture backend. */
export interface CapacityHost {
  readonly desktop: boolean;
}

type Body = Record<string, unknown>;

const mc = (model: string, effort?: string) => (effort ? { model, effort } : { model });

export function defaultPolicy(): RoutingPolicy {
  return {
    classes: {
      plan: mc('opus', 'high'),
      architecture: mc('opus', 'high'),
      review: mc('opus', 'high'),
      implement: mc('sonnet', 'medium'),
      test: mc('sonnet', 'medium'),
      fix: mc('sonnet', 'medium'),
      search: mc('haiku', 'low'),
      summarize: mc('haiku', 'low'),
      classify: mc('haiku', 'low'),
      ops: mc('sonnet', 'medium'),
    },
    ladder: [
      mc('haiku', 'low'),
      mc('haiku', 'high'),
      mc('sonnet', 'medium'),
      mc('sonnet', 'high'),
      mc('opus', 'high'),
      mc('opus', 'xhigh'),
      mc('fable', 'high'),
    ],
    fableOnEscalation: false,
    gatesPerStep: 2,
    cleanToStepDown: 2,
    gateChoice: mc('sonnet', 'high'),
    fallbacks: { opus: 'sonnet', fable: 'opus' },
  };
}

function defaultSettings(): CapacitySettings {
  return {
    userCap: 3,
    projectCap: 4,
    projectCaps: {},
    reserveMb: 2048,
    sessionMb: null,
    leadOnlyPct: 85,
    stopPct: 95,
    opusGatePct: 85,
    budgets: {
      projectDailyUsd: null,
      projects: { [PROJECT]: 20 },
      taskUsd: null,
      sessionUsd: 5,
      roles: {},
      nearPct: 80,
    },
    alerts: { desktop: true, push: true },
    haikuClassifier: false,
    billing: {},
  };
}

const CLASSES: [string, string][] = [
  ['plan', 'planning'],
  ['architecture', 'architecture'],
  ['review', 'review'],
  ['implement', 'implementation'],
  ['test', 'test'],
  ['fix', 'fix'],
  ['search', 'search'],
  ['summarize', 'summary'],
  ['classify', 'classification'],
  ['ops', 'operations'],
  ['other', 'other'],
];

/** Keywords per class, most specific first (a small copy of the server's). */
const RULES: [string, string[]][] = [
  ['fix', ['fix', 'bug', 'crash', 'broken', 'error', 'flaky', 'regression']],
  ['test', ['test', 'tests', 'coverage', 'e2e', 'spec']],
  ['review', ['review', 'audit', 'critique']],
  ['architecture', ['architecture', 'design', 'schema', 'adr']],
  ['plan', ['plan', 'roadmap', 'breakdown', 'scope', 'estimate']],
  ['ops', ['deploy', 'release', 'ci', 'pipeline', 'docker', 'infra']],
  ['summarize', ['summarize', 'summarise', 'summary', 'recap', 'changelog']],
  ['classify', ['classify', 'triage', 'label', 'categorize']],
  ['search', ['find', 'search', 'locate', 'investigate', 'research']],
  ['implement', ['implement', 'add', 'build', 'create', 'feature', 'refactor', 'write']],
];

export class MockCapacity {
  private settings = defaultSettings();
  private policy = defaultPolicy();

  constructor(private readonly host: CapacityHost) {}

  /** The answer to a request this mock takes, or null to let the fixture
   * backend answer it. */
  route(path: string, method: string, body: unknown, q: URLSearchParams): Response | null {
    const b = (body && typeof body === 'object' ? body : {}) as Body;
    if (path === '/host' && method === 'GET') return json(this.view().host);
    if (path === '/capacity' && method === 'GET') return json(this.view());
    if (path === '/capacity/settings' && method === 'PUT') return this.putSettings(b);
    if (path === '/routing/policy') {
      if (method === 'GET') return json(this.policyView());
      if (!this.host.desktop) return deskOnly();
      if (method === 'PUT') {
        const p = b as unknown as RoutingPolicy;
        if (!p.classes || !Array.isArray(p.ladder) || !p.gateChoice) {
          return fail(422, 'invalid_policy', 'Invalid policy: send the whole policy.', '');
        }
        const problems = Object.entries(policyProblems(p));
        if (problems.length) {
          const [field, message] = problems[0];
          return fail(422, 'invalid_policy', message, field);
        }
        this.policy = structuredClone(p);
        return json(this.policyView());
      }
      if (method === 'DELETE') {
        this.policy = defaultPolicy();
        return json(this.policyView());
      }
    }
    if (path === '/routing/report' && method === 'GET') {
      const days = Number(q.get('days') ?? 30);
      if (!Number.isInteger(days) || days < 1 || days > 365) {
        return fail(422, 'invalid_days', '`days` is 1 to 365.', 'days');
      }
      return json(this.report(days));
    }
    if (path === '/routing/report.csv' && method === 'GET') {
      const days = Number(q.get('days') ?? 30);
      const rows = q.get('rows') === 'outcomes' ? 'outcomes' : 'groups';
      return new Response(this.csv(days, rows), {
        status: 200,
        headers: {
          'content-type': 'text/csv; charset=utf-8',
          'content-disposition': `attachment; filename="${csvName(days, rows)}"`,
        },
      });
    }
    if (path === '/routing/classify' && method === 'POST') {
      const title = typeof b['title'] === 'string' ? b['title'] : '';
      const goal = typeof b['goal'] === 'string' ? b['goal'] : '';
      if (!title.trim() && !goal.trim()) {
        return fail(422, 'invalid_task', 'Give the task a title or a goal.', 'title');
      }
      if (b['haiku'] === true && !this.settings.haikuClassifier) {
        return fail(409, 'classifier_unavailable', 'the Haiku classifier is off', '');
      }
      return json(classify(title, goal));
    }
    return null;
  }

  private putSettings(patch: Body): Response {
    if (!this.host.desktop) {
      const k = ['haikuClassifier', 'billing'].find((key) => key in patch);
      if (k) {
        return fail(
          403,
          'desktop_only',
          `${k} can be changed on the Mother Claude desktop app only.`,
          k,
        );
      }
    }
    const next = structuredClone(this.settings);
    for (const [k, v] of Object.entries(patch)) {
      if (k === 'budgets' && v && typeof v === 'object') {
        Object.assign(next.budgets, v);
      } else if (k === 'alerts' && v && typeof v === 'object') {
        Object.assign(next.alerts, v);
      } else if (k in next) {
        (next as unknown as Body)[k] = v;
      }
    }
    const bad = invalidSettings(next);
    if (bad) return fail(422, 'invalid_settings', bad[1], bad[0]);
    this.settings = next;
    return json(this.view());
  }

  private policyView(): PolicyView {
    return {
      policy: this.policy,
      defaults: defaultPolicy(),
      isDefault: JSON.stringify(this.policy) === JSON.stringify(defaultPolicy()),
      classes: CLASSES.map(([c, label]) => ({ class: c, label })),
      models: ['haiku', 'sonnet', 'opus', 'fable'],
      efforts: ['low', 'medium', 'high', 'xhigh', 'max'],
      learned: [{ project: PROJECT, class: 'implement', level: 1, clean: 1 }],
      escalations: this.escalations(),
      canEdit: this.host.desktop,
    };
  }

  private escalations(): EscalationNote[] {
    const now = Date.now();
    return [
      {
        at: now - 25 * M,
        taskId: 'task-41',
        project: PROJECT,
        class: 'implement',
        direction: 'up',
        steps: 1,
        reason: '2 failed gates: escalated to 1 step',
      },
      {
        at: now - 26 * H,
        taskId: 'task-12',
        project: PROJECT,
        class: 'fix',
        direction: 'down',
        steps: 0,
        reason: '2 clean completions: fix tasks step back down',
      },
    ];
  }

  view(): CapacityView {
    const now = Date.now();
    const s = this.settings;
    const running: RunningView[] = [
      role('s-haiku', 'ceo', 'opus', true, now - 48 * M, 1.84, 512 * MIB),
      role('s-build', 'frontend', 'sonnet', false, now - 31 * M, 3.1, 420 * MIB),
      role('mock-backend-role', 'backend', 'sonnet', false, now - 22 * M, 1.42, 380 * MIB),
      role('mock-scout-role', 'scout', 'haiku', false, now - 9 * M, 0.06, 310 * MIB),
    ];
    const workers = running.filter((r) => !r.lead).length;
    const available = 9.6 * GIB;
    const estimate = s.sessionMb ? s.sessionMb * MIB : 420 * MIB;
    const reserve = s.reserveMb * MIB;
    const headroom = Math.max(0, Math.floor((available - reserve) / estimate));
    const window = { pct: 91, resetsAt: now + 80 * M };
    const waitWindow = `the 5-hour usage window is at ${window.pct} % (at ${s.leadOnlyPct} % or more only the lead starts, it resets in 1 h 20 min)`;
    const queue: QueueEntry[] = [
      {
        project: PROJECT,
        role: 'reviewer',
        model: 'opus',
        priority: 0,
        lead: false,
        account: 'default',
        admitted: false,
        reasons: [waitWindow, `${workers} role sessions run already (your limit is ${s.userCap})`],
        waitingSince: now - 14 * M,
        at: now - 4_000,
        asks: 168,
      },
      {
        project: '/Users/marcus/dev/website',
        role: 'writer',
        model: 'sonnet',
        priority: 2,
        lead: false,
        account: 'default',
        admitted: false,
        reasons: [waitWindow, "website's daily budget is spent ($12.10 of $12.00, estimated)"],
        waitingSince: now - 6 * M,
        at: now - 4_000,
        asks: 72,
      },
      {
        project: PROJECT,
        role: 'scout',
        model: 'haiku',
        priority: 2,
        lead: false,
        account: 'default',
        admitted: true,
        reasons: [],
        at: now - 9 * M,
        asks: 3,
      },
    ];
    const budgets: BudgetState[] = [
      ...Object.entries(s.budgets.projects).map(([project, limit]) =>
        budget('project', project, `${base(project)} today`, 16.4, limit, s.budgets.nearPct),
      ),
      ...(s.budgets.sessionUsd === null
        ? []
        : running.map((r) => {
            const b = budget(
              'session',
              r.sessionId,
              `${r.role} in ${base(r.project)}`,
              r.spentUsd ?? 0,
              s.budgets.roles[r.role] ?? s.budgets.sessionUsd ?? 5,
              s.budgets.nearPct,
            );
            return {
              ...b,
              project: r.project,
              role: r.role,
              sessionId: r.sessionId,
              applies: false,
              note: 'a plan account: not passed to Claude Code, shown only',
            };
          })),
    ];
    return {
      now,
      host: {
        sampledAt: now - 2_000,
        totalMemory: 24 * GIB,
        availableMemory: available,
        usedMemory: 14.4 * GIB,
        cpus: 12,
        load: [3.21, 2.87, 2.4],
        sessions: running.map((r) => ({
          sessionId: r.sessionId,
          pid: 40_000,
          rssBytes: r.rssBytes ?? 0,
        })),
        estimateBytes: estimate,
        measured: !s.sessionMb,
        reserveBytes: reserve,
        headroom,
      },
      caps: {
        userCap: s.userCap,
        projectCap: s.projectCap,
        workersRunning: workers,
        leadsRunning: running.length - workers,
        hardwareCap: Math.min(16, running.length + headroom),
      },
      accounts: [
        {
          id: 'default',
          name: 'Default',
          billing: { billing: 'plan', source: 'a claude.ai plan' },
          planUsage: true,
          windows: [
            {
              key: 'five_hour',
              label: 'Current session',
              pct: window.pct,
              resetsAt: window.resetsAt,
              severity: 'warning',
            },
            { key: 'seven_day', label: 'Weekly · all models', pct: 48, resetsAt: now + 3 * 24 * H },
            {
              key: 'seven_day_opus',
              label: 'Weekly · Opus',
              pct: 87,
              resetsAt: now + 3 * 24 * H,
              severity: 'warning',
            },
            {
              key: 'seven_day_sonnet',
              label: 'Weekly · Sonnet',
              pct: 22,
              resetsAt: now + 3 * 24 * H,
            },
          ],
          leadOnly: window.pct >= s.leadOnlyPct,
          stopped: window.pct >= s.stopPct,
          opusGated: 87 >= s.opusGatePct,
          rateLimit: {
            at: now - 3 * M,
            account: 'default',
            status: 'allowed_warning',
            limit: 'five_hour',
            pct: 90,
            source: 'stream',
          },
        },
      ],
      rateLimits: [
        {
          at: now - 3 * M,
          account: 'default',
          status: 'allowed_warning',
          limit: 'five_hour',
          pct: 90,
          source: 'stream',
        },
        {
          at: now - 5 * H,
          account: 'default',
          status: 'rejected',
          limit: 'five_hour',
          resetsAt: now - 4 * H,
          source: 'stream',
        },
      ],
      running,
      queue,
      budgets,
      routes: [
        {
          at: now - 2 * M,
          project: PROJECT,
          role: 'reviewer',
          class: 'review',
          choice: mc('opus', 'high'),
          reason: 'the policy for review tasks',
        },
        {
          at: now - 9 * M,
          project: PROJECT,
          role: 'scout',
          class: 'search',
          choice: mc('haiku', 'low'),
          reason: 'the policy for search tasks',
        },
        {
          at: now - 22 * M,
          project: PROJECT,
          role: 'backend',
          class: 'implement',
          choice: mc('sonnet', 'high'),
          reason: 'escalated 1 step from Sonnet · medium',
        },
        {
          at: now - 48 * M,
          project: PROJECT,
          role: 'ceo',
          class: 'plan',
          choice: mc('sonnet', 'high'),
          reason:
            'the Opus weekly window is at 87 % (the gate is 85 %): Sonnet · high instead of Opus · high',
        },
      ],
      settings: s,
      classifier: {
        enabled: s.haikuClassifier,
        ready: false,
        ...(s.haikuClassifier
          ? {
              whyNot:
                'the Haiku classifier needs an Anthropic API key (ANTHROPIC_API_KEY when Mother Claude starts)',
            }
          : {}),
        model: 'claude-haiku-5-5',
        stats: { calls: 0, failures: 0, inputTokens: 0, outputTokens: 0, costUsd: 0 },
      },
      canManage: true,
      canEditPolicy: this.host.desktop,
    };
  }

  private report(days: number): RoutingReport {
    const now = Date.now();
    const k = days / 30;
    const n = (v: number) => Math.max(1, Math.round(v * k));
    const rows = [
      row('plan', 'opus', 'high', n(4), n(4), 0, 0, 3.12 * k, 0.86, 1, 1),
      row('review', 'opus', 'high', n(9), n(8), n(1), 0, 6.48 * k, 0.81, 9, 8),
      row('implement', 'sonnet', 'medium', n(18), n(14), n(2), n(2), 11.9 * k, 0.78, 16, 12),
      row('implement', 'opus', 'high', n(2), n(2), 0, 0, 3.4 * k, 0.69, 2, 2),
      row('test', 'sonnet', 'medium', n(7), n(6), n(1), 0, 2.55 * k, 0.74, 7, 6),
      row('search', 'haiku', 'low', n(12), n(12), 0, 0, 0.19 * k, 0.88, 0, 0),
      row('summarize', 'haiku', 'low', n(5), n(5), 0, 0, 0.07 * k, 0.9, 0, 0),
    ];
    const classes = new Map<string, ReportRow>();
    for (const r of rows) {
      const c = classes.get(r.class) ?? row(r.class, 'all', 'all', 0, 0, 0, 0, 0, 0, 0, 0);
      classes.set(r.class, sum(c, r));
    }
    const totals = rows.reduce(
      (t, r) => sum(t, r),
      row('all', 'all', 'all', 0, 0, 0, 0, 0, 0, 0, 0),
    );
    return {
      generatedAt: now,
      since: now - days * 24 * H,
      days,
      rows,
      classes: [...classes.values()].map(finish),
      totals: finish(totals),
      escalations: this.escalations().filter((e) => e.at >= now - days * 24 * H),
      learned: [{ project: PROJECT, class: 'implement', level: 1, clean: 1 }],
      classifier: { calls: 0, failures: 0, inputTokens: 0, outputTokens: 0, costUsd: 0 },
      pending: 0,
      estimateNote:
        'Estimates: token usage from the sessions’ transcripts priced at first-party API rates. On plan accounts (Pro, Max, Team) the work is paid in usage windows, so dollars are what it would have cost through the API.',
    };
  }

  private csv(days: number, rows: 'groups' | 'outcomes'): string {
    if (rows === 'outcomes') {
      return (
        'task_id,project,role,class,model,effort,result,gate_passed,escalations,started_at,ended_at,minutes,account,pays_per_token,input,output,cache_read,cache_write,est_cost_usd,est_window_pct,escalation,note\r\n' +
        `task-41,${PROJECT},backend,implement,sonnet,medium,failed,false,0,,,22.0,default,false,12000,3100,180000,9000,0.31,0.6,,\r\n`
      );
    }
    const r = this.report(days);
    const head =
      'class,model,effort,tasks,completed,failed,escalated,cancelled,est_cost_usd,est_cost_per_completed_usd,tokens,tokens_per_completed,cache_hit_ratio,gate_pass_rate,escalation_rate\r\n';
    const line = (x: ReportRow) =>
      [
        x.class,
        x.model,
        x.effort,
        x.tasks,
        x.completed,
        x.failed,
        x.escalated,
        x.cancelled,
        x.costUsd,
        x.costPerCompletedUsd ?? '',
        x.tokens,
        x.tokensPerCompleted ?? '',
        x.cacheHitRatio ?? '',
        x.gatePassRate ?? '',
        x.escalationRate ?? '',
      ].join(',');
    return head + [...r.rows, ...r.classes, r.totals].map(line).join('\r\n') + '\r\n';
  }
}

function base(path: string): string {
  return path.split('/').filter(Boolean).pop() ?? path;
}

function role(
  sessionId: string,
  name: string,
  model: string,
  lead: boolean,
  since: number,
  spent: number,
  rss: number,
): RunningView {
  return {
    sessionId,
    project: PROJECT,
    role: name,
    model,
    lead,
    firstSeen: since,
    lastSeen: Date.now() - 4_000,
    live: true,
    spentUsd: spent,
    rssBytes: rss,
  };
}

function budget(
  scope: BudgetState['scope'],
  key: string,
  label: string,
  spent: number,
  limit: number,
  near: number,
): BudgetState {
  const pct = Math.round((spent / limit) * 1000) / 10;
  return {
    scope,
    key,
    label,
    ...(scope === 'project' ? { project: key } : {}),
    spentUsd: spent,
    limitUsd: limit,
    pct,
    level: pct >= 100 ? 'over' : pct >= near ? 'near' : 'ok',
    applies: true,
  };
}

function row(
  cls: string,
  model: string,
  effort: string,
  tasks: number,
  completed: number,
  failed: number,
  escalated: number,
  cost: number,
  cacheHit: number,
  gates: number,
  passed: number,
): ReportRow {
  const tokens = Math.round(cost * 210_000);
  return finish({
    class: cls,
    model,
    effort,
    tasks,
    completed,
    failed,
    escalated,
    cancelled: Math.max(0, tasks - completed - failed - escalated),
    withUsage: tasks,
    costUsd: Math.round(cost * 1e4) / 1e4,
    tokens,
    input: Math.round(tokens * (1 - cacheHit) * 0.6),
    output: Math.round(tokens * 0.08),
    cacheRead: Math.round(tokens * cacheHit * 0.85),
    cacheWrite: Math.round(tokens * 0.05),
    gates,
    gatesPassed: passed,
    perToken: 0,
  });
}

function sum(a: ReportRow, b: ReportRow): ReportRow {
  return {
    ...a,
    tasks: a.tasks + b.tasks,
    completed: a.completed + b.completed,
    failed: a.failed + b.failed,
    escalated: a.escalated + b.escalated,
    cancelled: a.cancelled + b.cancelled,
    withUsage: a.withUsage + b.withUsage,
    costUsd: Math.round((a.costUsd + b.costUsd) * 1e4) / 1e4,
    tokens: a.tokens + b.tokens,
    input: a.input + b.input,
    output: a.output + b.output,
    cacheRead: a.cacheRead + b.cacheRead,
    cacheWrite: a.cacheWrite + b.cacheWrite,
    gates: a.gates + b.gates,
    gatesPassed: a.gatesPassed + b.gatesPassed,
  };
}

function finish(r: ReportRow): ReportRow {
  const inputs = r.input + r.cacheRead + r.cacheWrite;
  return {
    ...r,
    costPerCompletedUsd: r.completed
      ? Math.round((r.costUsd / r.completed) * 1e4) / 1e4
      : undefined,
    tokensPerCompleted: r.completed ? Math.round(r.tokens / r.completed) : undefined,
    cacheHitRatio: inputs ? Math.round((r.cacheRead / inputs) * 1e4) / 1e4 : undefined,
    gatePassRate: r.gates ? Math.round((r.gatesPassed / r.gates) * 1e4) / 1e4 : undefined,
    escalationRate: r.tasks ? Math.round((r.escalated / r.tasks) * 1e4) / 1e4 : undefined,
    minutesPerCompleted: r.completed ? 24.5 : undefined,
  };
}

function invalidSettings(s: CapacitySettings): [string, string] | null {
  const int = (v: unknown, lo: number, hi: number) =>
    typeof v === 'number' && Number.isInteger(v) && v >= lo && v <= hi;
  if (!int(s.userCap, 1, 16)) return ['userCap', 'Your limit must be between 1 and 16.'];
  if (!int(s.projectCap, 1, 16)) {
    return ['projectCap', "A project's limit must be between 1 and 16."];
  }
  if (s.stopPct < s.leadOnlyPct) {
    return ['stopPct', 'Nothing-starts must be at or above lead-only.'];
  }
  const usd = (v: unknown) => v === null || (typeof v === 'number' && v > 0 && v <= 1_000_000);
  for (const k of ['projectDailyUsd', 'taskUsd', 'sessionUsd'] as const) {
    if (!usd(s.budgets[k])) {
      return [`budgets.${k}`, 'A budget must be more than $0 and at most $1,000,000.'];
    }
  }
  for (const v of Object.values(s.budgets.projects)) {
    if (!usd(v))
      return ['budgets.projects', 'A budget must be more than $0 and at most $1,000,000.'];
  }
  if (!(s.budgets.nearPct >= 1 && s.budgets.nearPct <= 99)) {
    return ['budgets.nearPct', 'The alert threshold must be between 1 and 99 %.'];
  }
  return null;
}

function classify(title: string, goal: string): Classification {
  const words = (t: string) =>
    ` ${t
      .toLowerCase()
      .replace(/[^a-z0-9;#+-]+/g, ' ')
      .trim()} `;
  const t = words(title);
  const g = words(goal);
  let best: { cls: string; score: number; hit: string[] } | null = null;
  for (const [cls, keywords] of RULES) {
    let score = 0;
    const hit: string[] = [];
    for (const kw of keywords) {
      const n = 2 * t.split(` ${kw} `).length - 2 + (g.split(` ${kw} `).length - 1);
      if (n > 0) {
        score += n;
        hit.push(kw);
      }
    }
    if (score > 0 && (!best || score > best.score)) best = { cls, score, hit };
  }
  if (!best) return { class: 'other', source: 'rules', reason: 'no keyword matched' };
  const shown = best.hit.slice(0, 3).map((k) => `“${k}”`);
  return {
    class: best.cls,
    source: 'rules',
    reason: `keyword${shown.length === 1 ? '' : 's'} ${shown.join(', ')}`,
  };
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function fail(status: number, code: string, message: string, field: string): Response {
  return json({ error: { code, message, ...(field ? { field } : {}) } }, status);
}

function deskOnly(): Response {
  return fail(403, 'desktop_only', 'Only the desktop app can do this.', '');
}
