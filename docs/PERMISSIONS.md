# Permissions: what Claude may do without asking

When a session asks to do something — run a command, edit a file, fetch a
page, run a command or hand a task to another computer — Mother Claude
shows a card. **Allow** answers it once. **Allow for…** answers it and
remembers the answer as a *permission*, so the same kind of request is
allowed from then on without a card. Everything else — and everything
dangerous — still asks. Permissions are listed, paused, changed and
removed on the **Permissions** page.

Code: `src-tauri/src/pending/rules.rs` (the rules, matching, the store),
`pending::raise` (applying them), `pending::respond` (saving one),
`server/routes/permissions.rs` (the page's routes); the card is
`shared/pending-card.component`, the page `pages/permissions`.

## What a permission names

| Part | Choices |
|---|---|
| **Kind** | a session's tool call — by family: `Bash`, `PowerShell`, `Edit` (Edit, Write, MultiEdit, NotebookEdit), `Read` (Read, Glob, Grep, LS), `WebFetch`, `WebSearch`, or one exact tool such as `mcp__github__create_issue` · a **run**: a command a session runs on another computer (`machine_run`) · a **task** handed to another computer (`machine_task`) |
| **Where** | **this session** · **this project** (every session whose project folder is this one: the repository's top, a worktree under `.claude/worktrees/` counting as its project, else the folder itself) · **this computer** (every project; for runs and tasks: every project, on the computer they run on) |
| **Patterns** (optional) | commands: their first words at a word boundary (`cargo test` covers `cargo test --lib`, never `cargo test-foo`; a trailing `*` means the same) · files: globs relative to the session's folder (`src/**`, `*.md`; never a path outside it) · sites: a host (`docs.rs`) or its subdomains (`*.github.com`) · tasks take none |

No patterns means *any* of that kind there. A command pattern never covers a
command holding a shell metacharacter (`;` `&` `|` `` ` `` `$` `(` `)` `{`
`}` `<` `>`, a line break), so a prefix cannot smuggle a second command in;
a pattern may not start with a shell or a program that runs what it is
given (`bash`, `sh`, `pwsh`, `cmd`, `env`, `sudo`, `xargs`, …), nor name an
interpreter alone (`python`, `node`, `npx` — name what it runs:
`python -m pytest`).

## What always asks

- **Anything dangerous** (`pending::classify_dangerous`: writes to `.claude/`,
  an account's config directory, `.mcp.json`, shell startup files, git hooks,
  launch agents, Mother Claude's data, credentials…; a run another computer
  calls dangerous; a task that would run in `bypassPermissions` or
  `dontAsk`). Its card says it can't be remembered.
- Questions, plans and elicitation forms: a permission only answers a
  permission request.
- Browser actions keep their own standing grants (docs/BROWSER.md).
- Everything, while the permissions are **paused** (the page's switch; the
  rules are kept).

## Who may save one

The card's **Allow for…** offers what fits the request
(`PendingItem.grant`); answering with `remember: {scope, patterns}` saves it
(`POST /api/sessions/{id}/respond`). **This session** is open to anyone who
may answer the card. **This project** and **this computer** outlive the
session — a standing grant — so they need the dangerous right: this
computer, or a device allowed dangerous actions (`403 dangerous_blocked`
otherwise). A pattern that does not fit is `422 invalid_pattern`.

On the page, pausing and removing only take away, so any paired device that
may manage data does them (never the local-script token); turning the
permissions back on, and a change that lets one allow more (a wider place,
patterns added or removed), need the dangerous right too.

## How it applies

`pending::raise` — the one door every channel's request comes through
(owned sessions' `can_use_tool`, the hooks plugin's held PermissionRequest,
the HTTP API's tool approvals, the sidecar) — asks the rules before it
shows a card. A covered request is allowed at once, as if the user had
answered: no card, no push, and its `pendingResolved` frame and timeline
row say *Allowed by your permission · commands like cargo test in shop*
(`ResolverKind::Permission`). The narrowest rule that covers it is credited
and counted (its last use and how often). A headless `claude -p` run's
request, which is not held when no dashboard is connected, is still
answered when a permission covers it. Runs and tasks ask the rules before
they raise their card, and a run records the permission it ran under.

## Where they live

`<app data dir>/permissions.json` (0600, written whole and atomically,
read entry by entry: one this build cannot read is skipped; a rule for one
session that nothing used for 30 days is dropped). **Never Claude Code's
settings**: a permission is Mother Claude's own and changes nothing under
`~/.claude` (Claude Code's own *always allow* suggestions, written to its
settings by Claude Code itself, remain on the card for owned and hooked
sessions). The standing grants for runs of v0.11–v0.13
(`machines/run-grants.json`) were taken over once, as permissions for their
project. Each change is audited (`permission.add`, `.update`, `.remove`,
`.pause`, `.resume`) with the rule's kind, scope and project, and the
SHA-256 of each pattern — never a pattern's text — and told to every client
as `{t:"permissions"}`.

## Routes

| Route | Who | Does |
|---|---|---|
| `GET /api/permissions` | Authed | `{rules: […], paused}` |
| `PATCH /api/permissions` `{paused}` | ManageData (resuming: the dangerous right) | pauses or resumes them |
| `PATCH /api/permissions/{id}` `{patterns?, scope?}` | ManageData (allowing more: the dangerous right) | the rule as it is now |
| `DELETE /api/permissions/{id}` | ManageData | removes one; answers the list |

A rule: `{id, kind, tool?, patterns, patternKind?, scope, session?,
sessionName?, project?, projectName?, machine?, machineName?, createdAt,
createdBy, usedAt?, uses}` (times in epoch ms).

Each computer keeps its own: a session's requests are answered where the
session runs, so a card answered from another computer through Machines
saves the permission on the computer that runs the session, with the hop
rule's right (docs/PLAN-MACHINES.md, `X-MC-Acting`).
