// Dev-only fixture data for `?mock=1`. Never bundled in production builds
// (mock-loader.ts is swapped for mock-loader.prod.ts).

import {
  AgentInfo,
  BackgroundTask,
  BridgeStatus,
  ControlCaps,
  DeviceInfo,
  ForeignControlStatus,
  GitOverview,
  LaunchDefaults,
  PendingItem,
  PendingPair,
  PendingPreview,
  ProjectInfo,
  PushPrefs,
  QueuedMessage,
  RemoteInfo,
  SessionSummary,
  UiEvent,
  WorkflowRun,
} from '../models';

export interface Fixtures {
  sessions: SessionSummary[];
  /** Full pending items (sessions carry previews). */
  pendingItems: PendingItem[];
  tasks: Record<string, BackgroundTask[]>;
  queued: Record<string, QueuedMessage[]>;
  terminals: Record<string, string>;
  history: SessionSummary[];
  agents: Record<string, AgentInfo[]>;
  workflows: Record<string, WorkflowRun[]>;
  transcripts: Record<string, UiEvent[]>;
  agentTranscripts: Record<string, UiEvent[]>;
  projects: ProjectInfo[];
  defaults: LaunchDefaults;
  remote: RemoteInfo;
  devices: DeviceInfo[];
  pendingPairs: PendingPair[];
  foreign: ForeignControlStatus;
  pushPrefs: PushPrefs;
  bridge: BridgeStatus;
  diff: GitOverview;
}

const S = 1000;
const M = 60 * S;
const H = 60 * M;
const D = 24 * H;

const OPUS = { id: 'claude-opus-5-5', label: 'Opus 5.5', contextWindow: 1_000_000 };
const SONNET = { id: 'claude-sonnet-5', label: 'Sonnet 5', contextWindow: 1_000_000 };
const HAIKU = { id: 'claude-haiku-4-5', label: 'Haiku 4.5', contextWindow: 200_000 };

export function ownedCaps(): ControlCaps {
  return {
    channel: 'owned',
    message: 'direct',
    approve: true,
    answer: true,
    interrupt: 'immediate',
    setModel: true,
    setMode: 'any',
    setEffort: true,
    slashCommands: true,
    attachImages: true,
    stopTask: true,
    fork: true,
    continueOwned: false,
    lifecycle: { stop: true, respawn: false, rm: false },
    hooksActive: false,
    rename: 'sync',
    terminal: false,
    keys: false,
    attach: 'inline',
    approveVia: 'control',
    wake: true,
    limits: [],
  };
}

function hookCaps(): ControlCaps {
  return {
    channel: 'hooks',
    message: 'relay',
    approve: true,
    answer: true,
    interrupt: 'next-tool',
    setModel: false,
    setMode: 'on-prompt',
    setEffort: false,
    slashCommands: false,
    attachImages: false,
    stopTask: false,
    fork: true,
    continueOwned: true,
    lifecycle: { stop: false, respawn: false, rm: false },
    hooksActive: true,
    rename: 'alias',
    terminal: false,
    keys: false,
    attach: 'path',
    approveVia: 'hook',
    wake: true,
    limits: [
      {
        code: 'relay_delivery',
        text: "Messages arrive at Claude's next stopping point and don't show in the VS Code panel.",
      },
      { code: 'interrupt_next_tool', text: 'Interrupts take effect at the next tool call.' },
      {
        code: 'foreign_model',
        text: 'Change the model or effort in VS Code itself, or continue here for full control.',
        action: 'fork',
      },
    ],
  };
}

function tmuxCaps(): ControlCaps {
  return {
    channel: 'tmux',
    message: 'relay',
    approve: true,
    answer: true,
    interrupt: 'immediate',
    setModel: true,
    setMode: 'on-prompt',
    setEffort: true,
    slashCommands: true,
    attachImages: false,
    stopTask: false,
    fork: true,
    continueOwned: true,
    lifecycle: { stop: false, respawn: false, rm: false },
    hooksActive: true,
    rename: 'alias',
    terminal: true,
    keys: true,
    attach: 'path',
    approveVia: 'hook',
    wake: true,
    limits: [
      {
        code: 'tmux_typed',
        text: 'Messages are typed into its tmux pane (session “work”, pane %3).',
      },
      {
        code: 'tmux_settings',
        text: '/model and /effort sent this way also change your saved Claude settings.',
      },
      {
        code: 'terminal_dialogs',
        text: 'Dialogs open in the terminal can be answered with keys.',
        action: 'open-terminal',
      },
    ],
  };
}

function noneCaps(extra: Partial<ControlCaps> = {}, limits?: ControlCaps['limits']): ControlCaps {
  return {
    channel: 'none',
    message: 'none',
    approve: false,
    answer: false,
    interrupt: 'none',
    setModel: false,
    setMode: 'none',
    setEffort: false,
    slashCommands: false,
    attachImages: false,
    stopTask: false,
    fork: true,
    continueOwned: true,
    lifecycle: { stop: false, respawn: false, rm: false },
    hooksActive: false,
    rename: 'alias',
    terminal: false,
    keys: false,
    attach: 'none',
    approveVia: 'none',
    wake: false,
    limits: limits ?? [
      {
        code: 'no_hooks',
        text: 'Install foreign-session control to answer prompts and send messages from here.',
        action: 'install-foreign-control',
      },
      {
        code: 'continue_owned',
        text: 'Or continue it here as a session Mother Claude controls fully.',
        action: 'fork',
      },
    ],
    ...extra,
  };
}

function base(
  now: number,
  over: Partial<SessionSummary> & Pick<SessionSummary, 'id'>,
): SessionSummary {
  return {
    name: over.id,
    projectName: 'project',
    cwd: '/Users/marcus/dev/project',
    surface: 'terminal',
    owned: false,
    live: true,
    status: 'idle',
    activity: { kind: 'idle', label: 'Idle' },
    usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
    messageCount: 0,
    agents: { running: 0, total: 0, workflowsRunning: 0, backgroundRunning: 0 },
    queued: 0,
    pending: [],
    control: noneCaps(),
    startedAt: now - H,
    lastActivityAt: now - 5 * M,
    version: '2.1.282',
    ...over,
  };
}

export const MIGRATION_PLAN = `## Plan: migrate billing to Stripe API v3

1. **Inventory** every call into \`billing/stripe_client.py\` (23 call sites).
2. Add a \`StripeV3Client\` behind the existing \`BillingGateway\` interface.
3. Migrate webhooks: verify signatures with the new endpoint secret; keep the v2 handlers for 14 days.
4. Backfill \`customer.default_payment_method\` for 1,204 legacy customers (idempotent script, dry run first).
5. Switch traffic with the \`billing_v3\` flag: 5% → 50% → 100%.

### Risks
- Webhook ordering differs in v3 — handlers must be idempotent.
- Proration rounding changed; add snapshot tests for invoices.

### Out of scope
- Tax (Stripe Tax) — separate project.`;

/** The list copy of a pending item (what `sessions` frames carry). */
export function toPreview(p: PendingItem): PendingPreview {
  const q = p.questions ?? [];
  const single = q.length === 1 && !q[0].multiSelect && q[0].options.length <= 4;
  return {
    id: p.id,
    kind: p.kind,
    title: p.title,
    detail: (p.detail ?? q[0]?.question ?? p.plan?.split('\n')[0] ?? '').slice(0, 240) || undefined,
    toolName: p.toolName,
    agentId: p.agentId,
    dangerous: p.dangerous,
    answerable: p.answerable,
    createdAt: p.createdAt,
    ...(q.length ? { questionCount: q.length } : {}),
    ...(single ? { quick: { options: q[0].options.map((o) => o.label) } } : {}),
  };
}

export function buildFixtures(now: number): Fixtures {
  const pendingItems: PendingItem[] = [];
  const P = (items: PendingItem[]): PendingPreview[] => {
    pendingItems.push(...items);
    return items.map(toPreview);
  };
  const sessions: SessionSummary[] = [
    base(now, {
      id: 's-auth',
      name: 'auth-refactor',
      title: 'Refactor auth middleware',
      projectName: 'api-server',
      cwd: '/Users/marcus/dev/api-server',
      surface: 'mother-claude',
      entrypoint: 'sdk-cli',
      owned: true,
      status: 'waiting',
      waitingFor: 'permission prompt',
      activity: {
        kind: 'waiting',
        label: 'Waiting for approval',
        tool: 'Bash',
        detail: 'npm run migrate -- --force',
        since: now - 45 * S,
      },
      model: OPUS,
      effort: 'high',
      permissionMode: 'default',
      gitBranch: 'feat/session-tokens',
      prLink: {
        number: 412,
        url: 'https://github.com/acme/api-server/pull/412',
        repo: 'acme/api-server',
      },
      startedAt: now - 52 * M,
      lastActivityAt: now - 45 * S,
      pid: 48213,
      usage: { input: 182_000, output: 41_000, cacheRead: 2_400_000, cacheWrite: 310_000 },
      costUsd: 4.87,
      context: { used: 412_000, window: 1_000_000, pct: 41 },
      messageCount: 86,
      lastPrompt: 'Move session validation into middleware and add token rotation',
      todos: { total: 7, done: 4, inProgress: 1, activeForm: 'Running the database migration' },
      agents: { running: 2, total: 5, workflowsRunning: 0, backgroundRunning: 1 },
      pending: P([
        {
          id: 'req-auth-1',
          sessionId: 's-auth',
          kind: 'permission',
          source: 'owned',
          toolName: 'Bash',
          toolUseId: 'toolu_auth_1',
          title: 'Run a database migration',
          detail: 'npm run migrate -- --force',
          input: {
            command: 'npm run migrate -- --force',
            description: 'Apply pending migrations to the dev database',
          },
          suggestions: [
            {
              index: 0,
              label: 'Always allow “npm run migrate:*” in this project',
              kind: 'addRules',
              destination: 'localSettings',
              standing: true,
              raw: {},
            },
            {
              index: 1,
              label: 'Allow “npm run migrate:*” for the rest of this session',
              kind: 'addRules',
              destination: 'session',
              standing: false,
              raw: {},
            },
          ],
          dangerous: false,
          answerable: true,
          createdAt: now - 45 * S,
        },
        {
          id: 'req-auth-2',
          sessionId: 's-auth',
          kind: 'permission',
          source: 'owned',
          toolName: 'Edit',
          toolUseId: 'toolu_auth_2',
          agentId: 'ag-explore',
          agentType: 'Explore',
          title: 'Edit src/routes/users.ts',
          detail: 'src/routes/users.ts',
          input: {
            file_path: '/Users/marcus/dev/api-server/src/routes/users.ts',
            old_string:
              'import { validateSession } from "../auth";\n\nrouter.get("/me", (req, res) => {\n  if (!validateSession(req)) return res.sendStatus(401);\n  res.json(req.user);\n});',
            new_string:
              'import { requireSession } from "../middleware/session";\n\nrouter.use(requireSession());\nrouter.get("/me", (req, res) => {\n  res.json(req.user);\n});',
          },
          suggestions: [
            {
              index: 0,
              label: 'Allow all edits in this session',
              kind: 'setMode',
              destination: 'session',
              standing: false,
              raw: {},
            },
          ],
          dangerous: false,
          answerable: true,
          createdAt: now - 30 * S,
        },
        {
          id: 'req-auth-3',
          sessionId: 's-auth',
          kind: 'elicitation',
          source: 'owned',
          title: 'GitHub needs a repository',
          input: {
            message: 'Which repository should the pull request be opened in?',
            requestedSchema: {
              type: 'object',
              properties: {
                repo: { type: 'string', title: 'Repository (owner/name)' },
                draft: { type: 'boolean', title: 'Open as a draft' },
                base: { type: 'string', title: 'Base branch', enum: ['main', 'develop'] },
              },
              required: ['repo'],
            },
          },
          suggestions: [],
          dangerous: false,
          answerable: true,
          createdAt: now - 12 * S,
        },
      ]),
      control: ownedCaps(),
    }),
    base(now, {
      id: 's-db',
      name: 'analytics',
      title: 'Pick a database for analytics',
      projectName: 'analytics',
      cwd: '/Users/marcus/dev/analytics',
      surface: 'vscode',
      entrypoint: 'claude-vscode',
      status: 'waiting',
      waitingFor: 'input needed',
      activity: { kind: 'waiting', label: 'Waiting for your answer', since: now - 2 * M },
      model: SONNET,
      effort: 'medium',
      permissionMode: 'acceptEdits',
      gitBranch: 'main',
      startedAt: now - 25 * M,
      lastActivityAt: now - 2 * M,
      usage: { input: 64_000, output: 12_500, cacheRead: 620_000, cacheWrite: 88_000 },
      costUsd: 0.92,
      context: { used: 128_000, window: 1_000_000, pct: 13 },
      messageCount: 31,
      lastPrompt: 'We need an analytics backend. Compare options and set up the schema.',
      pending: P([
        {
          id: 'req-db-1',
          sessionId: 's-db',
          kind: 'question',
          source: 'hook',
          toolName: 'AskUserQuestion',
          title: 'Claude has 2 questions',
          questions: [
            {
              question: 'Which database should the analytics service use?',
              header: 'Database',
              multiSelect: false,
              options: [
                {
                  label: 'PostgreSQL + TimescaleDB',
                  description: 'One engine to run; fine up to about a billion rows',
                },
                {
                  label: 'ClickHouse',
                  description: 'Columnar and very fast, but another system to operate',
                },
                { label: 'DuckDB on S3', description: 'Serverless and cheap; batch only' },
              ],
            },
            {
              question: 'Which dashboards are needed on day one?',
              header: 'Scope',
              multiSelect: true,
              options: [
                { label: 'Revenue' },
                { label: 'Retention' },
                { label: 'Funnel' },
                { label: 'Latency' },
              ],
            },
          ],
          suggestions: [],
          dangerous: false,
          answerable: true,
          createdAt: now - 2 * M,
        },
      ]),
      control: hookCaps(),
    }),
    base(now, {
      id: 's-billing',
      name: 'billing',
      title: 'Migrate billing to Stripe v3',
      projectName: 'payments',
      cwd: '/Users/marcus/dev/payments',
      surface: 'tmux',
      tmuxPane: 'work:@2.%3',
      status: 'waiting',
      waitingFor: 'plan approval',
      activity: { kind: 'waiting', label: 'Plan ready for review', since: now - 6 * M },
      model: OPUS,
      effort: 'xhigh',
      permissionMode: 'plan',
      gitBranch: 'stripe-v3',
      startedAt: now - 40 * M,
      lastActivityAt: now - 6 * M,
      usage: { input: 240_000, output: 30_000, cacheRead: 1_100_000, cacheWrite: 140_000 },
      costUsd: 2.41,
      context: { used: 780_000, window: 1_000_000, pct: 78 },
      messageCount: 44,
      pending: P([
        {
          id: 'req-billing-1',
          sessionId: 's-billing',
          kind: 'plan',
          source: 'hook',
          toolName: 'ExitPlanMode',
          title: 'Plan ready',
          detail: 'Migrate billing to Stripe API v3 in five steps behind a feature flag.',
          plan: MIGRATION_PLAN,
          suggestions: [],
          dangerous: false,
          answerable: true,
          createdAt: now - 6 * M,
        },
      ]),
      control: tmuxCaps(),
    }),
    base(now, {
      id: 's-audit',
      name: 'deps-audit',
      title: 'Nightly dependency audit',
      projectName: 'monorepo',
      cwd: '/Users/marcus/dev/monorepo',
      surface: 'background',
      kind: 'background',
      jobId: 'k7m2',
      status: 'working',
      activity: {
        kind: 'tool',
        label: 'Running Bash',
        tool: 'Bash',
        detail: 'npm audit --json',
        since: now - 12 * S,
      },
      model: HAIKU,
      permissionMode: 'acceptEdits',
      gitBranch: 'chore/deps',
      startedAt: now - 9 * M,
      lastActivityAt: now - 3 * S,
      usage: { input: 91_000, output: 8_000, cacheRead: 300_000, cacheWrite: 20_000 },
      costUsd: 0.21,
      context: { used: 150_000, window: 200_000, pct: 75 },
      messageCount: 18,
      todos: { total: 5, done: 2, inProgress: 1, activeForm: 'Scanning workspaces' },
      agents: { running: 3, total: 3, workflowsRunning: 1, backgroundRunning: 0 },
      control: noneCaps({ lifecycle: { stop: true, respawn: true, rm: true }, terminal: true }, [
        {
          code: 'background',
          text: 'Background session: stop, respawn and remove work from here; its log is in the terminal view.',
          action: 'open-terminal',
        },
        {
          code: 'no_hooks',
          text: 'Install foreign-session control to send it messages.',
          action: 'install-foreign-control',
        },
      ]),
    }),
    base(now, {
      id: 's-docs',
      name: 'docs',
      title: 'Docs site polish',
      projectName: 'website',
      cwd: '/Users/marcus/dev/website',
      surface: 'terminal',
      status: 'working',
      activity: {
        kind: 'delegating',
        label: 'Delegating',
        detail: 'Rewrite the quick-start guide',
        since: now - 95 * S,
      },
      model: SONNET,
      startedAt: now - 14 * M,
      lastActivityAt: now - 10 * S,
      usage: { input: 51_000, output: 9_800, cacheRead: 410_000, cacheWrite: 52_000 },
      costUsd: 0.58,
      context: { used: 96_000, window: 1_000_000, pct: 10 },
      messageCount: 22,
      queued: 1,
      agents: { running: 2, total: 2, workflowsRunning: 0, backgroundRunning: 0 },
      control: hookCaps(),
    }),
    base(now, {
      id: 's-flaky',
      name: 'checkout',
      title: 'Fix flaky checkout test',
      projectName: 'storefront',
      cwd: '/Users/marcus/dev/storefront',
      surface: 'jetbrains',
      status: 'waiting',
      waitingFor: 'permission prompt',
      activity: { kind: 'waiting', label: 'Waiting for approval in JetBrains', since: now - 4 * M },
      model: OPUS,
      gitBranch: 'fix/checkout-timeout',
      startedAt: now - 30 * M,
      lastActivityAt: now - 4 * M,
      usage: { input: 30_000, output: 6_000, cacheRead: 200_000, cacheWrite: 30_000 },
      costUsd: 0.64,
      messageCount: 17,
      pending: P([
        {
          id: 'obs-flaky-1',
          sessionId: 's-flaky',
          kind: 'permission',
          source: 'observed',
          toolName: 'Edit',
          title: 'Edit tests/checkout.spec.ts',
          detail: 'tests/checkout.spec.ts',
          suggestions: [],
          dangerous: false,
          answerable: false,
          createdAt: now - 4 * M,
        },
      ]),
      control: noneCaps(),
    }),
    base(now, {
      id: 's-cache',
      name: 'caching',
      title: 'Explore caching strategy',
      projectName: 'api-server',
      cwd: '/Users/marcus/dev/api-server',
      surface: 'desktop',
      entrypoint: 'claude-desktop',
      status: 'idle',
      activity: { kind: 'idle', label: 'Idle' },
      model: OPUS,
      startedAt: now - 3 * H,
      lastActivityAt: now - 38 * M,
      usage: { input: 70_000, output: 21_000, cacheRead: 500_000, cacheWrite: 60_000 },
      costUsd: 1.37,
      context: { used: 210_000, window: 1_000_000, pct: 21 },
      messageCount: 12,
      lastMessage:
        'I compared read-through, write-behind and request coalescing. For this API, read-through with a 30 s TTL wins.',
      control: noneCaps(),
    }),
    base(now, {
      id: 's-release',
      name: 'release',
      title: 'Release notes v0.3',
      projectName: 'mother-claude',
      cwd: '/Users/marcus/dev/mother-claude',
      surface: 'terminal',
      live: false,
      status: 'ended',
      activity: { kind: 'ended', label: 'Ended' },
      model: SONNET,
      startedAt: now - 4 * H,
      lastActivityAt: now - 3 * H,
      usage: { input: 20_000, output: 7_000, cacheRead: 90_000, cacheWrite: 10_000 },
      costUsd: 1.12,
      costFinal: true,
      messageCount: 9,
      lastMessage: 'Release notes are in CHANGELOG.md under 0.3.0.',
      control: noneCaps(),
    }),
    base(now, {
      id: 's-stream',
      name: 'search-index',
      title: 'Speed up the search index build',
      projectName: 'monorepo',
      cwd: '/Users/marcus/dev/monorepo',
      surface: 'mother-claude',
      entrypoint: 'sdk-cli',
      owned: true,
      status: 'working',
      activity: { kind: 'thinking', label: 'Writing', since: now - 8 * S },
      model: OPUS,
      effort: 'medium',
      permissionMode: 'acceptEdits',
      gitBranch: 'perf/search-index',
      startedAt: now - 6 * M,
      lastActivityAt: now - 2 * S,
      pid: 51002,
      usage: { input: 42_000, output: 9_000, cacheRead: 380_000, cacheWrite: 41_000 },
      costUsd: 0.74,
      context: { used: 96_000, window: 1_000_000, pct: 10 },
      messageCount: 14,
      lastPrompt: 'Profile the index build and make it at least 2x faster',
      queued: 1,
      control: ownedCaps(),
    }),
    base(now, {
      id: 's-deploy',
      name: 'release-cli',
      title: 'Publish the CLI to npm',
      projectName: 'cli',
      cwd: '/Users/marcus/dev/cli',
      surface: 'terminal',
      status: 'waiting',
      waitingFor: 'permission prompt',
      activity: {
        kind: 'waiting',
        label: 'Waiting for approval',
        tool: 'Bash',
        since: now - 70 * S,
      },
      model: SONNET,
      permissionMode: 'default',
      gitBranch: 'main',
      startedAt: now - 11 * M,
      lastActivityAt: now - 70 * S,
      usage: { input: 18_000, output: 4_000, cacheRead: 90_000, cacheWrite: 12_000 },
      costUsd: 0.19,
      messageCount: 9,
      pending: P([
        {
          id: 'req-deploy-1',
          sessionId: 's-deploy',
          kind: 'permission',
          source: 'hook',
          toolName: 'Bash',
          toolUseId: 'toolu_deploy_1',
          title: 'Publish to npm',
          detail: 'rm -rf dist && npm run build && npm publish --access public',
          input: {
            command: 'rm -rf dist && npm run build && npm publish --access public',
            description: 'Rebuild and publish version 2.4.0',
          },
          suggestions: [
            {
              index: 0,
              label: 'Always allow “npm publish:*” in every project',
              kind: 'addRules',
              destination: 'userSettings',
              standing: true,
              raw: {},
            },
          ],
          dangerous: true,
          answerable: true,
          createdAt: now - 70 * S,
        },
      ]),
      control: hookCaps(),
    }),
  ];

  const projectsList = [
    'api-server',
    'analytics',
    'payments',
    'monorepo',
    'website',
    'storefront',
    'mother-claude',
    'infra',
  ];
  const topics = [
    'Upgrade to Node 24',
    'Investigate memory leak',
    'Add rate limiting',
    'Write migration guide',
    'Tidy CI caching',
    'Explain the event loop lag',
    'Refactor feature flags',
    'Speed up test suite',
    'Localise the settings page',
    'Review dependency licences',
  ];
  const history: SessionSummary[] = Array.from({ length: 34 }, (_, i) => {
    const project = projectsList[i % projectsList.length];
    const ended = now - (26 + i * 9) * H;
    return base(now, {
      id: `h-${i}`,
      name: `h-${i}`,
      title: topics[i % topics.length],
      projectName: project,
      cwd: `/Users/marcus/dev/${project}`,
      surface: (['terminal', 'vscode', 'mother-claude', 'jetbrains'] as const)[i % 4],
      owned: i % 4 === 2,
      live: false,
      status: i % 7 === 3 ? 'failed' : 'ended',
      activity: { kind: 'ended', label: 'Ended' },
      model: i % 3 ? SONNET : OPUS,
      startedAt: ended - 50 * M,
      lastActivityAt: ended,
      usage: { input: 10_000 + i * 1000, output: 3000, cacheRead: 40_000, cacheWrite: 5000 },
      costUsd: 0.2 + i * 0.07,
      costFinal: true,
      messageCount: 6 + i,
      lastMessage: 'Done.',
      control: noneCaps(),
    });
  });

  const agents: Record<string, AgentInfo[]> = {
    's-auth': [
      {
        id: 'ag-explore',
        kind: 'subagent',
        sessionId: 's-auth',
        depth: 0,
        type: 'Explore',
        label: 'Map every caller of validateSession',
        status: 'running',
        background: false,
        model: 'Haiku 4.5',
        startedAt: now - 2 * M,
        lastActivityAt: now - 4 * S,
        activity: {
          kind: 'tool',
          label: 'Running Grep',
          tool: 'Grep',
          detail: 'validateSession\\(',
          since: now - 6 * S,
        },
        tokens: 38_200,
        toolUses: 14,
        toolUseId: 'toolu_explore',
        taskId: 'task-explore',
        canStop: true,
      },
      {
        id: 'ag-tests',
        kind: 'bash',
        sessionId: 's-auth',
        depth: 0,
        label: 'npm run test:watch',
        status: 'running',
        background: true,
        startedAt: now - 18 * M,
        activity: { kind: 'background', label: 'Watching tests', detail: '142 passed, 0 failed' },
        taskId: 'bash-tests',
        canStop: true,
      },
      {
        id: 'ag-review',
        kind: 'subagent',
        sessionId: 's-auth',
        depth: 0,
        type: 'code-reviewer',
        label: 'Review the token rotation diff',
        status: 'done',
        background: false,
        startedAt: now - 20 * M,
        endedAt: now - 16 * M,
        tokens: 22_000,
        toolUses: 9,
        canStop: false,
      },
    ],
    's-audit': [
      {
        id: 'wf-scan',
        kind: 'workflow-agent',
        sessionId: 's-audit',
        depth: 0,
        label: 'Scan npm workspaces',
        phase: 'Scan',
        workflowRunId: 'wf-deps',
        status: 'running',
        background: true,
        model: 'Haiku 4.5',
        startedAt: now - 4 * M,
        activity: {
          kind: 'tool',
          label: 'Running Bash',
          tool: 'Bash',
          detail: 'npm ls --all --json',
          since: now - 20 * S,
        },
        tokens: 12_400,
        toolUses: 6,
        canStop: false,
      },
      {
        id: 'wf-advisories',
        kind: 'workflow-agent',
        sessionId: 's-audit',
        depth: 0,
        label: 'Check security advisories',
        phase: 'Scan',
        workflowRunId: 'wf-deps',
        status: 'running',
        background: true,
        model: 'Haiku 4.5',
        startedAt: now - 4 * M,
        activity: {
          kind: 'tool',
          label: 'Running WebFetch',
          tool: 'WebFetch',
          detail: 'github.com/advisories',
          since: now - 9 * S,
        },
        tokens: 9_100,
        toolUses: 4,
        canStop: false,
      },
      {
        id: 'wf-triage',
        kind: 'workflow-agent',
        sessionId: 's-audit',
        depth: 0,
        label: 'Triage findings',
        phase: 'Triage',
        workflowRunId: 'wf-deps',
        status: 'running',
        background: true,
        model: 'Haiku 4.5',
        startedAt: now - 60 * S,
        activity: { kind: 'thinking', label: 'Thinking', since: now - 15 * S },
        tokens: 3_300,
        toolUses: 1,
        canStop: false,
      },
    ],
    's-docs': [
      {
        id: 'ag-writer',
        kind: 'subagent',
        sessionId: 's-docs',
        depth: 0,
        type: 'general-purpose',
        label: 'Rewrite the quick-start guide',
        status: 'running',
        background: false,
        startedAt: now - 95 * S,
        activity: {
          kind: 'tool',
          label: 'Editing',
          tool: 'Edit',
          detail: 'docs/quick-start.md',
          since: now - 3 * S,
        },
        tokens: 18_600,
        toolUses: 11,
        canStop: false,
      },
      {
        id: 'ag-samples',
        kind: 'subagent',
        sessionId: 's-docs',
        parentId: 'ag-writer',
        depth: 1,
        type: 'Explore',
        label: 'Check the code samples compile',
        status: 'running',
        background: false,
        startedAt: now - 40 * S,
        activity: {
          kind: 'tool',
          label: 'Running Bash',
          tool: 'Bash',
          detail: 'npx tsc --noEmit samples/*.ts',
          since: now - 11 * S,
        },
        tokens: 4_200,
        toolUses: 3,
        canStop: false,
      },
    ],
  };

  const workflows: Record<string, WorkflowRun[]> = {
    's-audit': [
      {
        runId: 'wf-deps',
        name: 'dependency-audit',
        status: 'running',
        phases: [
          { title: 'Scan' },
          { title: 'Triage' },
          { title: 'Fix', detail: 'Open an upgrade PR' },
        ],
        agents: { total: 3, running: 3, done: 0, failed: 0 },
        startedAt: now - 4 * M,
        totalTokens: 24_800,
      },
    ],
  };

  const transcripts: Record<string, UiEvent[]> = {
    's-auth': [
      ev('a0:0', now - 52 * M, 'system', 'compact', {
        text: 'Conversation compacted · 212k → 38k tokens',
      }),
      ev('a1:0', now - 50 * M, 'user', 'prompt', {
        text: 'Move session validation into middleware and add token rotation. Keep the public API unchanged.',
      }),
      ev('a1:1', now - 50 * M, 'user', 'image', {
        image: { mediaType: 'image/png', ref: '/api/sessions/s-auth/media/0/a1/1' },
      }),
      ev('a2:0', now - 50 * M, 'assistant', 'thinking', {
        messageId: 'msg_a2',
        text: 'The validation is duplicated in 11 handlers. A middleware keeps it in one place; rotation needs a refresh-token table.',
      }),
      ev('a3:0', now - 49 * M, 'assistant', 'text', {
        messageId: 'msg_a3',
        text: "I'll start by finding every place that validates sessions, then introduce `requireSession()` middleware.",
      }),
      ev('a4:0', now - 49 * M, 'assistant', 'tool', {
        tool: {
          id: 'toolu_g1',
          name: 'Grep',
          summary: 'validateSession\\(',
          input: { pattern: 'validateSession\\(', path: 'src' },
        },
      }),
      ev('a5:0', now - 49 * M, 'user', 'result', {
        result: {
          toolUseId: 'toolu_g1',
          isError: false,
          text: 'src/routes/users.ts:14\nsrc/routes/orders.ts:9\n… 9 more',
        },
      }),
      ev('a5b:0', now - 48 * M, 'assistant', 'tool', {
        tool: {
          id: 'toolu_r1',
          name: 'Read',
          summary: 'src/auth/index.ts',
          input: { file_path: '/Users/marcus/dev/api-server/src/auth/index.ts' },
        },
      }),
      ev('a5c:0', now - 48 * M, 'user', 'result', {
        result: {
          toolUseId: 'toolu_r1',
          isError: false,
          text: 'export function validateSession(req) { … }',
        },
      }),
      ev('a5d:0', now - 47 * M, 'assistant', 'tool', {
        tool: {
          id: 'toolu_w1',
          name: 'Write',
          summary: 'src/middleware/session.ts',
          input: {
            file_path: '/Users/marcus/dev/api-server/src/middleware/session.ts',
            content:
              'export function requireSession() {\n  return (req, res, next) => {\n    if (!req.session?.valid) return res.sendStatus(401);\n    next();\n  };\n}\n',
          },
        },
      }),
      ev('a5e:0', now - 47 * M, 'user', 'result', {
        result: { toolUseId: 'toolu_w1', isError: false, text: 'File created successfully.' },
      }),
      ev('a5f:0', now - 46 * M, 'assistant', 'tool', {
        tool: {
          id: 'toolu_b1',
          name: 'Bash',
          summary: 'npm test -- --silent',
          input: { command: 'npm test -- --silent', description: 'Run the test suite' },
        },
      }),
      ev('a5g:0', now - 46 * M, 'user', 'result', {
        result: {
          toolUseId: 'toolu_b1',
          isError: true,
          text: 'FAIL src/routes/orders.test.ts\n  ● GET /orders › returns 401 without a session\n\nTests: 1 failed, 141 passed',
        },
      }),
      ev('a5h:0', now - 45 * M, 'assistant', 'tool', {
        tool: {
          id: 'toolu_e1',
          name: 'Edit',
          summary: 'src/routes/orders.ts',
          input: {
            file_path: '/Users/marcus/dev/api-server/src/routes/orders.ts',
            old_string: 'router.get("/orders", handler);',
            new_string: 'router.get("/orders", requireSession(), handler);',
          },
        },
      }),
      ev('a5i:0', now - 45 * M, 'user', 'result', {
        result: {
          toolUseId: 'toolu_e1',
          isError: false,
          patch: {
            path: 'src/routes/orders.ts',
            hunks: [
              {
                oldStart: 7,
                oldLines: 3,
                newStart: 7,
                newLines: 3,
                lines: [
                  ' import { handler } from "./handler";',
                  '-router.get("/orders", handler);',
                  '+router.get("/orders", requireSession(), handler);',
                  ' export default router;',
                ],
              },
            ],
          },
        },
      }),
      ev('a5j:0', now - 44 * M, 'assistant', 'question', {
        question: {
          questions: [
            {
              question: 'How long should refresh tokens live?',
              header: 'Rotation',
              multiSelect: false,
              options: [{ label: '7 days' }, { label: '30 days' }, { label: '90 days' }],
            },
          ],
          answers: { 'How long should refresh tokens live?': '30 days' },
        },
      }),
      ev('a5k:0', now - 43 * M, 'system', 'decision', {
        text: "Allowed by Marcus's iPhone · Always allow Bash(npm test:*)",
      }),
      ev('a6:0', now - 30 * M, 'assistant', 'todos', {
        todos: [
          { content: 'Add requireSession() middleware', status: 'completed' },
          { content: 'Replace inline validation in 11 handlers', status: 'completed' },
          { content: 'Add refresh_tokens table', status: 'completed' },
          { content: 'Rotate tokens on refresh', status: 'completed' },
          {
            content: 'Run the database migration',
            status: 'in_progress',
            activeForm: 'Running the database migration',
          },
          { content: 'Update API docs', status: 'pending' },
          { content: 'Open the PR', status: 'pending' },
        ],
      }),
      ev('a6b:0', now - 20 * M, 'system', 'task', {
        task: {
          taskId: 'bash-lint',
          status: 'completed',
          summary: 'npx eslint . --cache: no problems',
        },
      }),
      ev('a6c:0', now - 3 * M, 'user', 'relayed', {
        text: 'Keep the old /session endpoint working until the mobile app ships',
      }),
      ev('a7:0', now - 60 * S, 'assistant', 'text', {
        messageId: 'msg_a7',
        text: 'Middleware is in place and all **142 tests pass**. Next I need to apply the migration for the new `refresh_tokens` table.',
      }),
      ev('a8:0', now - 45 * S, 'assistant', 'tool', {
        tool: {
          id: 'toolu_auth_1',
          name: 'Bash',
          summary: 'npm run migrate -- --force',
          input: { command: 'npm run migrate -- --force' },
        },
      }),
    ],
    's-stream': [
      ev('st1:0', now - 6 * M, 'user', 'prompt', {
        text: 'Profile the index build and make it at least 2x faster',
      }),
      ev('st2:0', now - 5 * M, 'assistant', 'text', {
        messageId: 'msg_st2',
        text: 'The profile shows **68%** of the time in `tokenize()`, which re-compiles its regex for every document.',
      }),
    ],
    's-db': [
      ev('d1', now - 25 * M, 'user', 'prompt', {
        text: 'We need an analytics backend. Compare options and set up the schema.',
      }),
      ev('d2', now - 20 * M, 'assistant', 'text', {
        text: 'Here is a quick comparison of the three realistic options…',
      }),
    ],
    's-billing': [
      ev('b1', now - 40 * M, 'user', 'prompt', {
        text: 'Plan the migration of billing to Stripe API v3.',
      }),
      ev('b2', now - 6 * M, 'assistant', 'plan', { plan: { text: MIGRATION_PLAN } }),
    ],
    's-docs': [
      ev('w1', now - 14 * M, 'user', 'prompt', { text: 'Polish the docs site before the launch.' }),
      ev('w2', now - 95 * S, 'assistant', 'tool', {
        tool: { id: 'toolu_w', name: 'Agent', summary: 'Rewrite the quick-start guide' },
      }),
      ev('w3', now - 30 * S, 'user', 'relayed', {
        text: 'Also fix the broken link in the footer (from your phone)',
      }),
    ],
  };

  const agentTranscripts: Record<string, UiEvent[]> = {
    'ag-explore': [
      ev('x1', now - 2 * M, 'user', 'prompt', {
        agentId: 'ag-explore',
        text: 'Find every caller of validateSession and report the file and line.',
      }),
      ev('x2', now - 100 * S, 'assistant', 'tool', {
        agentId: 'ag-explore',
        tool: { id: 'toolu_x1', name: 'Grep', summary: 'validateSession\\(' },
      }),
      ev('x3', now - 99 * S, 'user', 'result', {
        agentId: 'ag-explore',
        result: { toolUseId: 'toolu_x1', isError: false, text: '11 matches in 9 files' },
      }),
      ev('x4', now - 6 * S, 'assistant', 'text', {
        agentId: 'ag-explore',
        text: 'Checking the two dynamic call sites in `src/legacy/`…',
      }),
    ],
  };

  const projects: ProjectInfo[] = projectsList.map((name, i) => ({
    name,
    path: `/Users/marcus/dev/${name}`,
    lastActivityAt: now - i * 7 * H,
  }));

  const tasks: Record<string, BackgroundTask[]> = {
    's-auth': [
      {
        id: 'bash-tests',
        kind: 'bash',
        label: 'npm run test:watch',
        status: 'running',
        startedAt: now - 18 * M,
        outputAvailable: true,
      },
      {
        id: 'bash-lint',
        kind: 'bash',
        label: 'npx eslint . --cache',
        status: 'completed',
        startedAt: now - 30 * M,
        endedAt: now - 29 * M,
        outputAvailable: true,
      },
      {
        id: 'mon-logs',
        kind: 'monitor',
        label: 'tail -f logs/api.log',
        status: 'running',
        startedAt: now - 25 * M,
        outputAvailable: true,
      },
    ],
  };
  const queued: Record<string, QueuedMessage[]> = {
    's-stream': [
      {
        id: 'q-1',
        text: 'Also keep the old index format readable for one release',
        createdAt: now - 20 * S,
        via: 'owned',
        status: 'queued',
        cancellable: true,
      },
    ],
    's-docs': [
      {
        id: 'q-2',
        text: 'Use British spelling in the new guide',
        createdAt: now - 50 * S,
        via: 'hook-busy',
        status: 'queued',
        cancellable: true,
      },
    ],
  };
  const terminals: Record<string, string> = {
    's-billing': [
      '╭──────────────────────────────────────────────────────────────╮',
      '│ ✻ Welcome to Claude Code!                                     │',
      '╰──────────────────────────────────────────────────────────────╯',
      '',
      '> Plan the migration of billing to Stripe API v3.',
      '',
      '⏺ Here is the plan. Would you like to proceed?',
      '',
      '  ❯ 1. Yes, and auto-accept edits',
      '    2. Yes, and manually approve edits',
      '    3. No, keep planning',
      '',
      '  plan mode on (shift+tab to cycle)',
    ].join('\n'),
    's-audit': [
      '[09:12:03] claude --bg started (job k7m2)',
      '[09:12:05] Running workflow dependency-audit',
      '[09:14:40] Scan: 2 agents running',
      '[09:16:12] Triage: 1 agent running',
      '[09:16:31] Bash npm audit --json',
    ].join('\n'),
  };

  return {
    sessions,
    pendingItems,
    tasks,
    queued,
    terminals,
    history,
    agents,
    workflows,
    transcripts,
    agentTranscripts,
    projects,
    defaults: {
      model: 'opus',
      effort: 'high',
      models: [
        { value: 'opus', label: 'Opus 5.5', description: 'Most capable' },
        { value: 'sonnet', label: 'Sonnet 5', description: 'Fast and capable' },
        { value: 'haiku', label: 'Haiku 4.5', description: 'Fastest' },
      ],
      efforts: ['low', 'medium', 'high', 'xhigh', 'max'],
    },
    remote: {
      lan: {
        addresses: [
          { ip: '192.168.1.23', url: 'https://192.168.1.23:6725' },
          { ip: '10.0.0.5', url: 'https://10.0.0.5:6725' },
        ],
        port: 6725,
        fingerprint: '7F:3A:92:C4:1B:E0:55:8D',
      },
      internet: {
        enabled: true,
        port: 6726,
        publicUrl: 'https://mother-claude-demo.ngrok-free.app',
        requireApproval: true,
        detected: [
          {
            provider: 'ngrok',
            url: 'https://mother-claude-demo.ngrok-free.app',
            upstream: 'http://127.0.0.1:6726',
            inspect: true,
          },
        ],
        commands: {
          ngrok: 'ngrok http 127.0.0.1:6726 --inspect=false',
          cloudflared: 'cloudflared tunnel --url http://127.0.0.1:6726',
          tailscale: 'tailscale funnel 6726',
        },
        warnings: [
          "ngrok's traffic inspector is on: restart ngrok with --inspect=false so request bodies aren't recorded.",
        ],
      },
      bridge: { port: 5612, running: false },
    },
    devices: [
      {
        id: 'dev-iphone',
        name: "Marcus's iPhone · Safari",
        platform: 'ios',
        createdAt: now - 12 * D,
        lastSeenAt: now - 3 * M,
        lastIp: '192.168.1.41',
        ingress: 'lan',
        allowDangerous: false,
      },
      {
        id: 'dev-pixel',
        name: 'Pixel 9 · Chrome',
        platform: 'android',
        createdAt: now - 2 * D,
        lastSeenAt: now - 5 * H,
        lastIp: '203.0.113.7',
        ingress: 'tunnel',
        allowDangerous: false,
      },
    ],
    pendingPairs: [
      {
        id: 'pp-ipad',
        deviceName: 'iPad · Safari',
        platform: 'ios',
        ip: '198.51.100.23',
        ingress: 'tunnel',
        target: 'internet',
        fingerprint: '4821',
        createdAt: now - 20 * S,
        expiresAt: now + 280 * S,
      },
    ],
    foreign: {
      installed: true,
      pluginVersion: '1.0.0',
      cliAvailable: true,
      lastEventAt: now - 4 * S,
      sessionsWithHooks: 3,
      notes: ['Sessions started before the plugin was installed pick it up after a restart.'],
    },
    pushPrefs: { permission: true, question: true, plan: true, finished: false, failed: true },
    bridge: {
      enabled: true,
      running: false,
      port: 5612,
      url: 'http://127.0.0.1:5612',
      requireToken: false,
      hostBuilt: true,
      hostRunning: false,
      messagesApi: true,
      claudeAuthenticated: true,
      claudeAccount: 'marcus@example.com (Max)',
      activeOperations: 0,
      defaults: {},
      threads: [],
    },
    diff: {
      isRepo: true,
      repoRoot: '/Users/marcus/dev/api-server',
      branch: 'feat/session-tokens',
      head: 'a1b2c3d',
      files: [
        { path: 'src/middleware/session.ts', status: 'added', additions: 58, deletions: 0 },
        { path: 'src/routes/users.ts', status: 'modified', additions: 4, deletions: 19 },
        {
          path: 'migrations/0042_refresh_tokens.sql',
          status: 'added',
          additions: 21,
          deletions: 0,
        },
      ],
      additions: 83,
      deletions: 19,
      commits: [
        { id: 'a1b2c3d', summary: 'Add refresh token table', author: 'Claude', time: now - H },
      ],
      worktrees: [],
    },
  };
}

export function ev(
  id: string,
  ts: number,
  role: UiEvent['role'],
  kind: UiEvent['kind'],
  rest: Partial<UiEvent> = {},
): UiEvent {
  return { id, ts, role, kind, ...rest };
}
