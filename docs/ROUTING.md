# Routing, admission control and budgets

The scheduler side of teams of sessions (docs/PLAN-BROWSER-AND-TEAMS.md §7,
§9): which model and effort a task runs on, whether a role session may start
now and why it waits, what tasks cost and how they ended, and budgets. Code:
`src-tauri/src/orchestration/` (`router.rs`, `capacity.rs`, `metrics.rs` are
the contract; `host.rs`, `limits.rs`, `policy.rs`, `classify.rs`,
`outcomes.rs`, `budget.rs`, `reports.rs`, `scheduler.rs`, `capacity_api.rs`
behind them), `src-tauri/src/claude/index/window.rs` (a session's usage
between two times). UI: **Teams › Capacity** (`#/teams/capacity`).

The teams side calls three functions and nothing else of this side; this side
never launches, stops or messages a session.

| Entry point | Called | Answers |
|---|---|---|
| `router::choose(state, &TaskFacts)` | when a role session starts or gets a task | `Decision { choice: {model, effort}, reason }` |
| `capacity::admit(state, &StartRequest, &[RunningWorker])` | before a role session is launched, with every role session running (the lead too) | `Admit`, or `Wait { reasons }` in the user's words |
| `metrics::record_outcome(state, &TaskOutcome)` | whenever a task ends, in any way (a failed gate is `Failed` with `gatePassed: false`) | nothing; the end is stored and learned from |

Optional helpers for the launcher: `budget::session_cap_usd(state, project,
role)` (the `--max-budget-usd` to pass, default **$5**, `None` when you turned
it off), `budget::pays_per_token(state, account, project)` (whether to pass
it), `router::escalation_for(state, task_id)` (steps a task earned from its
failed gates), `router::fallback_for(state, &choice)` (`--fallback-model`),
`classify::classify(title, goal)` / `classify::classify_task(state, title,
goal)` (a class for a task that has none), `metrics::task_started(state,
task_id, project, role, session_id)` (a running task's budget then alerts
live).

## Admission: may a role session start now?

`admit = min(hardware cap, plan-window cap, repository cap, user cap)`. Every
rule that holds adds its reason, so the page shows all of them at once.

| Rule | Who it holds | Where the number comes from | Default |
|---|---|---|---|
| A rate-limit **rejection** that has not reset | everyone, the lead too | the newest `rate_limit_event` of the account's owned sessions (`status: rejected`, `resetsAt`), or a hooked session's `StopFailure` on a rate limit | until `resetsAt`, else 5 min |
| A **warning** about the 5-hour limit | workers | `rate_limit_event` `status: allowed_warning` with `rateLimitType: five_hour` (or none) | 5 min |
| The **5-hour window** | workers at `leadOnlyPct`, everyone at `stopPct` | `claude::usage` for the account the session runs under (the request's, else the project's default account) | 85 %, 95 % |
| **Your cap** | workers (the lead is not counted) | `userCap` | 3 |
| **The project's cap** | workers in the same project folder | `projectCaps[project]`, else `projectCap` | 4 |
| **Memory** | everyone (a lone session always starts) | `floor((free − reserve) / estimate)` more fit; the cap is running + that, between 1 and 16 | reserve 2 GB, estimate measured |
| A **spent daily budget** | workers in that project | what every session in the project's folder spent today ≥ its budget | no budget |
| **Priority** | a worker | a waiting lead, or a waiting request with a higher priority (P0 highest) that waits only for a free slot and asked in the last 90 s, takes the slot first | — |

Memory, from `sysinfo` every 5 s: *free* is the memory a new process can get
without swapping (`available_memory`: free plus reclaimable caches); the
*estimate* is the 75th percentile of the resident memory of the live Claude
Code processes the roster knows (their pidfiles' pids), clamped to
100 MB – 8 GB, 600 MB until one was measured, or `sessionMb` when set.
Until the first sample the memory rule holds nothing up. The load average is
shown but not used: the work runs remotely, builds and tests are what load
the machine.

The weekly windows do not hold anything up; they feed the router.

The last decision per (project, role) is kept with its reasons (a waiting one
for 10 minutes after its last ask, a started one for 2 minutes), and the role
sessions teams reports running are kept while they live. That is how
Teams › Capacity shows who waits and why without knowing the teams code.

A team role that runs on another computer (docs/TEAMS.md "A role on another
computer") is admitted by **that** computer, with its own settings and every
role session it runs — its own teams' and the ones it runs for others
(`RunningWorker::for_machine`, the lead's computer's name) — and counts only
there: its Teams › Capacity lists it *for <the lead's computer>*, the lead's
computer neither counts nor lists it (its team page says where it runs and,
while it waits, that computer's reasons). The router still chooses its
model and effort on the lead's computer.

## Routing: which model and effort

In order of precedence:

1. **Your override** (`TaskFacts.userOverride`, on the task or the role):
   always wins, Fable included. Reason: "set by you".
2. **Escalation**: the task's own steps (`TaskFacts.escalation`) or its class's
   steps in this project, whichever is higher, climb the ladder from the base
   choice (the highest rung at or below it). The ladder climbs effort before
   model. Fable rungs are skipped unless `fableOnEscalation`.
3. **The budget gate**, for Opus-tier work (Opus and Fable): while the account's
   Opus weekly window (`seven_day_opus`; the weekly window when the plan has no
   Opus one) is at `opusGatePct` or more, or the project's daily budget is
   `budgets.nearPct` spent, the task runs on `gateChoice` (Sonnet at high
   effort). Fable from anything but your override runs as Opus at the same
   effort. Escalation (2) comes before the gate: a task that failed its gates
   keeps the stronger model.
4. **The task class** → the policy's model and effort.
5. **The role's default**, else Sonnet at medium.

Default policy (`routing-policy.json`, the plan's first template):

| Classes | Model · effort |
|---|---|
| plan, architecture, review | Opus · high |
| implement, test, fix, ops | Sonnet · medium |
| search, summarize, classify | Haiku · low |
| other | the role's default |

Ladder: Haiku·low → Haiku·high → Sonnet·medium → Sonnet·high → Opus·high →
Opus·xhigh → (Fable·high, only with `fableOnEscalation`). Fallbacks
(`--fallback-model`): opus → sonnet, fable → opus. A family never falls back
to itself, and a role session's fallbacks leave out the model it runs on:
Claude Code refuses to start when a fallback is its main model.

**Escalation state** is learned from recorded outcomes (rebuilt from the log
at startup):

* every `gatesPerStep` (2) failed gates of a task (`gatePassed: false`) earn it
  one step, and the reason is recorded ("2 failed gates: escalated to 1 step");
  `router::escalation_for` returns them for `TaskFacts.escalation`;
* a task's escalation (from its gates, or an `Escalated` end) raises its
  class's level in that project to the task's steps;
* `cleanToStepDown` (2) clean completions of the class (done, gate not failed,
  never escalated) take the class one step back down.

Every decision carries its reason; the last 50 are on Teams › Capacity. The
policy is edited on the desktop (validated: known models and efforts, each
rung above the one before, the gate cheaper than Opus) and reset by removing
the file.

## Task classes for tasks without one

`classify::classify` (the default): whole-word keywords in the title (counted
twice) and the goal; the most hits win, ties to the more specific class (fix
before implement). No keyword: `other`, which runs on the role's default.

The optional **Haiku classifier** (`haikuClassifier`, off by default, desktop
only): `classify::classify_task` asks `claude-haiku-5-5` (thinking off, effort
low, 16 tokens, one class name) through the bridge's direct Messages API client
— the existing path that already calls Claude — so it needs an Anthropic API
key (`ANTHROPIC_API_KEY` when Mother Claude starts). Its calls, tokens and
estimated cost are counted and shown. Any failure falls back to the keywords,
saying why.

## Outcomes, costs and reports

`record_outcome` stores each end in `<app data dir>/outcomes.jsonl` (0600, one
record per line; at 4 MB it becomes `outcomes.1.jsonl`, the one before is
dropped; 20,000 kept in memory). Three seconds later, off the runtime, it
joins the end with the session's token usage: the main and subagent
transcripts the index found, read a line at a time, every API message counted
once (the last record's usage, the first record's time) when it started
between `startedAt` and `endedAt` + 5 s. The record is stored again with the
usage, the estimated cost, the window share and a note when something is
missing ("no transcript found", "part of the transcript could not be read",
messages on a model without a price).

**Prices** (`claude/pricing.rs`, first-party per MTok, input / output, cache
read; cache writes 1.25× input for 5 minutes, 2× for 1 hour): Fable 5.1
$10 / $50 / $0.25, Opus 5.5 $4 / $20 / $0.20, Sonnet 5.5 $2 / $10 / $0.20,
Haiku 5.5 $0.10 / $0.50 / $0.01 (prompts up to 100K tokens; longer ones cost
more and are not modelled); ×1.1 for US-only inference, $0.01 per web search.
**Every dollar figure is an estimate.** On plan accounts (Pro, Max, Team) the
work is paid in usage windows; their dollars are what it would have cost
through the API.

**Window share** (plan accounts): how far the account's 5-hour window moved
while the task ran, times the task's part of what the account spent then —
`(pct_end − pct_start) × min(1, task_cost / account_spend_in_between)`, from a
per-account series taken once a minute. Unknown when the window started over,
did not move, or the app was not running when the task began.

**Report** (`GET /api/routing/report?days=`, and CSV): every task end is an
attempt in the group of the model and effort it ran on; the per-class rows add
every attempt of the class up (what a completed task cost all told, cascades
included).

| Figure | Meaning |
|---|---|
| tasks / completed / failed / escalated / cancelled | task ends by result |
| est. cost, per completed | estimated USD of the ends with usage; ÷ completed |
| tokens each | all tokens (input, output, cache reads and writes) ÷ completed |
| cache hits | cache reads ÷ all input tokens (input + cache reads + cache writes) |
| escalation rate | escalated ends ÷ all ends |
| gate pass rate | passed ÷ gates run |
| window share per completed | estimated 5-hour window points ÷ completed |

## Budgets

Estimated dollars; alerts at `budgets.nearPct` (80 %) and 100 %, as a desktop
notice and a push notification (`alerts.desktop`, `alerts.push`); each budget
alerts once per threshold (a project's once per threshold per day).

* **A project per day** (`budgets.projects[folder]`, else
  `budgets.projectDailyUsd`; none by default): what every session in the
  project's folder (worktrees under it included) spent since local midnight —
  the sessions' `costToday`. At 100 % no more workers start there (the lead
  still does); from the near threshold Opus work there runs on Sonnet.
* **A role session** (`budgets.roles[role]`, else `budgets.sessionUsd`, $5):
  `session_cap_usd` is the `--max-budget-usd` the launcher passes on accounts
  that pay per token; Claude Code enforces it with its own client-side
  estimate. Watched here from the session's cost since teams first reported
  it; on plan accounts it is shown, not enforced.
* **A task** (`budgets.taskUsd`, none by default): live when the task was
  reported with `task_started`, else checked when it ends.

**Who pays per token**: your setting (`billing[account]`), else the account's
sign-in (`claude auth status`): a cloud provider, an API key or a Console
sign-in pay per token; a claude.ai plan and a `setup-token` account pay
through the plan's windows; anything else is not known (no cap passed).

## Settings (`<app data dir>/scheduler.json`, 0600)

| Key | Default | |
|---|---|---|
| `userCap` | 3 | role sessions at once, the lead not counted (1–16) |
| `projectCap`, `projectCaps` | 4, {} | per repository (1–16) |
| `reserveMb` | 2048 | memory always free |
| `sessionMb` | null | per role session; null: measured |
| `leadOnlyPct`, `stopPct` | 85, 95 | the 5-hour thresholds |
| `opusGatePct` | 85 | the Opus weekly gate |
| `budgets` | see above | `projectDailyUsd`, `projects`, `taskUsd`, `sessionUsd` (5), `roles`, `nearPct` (80) |
| `alerts` | desktop and push | |
| `haikuClassifier` | false | desktop only |
| `billing` | {} | account → `perToken` / `plan`; desktop only |

Read field by field (a bad value keeps its default; unknown keys are kept); an
update names only what changes (`null` turns an optional cap off, maps are
replaced whole) and is refused with the field that is wrong.

## HTTP

| Route | Who |
|---|---|
| `GET /api/host`, `GET /api/capacity` | any signed-in caller |
| `PUT /api/capacity/settings` | the desktop or a paired device (never the local-script token); `haikuClassifier` and `billing` desktop only; a device's change is named on the desktop |
| `GET /api/routing/policy` | any signed-in caller |
| `PUT`, `DELETE /api/routing/policy` | the desktop |
| `GET /api/routing/report`, `/api/routing/report.csv?rows=groups\|outcomes` | any signed-in caller |
| `POST /api/routing/classify {title, goal, haiku?}` | any signed-in caller; `haiku: true` the desktop or a paired device, and only when it is on |

## Tuning

1. Run on the defaults for a while; thresholds change on data, not hunches.
2. Read the **per-class** rows first: what a completed task costs all told,
   escalations included. A cheaper model that escalates often is not cheaper.
3. Try **effort before model** (Anthropic's guidance): the top model at a lower
   effort often beats a cascade, and one model keeps one prompt cache (caches
   are model-scoped; the cache-hit column shows what a cascade loses).
4. A class that escalates a lot: give it the rung it ends up on. One that never
   fails its gate: try a rung lower and watch the gate pass rate.
5. On plan accounts watch the window share per completed task; on
   pay-per-token accounts the dollars and the session cap.
6. Turn the Haiku classifier on only when tasks often arrive without a class
   and the keywords misjudge them; its cost is shown.
