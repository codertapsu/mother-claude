# Teams of sessions

A **team** is a group of Claude Code sessions working on one project: a
**lead** plans the work and writes **task packets** on Mother Claude's task
**board**, and **role sessions** (frontend, backend, scout, writer, …) do
them, each in its own files. Mother Claude starts, wakes and stops the
sessions it runs as capacity allows, tells the lead what changed on the
board, and holds every role to the files it owns.

This is Phase 2 of [PLAN-BROWSER-AND-TEAMS.md](PLAN-BROWSER-AND-TEAMS.md)
(§6, §8). The model router, admission control and the outcome records are
the scheduler's (`orchestration/{router,capacity,metrics}.rs`); the teams
side calls `router::choose`, `capacity::admit` and `metrics::record_outcome`
and nothing else of it.

Everything marked **verified** was checked against Claude Code 2.1.319 on
2026-11-06 (the spike, at the end of this document).

## Concepts

| | What it is | Where it lives |
|---|---|---|
| Template | Roles and how their work comes together; data, not code | `<app data dir>/workflows/<id>.json` (0600), plus five built-ins |
| Team | A template applied to a project folder, an account and per-role overrides; one per project | `<app data dir>/teams.json` (0600) |
| Role | A template's role: charter, model, effort, permission mode, worktree, path globs, read-only, budget, autostart | inside the team (a snapshot of the template as started) |
| Role session | The role's Claude Code session: an **owned** one Mother Claude launched, or a **foreign** live session that took the role | the usual session store; named `<project>/<role>` |
| Board | The project's tasks: packets, claims, progress, results, history | `<app data dir>/boards/<hash of the folder>.json` (0600) |

Nothing is ever written in the project or in an account's config directory
by Mother Claude's own code: the files above are all in the app data dir.
The worktrees under `<project>/.claude/worktrees/` are made by Claude Code
itself (`-w`).

**Only owned sessions are started, stopped or resumed** (the hard invariant,
CLAUDE.md). A foreign session that takes a role from the dashboard is guarded
and told about its tasks through the hooks plugin, but Mother Claude never
starts or stops it.

## Templates

```json
{
  "id": "software-delivery",
  "name": "Software delivery",
  "roles": [
    {"id": "lead", "title": "Lead", "lead": true, "charter": "…",
     "model": "opus", "effort": "high", "permissionMode": "default",
     "paths": ["docs/**", "*.md"], "autostart": true},
    {"id": "frontend", "title": "Frontend", "charter": "…",
     "model": "sonnet", "effort": "medium", "permissionMode": "acceptEdits",
     "worktree": true, "paths": ["src/app/**"], "budgetUsd": 5},
    {"id": "scout", "title": "Scout", "model": "haiku", "effort": "low",
     "permissionMode": "plan", "readOnly": true}
  ],
  "integration": {"branch": "main", "mergeBy": "lead", "gate": "npm test", "review": true},
  "flow": [
    {"from": "scout", "to": "frontend", "note": "what the code already does"}
  ],
  "idleStopMinutes": 30
}
```

Role fields: `id` (lower case, digits, `-`; it names the worktree and is
part of the session name), `title`, `lead` (exactly one), `description`,
`charter` (the role's standing instructions), `model`, `effort`,
`permissionMode` (always passed explicitly; `bypassPermissions` and
`dontAsk` need the dangerous right to start), `worktree`, `paths`,
`readOnly`, `budgetUsd`, `autostart`, `addDirs` (folders relative to the
project, `--add-dir`), `tools` (the role's agent definition's tools when
another session runs it as a subagent), `classes` (task classes it usually
takes), `machine` (the computer it runs on: a paired computer's name or id,
or `os:windows` / `os:macos` / `os:linux`; see *A role on another computer*;
never the lead's). `integration`: `branch`, `mergeBy` (a role), `gate` (the
command the lead runs before merging), `review` (a worker's finished task
waits for the lead; default on). `idleStopMinutes`: 0 never stops an idle
role.

`flow` says how work usually moves on between the roles: each step names
two different roles (`from`, `to`; the lead too) and, at most 200
characters, what passes along (`note`). Nothing moves by itself — the
lead's charter lists the steps ("How work usually moves on: Scout →
Frontend (what the code already does)") and asks it to write the next
role's task from a finished one; a worker's charter says where its
finished work usually goes and what its work builds on. Every built-in
has one (Content production: planner → scriptwriter → designer → producer
→ QA → publisher). `layout` is where the visual editor placed each node
(role ids, `@merge` for the merge node): kept as written, never read by a
team.

Parsing is tolerant — unknown fields are kept, missing ones default — and
validation reports every problem at once (errors block saving and starting,
warnings do not): ids, one lead, model names, effort and mode values,
relative path globs, budgets, `mergeBy` naming a role, a flow's steps
(two different roles, each step once), `machine` (a name or
id of at most 64 characters, or one of the three `os:`; an error on the
lead). A `machine` no paired computer matches is a warning on the template
and a refusal when a team starts with it (`422 machine_unknown`).

Built-ins (fixed; duplicate one to change it): **Software delivery** (lead,
frontend and backend in worktrees, read-only scout), **Content production**
(planner, scriptwriter, designer, producer, QA, publisher), **Research &
analysis** (lead, researcher, synthesiser, fact-checker), **Browser
operations** (operator, read-only verifier) and **Documentation** (editor,
read-only librarian, writer in a worktree). Their path globs are guesses at a
common layout: duplicate the template and set your project's.

## Starting a team

From the Teams page: a project (one of the known projects: a folder with
sessions or one an account trusts), a template, an account (the project's by
default), an optional **brief** (the lead's first message; without one it
starts quietly) and per-role overrides (model, effort, permission mode,
autostart, and — with a computer paired — the **computer** a role runs on:
the template's, this one, a paired computer, or any of an OS). The server checks the folder as for any owned launch (absolute,
existing; trusted for a caller other than the desktop), that it is a git
repository when a role uses a worktree, that the account's environment is
available (a token account's token on this machine), and the dangerous right
when a role would run without permission prompts. One team per project; a
stopped team is resumed, not started again.

## How a team runs

The launcher's loop (`orchestration::launcher::tick`, every 3 s and on every
board change) goes over every running team:

1. **Reconcile**: a role session whose process ended is marked stopped.
2. **The lead** starts when it never ran and autostarts, when a board notice
   waits for it, or when someone asked for it.
3. **A role with work and no live session** starts for its highest-priority
   todo task (or resumes for the task its stopped session held). Its model
   and effort come from `router::choose` (the role's default, the task's
   class, a user override, the escalation count); whether it may start now
   from `capacity::admit`, given every running role session of every team,
   the lead's included (`lead: true`). When the answer is *wait*, the role
   shows **waiting** with the reasons, and a push goes out after 10 minutes.
4. **An idle role session with nothing held** is handed the next task: the
   task is claimed for it and its packet sent as a message.
5. **Idle stop**: a role session that sat idle with nothing to do for the
   template's minutes is stopped (never mid-turn; the lead only when nothing
   is open and no worker runs). Its conversation stays.

A role session is a Path B owned session (`claude/control.rs`), launched with
the team's block of `SpawnOptions` and argv:

| Argument | Why |
|---|---|
| `-n <project>/<role>` | its name: what `SendMessage` and `ListAgents` use (a `/` in a name works, verified) |
| `--agents <app data dir>/teams/<team>/agents.json` | the team's roles as agent definitions (0600), usable as subagents |
| `--append-system-prompt <charter>` | the role's charter and how the team works. Not `--agent`: it **replaces** Claude Code's whole system prompt (verified) |
| `--settings {"crossSessionInbound":"accept"}` | and nothing else: the session takes other sessions' messages unattended |
| `--add-dir <project>/<dir>` | the role's `addDirs` |
| `-w <role>` | a worktree role, when its worktree does not exist yet (then it runs in it) |
| `--max-budget-usd <n>` | only when the account pays per token (the scheduler decides, with your billing setting on Teams › Capacity): the role's `budgetUsd`, else the cap for role sessions on Teams › Capacity ($5 unless you changed or switched it off) |
| `--fallback-model <a,b>` | the routing policy's fallbacks for the chosen model's family (by default Opus → Sonnet, Fable → Opus), tried when the model is overloaded or unavailable; never the model it runs on (Claude Code refuses that) |
| `--disallowed-tools Edit,Write,MultiEdit,NotebookEdit` | a read-only role |
| `--allowedTools mcp__plugin_mother-claude_mc__task_create,…` | the ten team tools by exact name, one comma-joined value (as the Agent SDK passes it): Mother Claude decides each call itself, so none raises a permission card. Never the server's name, which would allow its other tools too (the browser's) |
| `--model`, `--effort`, `--permission-mode` | from the router and the role, as for every owned session |
| `initialize.hooks` | the guard's `PreToolUse` callback (below) |

The account environment is the team's account's, exactly as for any owned
launch (`accounts::AccountEnv`). The first message is the role's
**assignment**: its task packet (claimed for it before the process starts),
or the lead's brief and pending notices. A stopped role resumes with
`--resume <id>` in its worktree — a fork (`--fork-session --session-id
<new>`) when another process holds the transcript, the existing rule — and a
task its old session held moves to the new one.

## The board

| Status | Means | Who moves it there |
|---|---|---|
| `todo` | written, nobody holds it | the lead or the user (also: sending back, reopening, giving back) |
| `claimed` | a session holds it | Mother Claude when it hands it over; `task_claim` |
| `in_progress` | the holder works on it | the holder |
| `review` | finished by a worker, waits for the lead | the holder, through `task_done` (when the team reviews) |
| `done` | finished | the lead or the user accepting it; the holder when the team does not review |
| `failed` | could not be done | the holder (`task_done` outcome `failed`), the lead or the user |
| `cancelled` | dropped | the lead or the user |

A task carries its packet (title, goal, acceptance criteria, files,
constraints, priority 0–3, class, role), progress notes, a blocked question,
its result (summary, branch, changed files, whether the gate passed), its
history, the model and effort it ran with and why, and how often it was sent
back for a stronger model. Workers never write tasks (they ask the lead);
only the lead and the user change a packet, a priority, a role or a model.
Every change goes to the dashboard as a `teams` WebSocket frame
(`{t:"teams", teamId, task}`; a team's whole view as `{t:"teams", team}`).

## Tools

The team's sessions reach the board through Mother Claude's MCP server
(`mcp::registry`, served at `/mcp`, docs/MCP.md). Every owned session,
role sessions included, gets it from the plugin loaded inline
(`--plugin-dir`, added to every Path B launch), so the model sees the tools
as `mcp__plugin_mother-claude_mc__<tool>`; a foreign session that takes a
role gets them from the installed plugin. A call is accepted only from the
lead or a role session of a team (the caller's session id, which the server
takes from the calling process, and its working folder inside the project)
— or, for a role on another computer, from its session there, relayed with
the role's binding (*A role on another computer*); anything else gets a
`denied` result that says why. The read-only tools are
marked so (`readOnlyHint`), which keeps their calls out of the audit log.

| Tool | Who | What |
|---|---|---|
| `task_create` | lead | write a packet for one role |
| `task_list`, `task_get` | everyone | the board, one task |
| `task_claim` | a worker, its own role's task | rarely needed: handed tasks are claimed |
| `task_update` | the holder (notes, `in_progress`, blocked, give back); the lead (also priority, role, packet, accept, send back with `escalate`, cancel) | |
| `task_done` | the holder (or the lead) | summary, branch, changed files; outcome `failed` |
| `team_roles`, `team_status` | everyone | who is on the team (session names), states, the queue |
| `team_request_role` | lead | start a role's session now, capacity allowing |
| `session_summary` | lead, read-only | what a role's session is doing: activity, task, last prompt and message, cost |

## How delegation works

* The **lead talks to its roles itself**, with Claude Code's own
  `SendMessage` and `ListAgents`, by session name. Cross-session messages are
  local, on by default, every provider; with `crossSessionInbound: accept` a
  `-p` role session takes one unattended and starts a turn (verified, ~40 ms
  from send to inbox).
* **Workers report on the board** (`task_update`, `task_done`) and may
  message the lead.
* **Mother Claude tells the sessions what changed on the board**, in short
  messages that start with `[Mother Claude · board]`: the lead hears about a
  task ready for review, done, failed or blocked, and about the user's own
  changes; a worker hears when its task is cancelled, given to another role,
  unblocked or annotated. To an owned session through the owned-session
  message path (queued behind a running turn); to a foreign one through the
  hooks relay (at most 8,000 bytes). A lead that is not running keeps its
  notices for its next start, which they bring about.
* **Every end of a task** — done, failed, cancelled, sent back from review (a
  failed gate) or escalated — is recorded with `metrics::record_outcome`
  (task id `<board key>#<number>`).
* **The phone** hears about tasks done, ready for review, failed or blocked,
  and about a role waiting long for capacity (on the *finished* and *failed*
  notification settings; details show the task's title only when turned on).
  The notification opens the team.

A foreign live session can **take a role** from the team page (it must work
in the project, have the Mother Claude plugin active, and hold no other role;
one that runs without permission prompts needs the dangerous right). It gets
the charter and its tasks through the relay, and the guard applies to it.
*Release role* gives the role back (its tasks return to the board).

## The guard

For a role session, an `Edit`, `Write`, `MultiEdit` or `NotebookEdit` whose
target is outside the role's path globs is **denied**, and so is a `Bash` or
`PowerShell` command whose write targets are detectably outside — with the
reason told to the model ("Mother Claude's team guard stopped this Write: the
Frontend role may change only src/app/** (relative to …), and
src-tauri/main.rs is outside them. If the change is needed, ask the lead
(SendMessage to "shop/lead") or say so with task_update."). A read-only role
writes nothing. Each denial is audited (`team.guard.deny`: session, team,
role, tool, target path — never the content or the command) and told to the
desktop; the team page counts them.

* **Globs** are relative to the role's base — its worktree for a worktree
  role, else the project root — and read like Claude Code's own path rules
  (gitignore style): `src/app/**`, `docs/` (the folder and everything in it),
  `*.md` (a pattern without a `/` matches a name at any depth), `?`, `[a-z]`.
  No globs: no limit.
* **Paths** are judged by their own form, on either machine: a Windows path
  (`C:\…`, `C:/…`, `\\server\share\…`, `\\?\…`) without regard to case or
  separator, a Mac's exactly; on this machine's own paths the
  symlink-resolved form must pass too.
* **Shell commands**: redirections (`>`, `>>`, `&>`, `2>`), `tee`, `cp`/`mv`/
  `install`/`rsync` destinations, `rm`, `rmdir`, `touch`, `mkdir`,
  `truncate`, `chmod`/`chown`, `ln`, `sed -i`, `perl -i`, `dd of=`,
  `curl -o`, `wget -O`, and PowerShell's `Set-Content`, `Add-Content`,
  `Out-File`, `New-Item`, `Remove-Item`, `Copy-Item`, `Move-Item`,
  `Rename-Item` and their aliases; `cd` moves the working directory (a
  subshell's stays inside). A target computed by a variable, a glob or a
  command substitution is not detectable and passes; `git` subcommands are
  not read.
* **Nobody's**: `/dev/*`, Windows' `NUL` and, outside the role's base, the OS
  temp folders.

Where it runs: an **owned** role session registers a `PreToolUse` callback in
its `initialize` request (`hooks`, as the Agent SDK does), which the CLI asks
before every matching call **in every permission mode** — `can_use_tool`
alone is never asked for an edit in `acceptEdits` (verified) — and
`can_use_tool` checks again (a denial there raises no card). A **foreign**
role session goes through the plugin's `PreToolUse` decide hook, its working
folder taken from the live roster, never from the hook body.

## Worktrees

`claude -w <role>` (verified with `-p`): Claude Code creates
`<project>/.claude/worktrees/<role>` on a new branch `worktree-<role>` from
`HEAD`, runs the session there (its transcript filed under the worktree's
path), and keeps the worktree when the process exits (git marks it `locked
claude session <role> (pid …)`). `--resume <id>` from the main checkout
resumes in the worktree, and `--resume <id> -w <role>` reuses it. So
Mother Claude launches a worktree role with `-w <role>` the first time and in
the existing worktree after that; it never creates, prunes or removes
worktrees itself. Notes:

* The main checkout sees `.claude/` as untracked: add `.claude/worktrees/` to
  the project's `.gitignore`.
* An edit inside a worktree is a project file there: the approval
  classifier no longer calls it Claude Code configuration (only the
  worktree's own `.claude/`, its `.mcp.json` and its `.git` link are), so a
  worktree role in `default` mode can be approved from a phone. In
  `acceptEdits` its edits do not prompt at all (verified).
* Workers commit in their worktree and never merge; the lead integrates
  (`integration.mergeBy`) after the gate.

## A role on another computer

A role may run on another of your computers ([MACHINES.md](MACHINES.md),
docs/PLAN-MACHINES.md Phase 4 decision 9): a multi-platform app gets its
lead and an `ios` role on the Mac and a `windows` role on the desktop PC.
The template names the computer on the role — `"machine": "DESKTOP-PC"` (a
paired computer's name or id), or `"os:windows"` / `"os:macos"` /
`"os:linux"` (the first online paired computer of that OS that gave this
one *Run*; once the role ran on one, it stays there while that computer is
paired). The lead never runs elsewhere: the team, its board and its
launcher stay on the computer that started it.

* **Starting it.** The launcher starts such a role *there*
  (`machines::work::roles::lead::start`): the computer resolved (Machines
  on here, that computer online, its **Run** permission for this one), a
  **binding** minted the first time — an id and a secret; that computer
  keeps the secret, this one its SHA-256 — the project's code snapshotted
  (commits, staged, unstaged and untracked files; part 2's transfer) and
  sent when that computer will take it, then that computer **admits it with
  its own `capacity::admit`** (every role session it runs, its own memory,
  plan window and caps) and starts it as an owned session in a worktree of
  its own (`machines/work/roles/<binding>/<project>` in its data folder, a
  detached worktree of its checkout at this computer's code — never `-w`).
  Its options are a local role's: its name, charter (saying where it runs
  and that `SendMessage` does not reach the lead), `{"crossSessionInbound":
  "accept"}` and nothing else, the team tools by exact name, the role
  definitions, the guard's callback with the role's globs (relative to the
  worktree there), the mode, model, effort, fallbacks and budget the router
  and the scheduler chose here. It runs under that computer's account of
  the same name as the team's, else its Default. The task is claimed for
  its session before it starts, as for a local role.
* **Waiting and refusals.** When that computer's admission says not now,
  the role waits here with its reasons (*on DESKTOP-PC: 3 role sessions run
  already…*). A refusal — Machines off on either computer, offline, no Run
  permission, no checkout there, *Claude Code tools* off there — is the
  role's last error on the team page, tried again a minute later.
* **Running it.** This computer looks at it every few seconds (what its
  session does feeds the team's view and the loop: hand it the next task,
  idle stop), the board's notices and the next task's packet go to its
  session there through the peer link, and stop, idle stop and resume come
  from this launcher. A resume continues its conversation there (`--resume`,
  in its worktree) when that computer has it. Its worktree is kept between
  starts; a start moves it to the lead's newer code only when it holds
  nothing the lead has not got back and the lead's code holds what came
  back (the lead merged its branch) — otherwise it keeps the role's own
  work and says so. That computer stops it by itself when the lead is not
  heard from for 30 minutes while it sits idle.
* **A message to it while it is stopped.** A message you send its stopped
  session — from this computer's page of it (through the proxy) or on that
  computer — resumes it through this launcher, with its role's options as
  at every start and that computer's admission, then reaches it
  (`lead::resume_with`; from that computer, `POST /peer/teams/roles/wake`
  with the binding, `host::wake_with`). While it waits there the message
  is refused with that computer's reasons (`409 role_waiting`); a
  message with images is sent as before (a stopped session there answers
  that it is not running). From the lead's computer it goes through the
  role's binding — that computer's *Run* permission for this one, as the
  board's notices do; from that computer, the call to the lead's computer
  is accepted only with the binding and its secret (refusals audited,
  `team.remote.refused`). A role without permission prompts needs the
  sender's dangerous right as well (on the lead's computer, a peer's right
  ∧ its *Dangerous* permission).
* **Its board.** The role session's `task_*` and `team_*` tools there act
  on this computer's board: that computer sends each call here with the
  binding (`POST /peer/teams/board`), and this computer answers only a call
  from the computer the binding was minted for, for that team and role,
  with the secret and the role's current session — then under the board's
  rules for a role session here (claims, updates, `task_done`, every end
  to `metrics::record_outcome`).
* **Its work.** When it finishes a task (`task_done`), that computer
  snapshots its worktree — its commits and what it left uncommitted — and
  this computer fetches it into the project as the branch
  **`mother-claude/<computer>/<team>/<role>`**: never checked out, never
  merged, moved only forward (part 2's branch rules), whatever branch the
  role named. The task's result names the branch, and the task records it
  with what it changed (`remote`: the computer, the branch, its commits and
  files); the lead reviews and merges it.
* **Where it counts.** Its session runs on that computer, so it counts in
  that computer's admission and on its Teams › Capacity (*for <the lead's
  computer>*), never here; the team page shows the role *on DESKTOP-PC*,
  its worktree there and its branch. This computer's Teams › Capacity lists
  it apart, under *On your other computers* (`remoteRoles`): where it
  runs, whether it runs, waits there (with that computer's reasons, *on
  DESKTOP-PC: …*) or is stopped, and when it was last heard — never in
  this computer's counts.
* **Only owned sessions** run a role elsewhere: a foreign session can take
  a role only on the team's own computer.

That computer lists the roles it runs under **Computers › Activity › Team
roles**: *Stop* (its lead starts it again when it has work) and *Remove*
(its session, worktree and record; the lead's next start makes a new
binding). Removing the team removes them there too.

## Hooks

The plugin's `hooks.json` adds six observe-only events: `TeammateIdle`,
`TaskCreated`, `TaskCompleted` (Claude Code's own agent teams in interactive
sessions — Sessions › Agents › Teammates lists them; Mother Claude does not
run them),
`PreModelSwitch` / `PostModelSwitch` (the model of a role a foreign session
holds, shown on the team page) and `ConfigChange` (a foreign role session
changing settings is audited, `team.config_change`). They come only from
sessions Mother Claude does not own: in an owned session the plugin's
scripts exit at once (`MOTHER_CLAUDE_OWNED=1`), and an owned role session
reports through its own stream (the guard through `initialize.hooks`). **`WorktreeCreate` and `WorktreeRemove` are
deliberately not registered**: a `WorktreeCreate` hook *replaces* Claude
Code's git worktree creation ("Hook failure or missing path fails creation",
hooks reference; confirmed in the CLI binary), so an observe-only one in the
user-scope plugin would break `-w`, `isolation: "worktree"` and background
sessions everywhere. Worktrees are read from the role sessions instead.

## Who may do what (HTTP)

| Route | Guard |
|---|---|
| `GET /api/team-templates`, `POST …/validate` | any authenticated caller |
| `POST/PUT/DELETE /api/team-templates…`, `…/{id}/duplicate` | `ManageData` (the desktop or a paired device; never the local-script token) |
| `GET/POST /api/teams`, `…/{id}`, `/stop`, `/resume`, `DELETE` (stopped only) | any authenticated caller; starting or resuming a team with a role without permission prompts needs the dangerous right |
| `…/roles/{role}/start`, `/stop`, `/take` (`POST`, `DELETE`) | any authenticated caller; giving a role to a session without permission prompts needs the dangerous right |
| `…/tasks` (`GET`, `POST`), `…/tasks/{n}` (`GET`, `PATCH`) | any authenticated caller: a phone follows, adds, reprioritises, reassigns, accepts and cancels |
| `GET /api/teams/events` | any authenticated caller |

Every change is audited (`team.*`) and a device's is told to the desktop.

## Limits

* The tools reach a session only while Mother Claude's tools are on (the
  inline plugin is added to owned launches then); a foreign session holding
  a role needs the plugin installed for its account (`/reload-plugins` when
  it started before).
* The guard reads what a command names; a write through a script, a build
  tool, `git`, or a computed path is not caught. It is an ownership fence for
  cooperating sessions, not a sandbox (the approval classifier and the
  permission modes remain the security boundary).
* `--max-budget-usd` is a client-side estimate and only applies to accounts
  that pay per token; on a plan the budget shows as *not applied*.
* Foreign sessions cannot be started, stopped or resumed by a team; a dead
  one's role is taken over by an owned session when work arrives.
* A role on another computer cannot `SendMessage` the lead or be messaged
  by it (cross-session messages stay on one computer): the board — notes,
  *blocked*, the notices — is how they talk. While the lead's computer
  cannot be reached, its tools answer so and its work stays in its
  worktree. A role started there under a new binding (removed there, or on
  another computer) brings its work back as new commits that cannot move
  the old branch forward: the task names the newer commit instead.
* The router and admission control are the scheduler's (docs/ROUTING.md):
  each task's model comes from its routing policy, and each start from its
  admission rules; a task handed to a session is reported to it
  (`metrics::task_started`), so its budget alerts while it runs.
* A task created without a class gets one from the scheduler's classifier
  (keyword rules, or Claude Haiku when you switch that on); when nothing
  matches, the role's default model applies.

## Spike (Claude Code 2.1.319, 2026-11-06)

Every run: `--model haiku --max-budget-usd 0.25`, `MOTHER_CLAUDE_OWNED=1`,
a throwaway git repository as the working folder, no bypass. The same checks
run against the real CLI in `cargo test --test teams_live -- --ignored`.

**(a) Cross-session messages to a `-p` worker.** Worker: `claude -p
--input-format stream-json --output-format stream-json -n mc-spike-worker
--settings '{"crossSessionInbound":"accept"}'`, idle, no prompt. Its pidfile
carries `name`, `nameSource: user`, `messagingSocketPath:
/tmp/cc-socks/<pid>.sock`, `peerProtocol: 1`, `peerFeatures: [notify_idle,
reply_across_default_dirs, artifact_yield]`. Lead: `claude -p -n
mc-spike-lead "…"`. `ListAgents` lists the peers (`name [ref] · kind ·
busy|idle · started …`, never itself); `SendMessage {to: "mc-spike-worker",
message, notify_when_idle: true}` answers `success: true` ("in that session's
inbox, not yet read by its Claude"). Timing from the transcripts: the lead's
`SendMessage` at .282 s, the worker's inbox (`queue-operation enqueue`) at
.323 s, the worker's turn began at .336 s — about 40 ms. The worker's reply
reached the lead's inbox in 33 ms and waited for the lead's running turn to
end; the `notify_when_idle` notice came 0.8 s after the worker's turn ended.
A plain `claude -p "…"` lead stays alive while a reply or an idle notice is
due, and runs a turn for each. On the worker's stdout the turn opens with
`{"type":"command_lifecycle","state":"started"}`, then (with
`--replay-user-messages`) a replayed `user` frame with `origin: {kind:
"peer", from, verifiedPeerPid, msg_id, name, fromMode: "prompting", body}`
and the text "Another Claude session sent a message: <cross-session-message
from=… from-name=… from-mode=…>…</cross-session-message>", then the turn,
`result` and `command_lifecycle` "completed" — a turn the host did not
start, which `control.rs` now sees as one. The transcript records it as a
`queue-operation` and a `user` record with `isMeta: true` and the same
`origin`. A name with a `/` (`spikeproj/worker`) works as the address.

**(b) `--agent <name>` with `--agents <file>`** (a file with `-p`): the main
session runs with the definition's tools (`[Read, Glob]` in `system/init`)
and its prompt **replaces Claude Code's whole system prompt** — the recorded
`prompt_snapshot` is only the agent's prompt; the definition's
`permissionMode: plan` was not applied (the session came up in the user's
default mode). The definition's model could not be told apart (every run
passes `--model haiku`). With `--append-system-prompt` the default prompt
stays and the charter is appended. So role sessions get `--agents` and the
charter through `--append-system-prompt`, never `--agent`.

**(c) `-w <name>` with `-p`**: see *Worktrees* above (location, branch
`worktree-<name>`, kept after exit and locked with the dead pid, transcript
under the worktree's path, `--resume` from the main checkout resumes in the
worktree, `--resume … -w <name>` reuses it).

**(d) `initialize.hooks`**: `{"subtype":"initialize","hooks":{"PreToolUse":
[{"matcher":"Edit|Write|…|Bash","hookCallbackIds":["mc_guard"]}]}}` makes the
CLI send `{"subtype":"hook_callback","callback_id":"mc_guard","input":
{session_id, cwd, permission_mode, hook_event_name: "PreToolUse",
tool_name, tool_input, tool_use_id}, "tool_use_id"}` before every matching
call; answering `{"hookSpecificOutput":{"hookEventName":"PreToolUse",
"permissionDecision":"deny","permissionDecisionReason":"…"}}` blocks it and
the model reads the reason, `{}` lets it run. In `acceptEdits` neither Write
reached `can_use_tool`.

**(e)** An edit inside a worktree under `.claude/worktrees/` in
`acceptEdits` does not prompt.

**(f) `WorktreeCreate`**: the hooks reference and the CLI binary agree that
the hook replaces git worktree creation and that a hook printing no path
fails it — not registered (*Hooks* above).
