# Mother Claude — AI coding guidance

> This file is auto-loaded by Claude Code. Read it before making changes.

## What this is

**Mother Claude** is a Tauri v2 (Rust) + Angular desktop app that monitors and
controls every local Claude Code session/agent/service from one dashboard, and
serves that *same* dashboard over the LAN (default `0.0.0.0:6725`, TLS on
private addresses) and, when the user turns it on, to a tunnel through a
loopback-only port (default `6726`), so it can be driven from a phone. Four
layers:

1. **Rust core** (`src-tauri/src/claude/`) — an adapter over `~/.claude` plus
   session control.
2. **Embedded axum server** (`src-tauri/src/server/`) — REST + WebSocket on the
   Tokio runtime. **All** dashboard data flows through here.
3. **Angular SPA** (`src/`) — desktop webview *and* phone browser, one code path.
4. **Claude HTTP bridge** (`src-tauri/src/bridge/` + `sidecar/src/bridge-host.ts`)
   — a programmatic HTTP API over Claude on this machine (conversations, turns,
   SSE, tool approvals, images) plus a direct Messages API path. Mounted at `/v1`
   on the main server and on its own loopback port (default `5612`).
   **It does not listen until started** from the app's API screen
   (`bridge::control`, `/api/bridge/*`); until then the routes that drive Claude
   answer `503 bridge_not_started` while discovery stays open. Start-time
   choices are *defaults* — every request may override them — and one of them is
   which conversation an unaddressed `/chat` message joins.
   See [docs/BRIDGE.md](docs/BRIDGE.md).

## The one hard invariant: owned vs foreign sessions

Full two-way control (inject answers / approve permission prompts live) is only
reliable for sessions **Mother Claude launched itself** ("owned"). There is no
supported way to inject input into a *foreign* live session (started in another
terminal/editor and held by the Claude Code supervisor — its control socket uses
a rotating key and an undocumented protocol).

- **Owned sessions** → full control via the installed `claude -p` speaking the
  stream-json control protocol (Path B, the default; `claude/control.rs`):
  launched like VS Code does (`--permission-prompt-tool stdio`, always an
  explicit `--permission-mode`), `initialize` first, every CLI → host control
  request answered (`can_use_tool` becomes a pending item; unknown subtypes get
  `error: unsupported`), and interrupt / model / mode / effort / rename /
  stop-task / cancel-queued-message sent as control requests (CLI errors are a
  422). Children get a scrubbed environment (`control::SCRUBBED_ENV`: the
  parent session's `CLAUDECODE`/`CLAUDE_CODE_*` identity and our token — not
  provider or auth variables) plus `MOTHER_CLAUDE_OWNED=1`. Continuing a
  session another process holds **forks** it (`--resume --fork-session
  --session-id <new>`); never two writers on one transcript. The Agent SDK
  sidecar (Path A) stays behind `MOTHER_CLAUDE_SIDECAR=1`. Background jobs are
  addressed by their SHORT job id only.
- **Foreign sessions** → monitoring (transcripts, pidfiles, hook events),
  lifecycle (`stop`/`respawn`/`rm`, short job id) and, through documented
  channels only:
  - the **Mother Claude plugin** (user scope, installed from the desktop via
    `claude plugin marketplace add` + `claude plugin install`; never by
    editing `~/.claude/settings.json` hooks). Its `mc-hook`/`mc-wake` scripts
    (`src-tauri/assets/claude-plugin/`, exec-form hooks, always exit 0) feed
    `server/hooks.rs`: a held `PermissionRequest` races the session's own
    dialog (first answer wins), relays go out as PostToolUse/Stop
    `additionalContext` or wake an idle session (`asyncRewake`), interrupts
    land at the next tool boundary. Owned ids are ignored there;
  - **tmux** (`claude/tmux.rs`) for sessions running in a pane: argv-only
    `send-keys`, never while a dialog is open.
  What each session supports is `SessionSummary.control` (`claude/caps.rs`);
  the UI must stay honest about it. PTY-driving `claude attach` remains an
  experimental extra for **background (`--bg`) jobs only**, addressed by their
  short job id and gated as dangerous (`experimental` feature, default on;
  runtime opt-out `MOTHER_CLAUDE_FOREIGN_INJECTION=0`). It cannot reach VS
  Code, JetBrains, Desktop or terminal sessions: they are not supervisor-hosted
  and have no short id. The "CCR v1" transport is a status probe only. Never
  speak the cc-daemon control socket or `/tmp/cc-socks/*.sock` directly.
  Owned sessions remain the path to *reliable* control (model, effort, slash
  commands, images).
- **Limits of the hooks channel** (verified on CLI 2.1.282): no model, effort or
  slash-command changes (tmux or an owned fork only); the permission mode
  changes only together with an approval (`updatedPermissions` `setMode`);
  interrupts land at the next tool boundary (`continue:false`), never
  mid-generation; relayed messages reach the model as `additionalContext` or
  the `mc-wake` stderr and never show in the session's own UI; headless
  `claude -p`/SDK runs are never woken, and their `PermissionRequest` is held
  only briefly (`MOTHER_CLAUDE_HEADLESS_HOLD_SECS`, default 120) and only while
  a dashboard is connected; a `bypassPermissions` session raises almost no
  `PermissionRequest`s; sessions started before the install need
  `/reload-plugins`.

**Design the product around owning sessions.** Do not block on foreign injection.

## Data sources (adapter contains all version drift)

Everything lives under `CLAUDE_CONFIG_DIR` or `~/.claude`. Keep all reads in
`src-tauri/src/claude/` so schema churn is contained to one place. Make every
deserialized field tolerant (`#[serde(default)]`, capture unknowns) — these are
undocumented research-preview internals.

| Source | Path / command | Gives |
|---|---|---|
| **Live roster** | `~/.claude/sessions/<pid>.json` (**never** the `*.key` files beside them) | `pid, sessionId, cwd, kind, entrypoint, status (busy/idle/waiting), waitingFor, tmux, jobId, procStart, …` — `claude/live.rs` |
| Background jobs | `~/.claude/jobs/<short>/state.json` (keyed by the **short** job id, not the session uuid) | short id (`daemonShort`), `sessionId`, state (`working/blocked/done/failed/stopped`), detail — also for stopped jobs with no process |
| Background lifecycle | `claude stop\|respawn\|rm <short>`, `claude logs <short>` | lifecycle; the terminal view (rendered with `vt100`) |
| Daemon health | `claude daemon status` | pid, version, uptime (`/api/daemon`, `/api/services`) |
| **Transcripts (all surfaces)** | `~/.claude/projects/<encoded-cwd>/<id>.jsonl` | append-only NDJSON, tailable — indexed incrementally by `claude/index.rs` (checkpoint `index-cache.json` in the app data dir); found by session id, since the cwd can drift |
| Subagents | `projects/<enc>/<id>/subagents/agent-<aid>.{jsonl,meta.json}` | per-agent transcripts (written per block, so live), type, parent tool use |
| Workflows | `projects/<enc>/<id>/subagents/workflows/<run>/journal.jsonl`, `workflows/<run>.json` | running (`started` − `result`) / finished workflow agents |
| Background task output | `<tmp>/claude-<uid>/<enc>/<id>/tasks/<task>.output` | shell output tail (read on demand, 16 KB) |
| MCP inventory | `~/.claude.json` `mcpServers` | configured MCP servers (`env`/`headers` values redacted before serving) |
| Trusted folders | `~/.claude.json` `projects[<dir>].hasTrustDialogAccepted` | where non-desktop callers may spawn/continue |
| Claude sign-in | `claude auth status --json` | account, checked at startup (`claude/auth.rs`) |
| Worktrees | `<project>/.claude/worktrees/<name>` | per-session isolated edits |
| tmux panes | pidfile `tmux` (`session:@win.%pane`) + `tmux list-panes -a` on the default socket, then `${TMUX_TMPDIR:-/tmp}/tmux-<uid>/*` | the pane to type into / capture (`claude/tmux.rs`) |
| Live events | **Hooks plugin** (`mc-hook` → `POST /hooks/event`, `mc-wake` → `POST /hooks/wait?pid=`), installed via `claude plugin …`, status from `claude plugin list --json` | tool, prompt, permission, subagent, compaction, notification, stop and session events; held approvals; relays and wake-ups (`server/hooks.rs`, `claude/hooks.rs`) |
| Session settings | transcript `perTurnEffort ?? effort` and `message.usage.speed`; `ultra_effort_enter`/`ultra_effort_exit` attachments; hook `effort.level`; launch argv (`--effort`, `--max-thinking-tokens`, `--thinking`, inline `--settings`); settings `alwaysThinkingEnabled`; owned sessions: `get_settings.applied` | effort, ultracode, thinking and fast mode, each with its source (`claude/summaries.rs` `derive_settings`, `claude/live.rs` `LaunchFlags`) |
| Plan usage | owned streams' `rate_limit_event` (`unifiedWindows`: 0–1 fraction, epoch seconds); `get_usage {skip_behaviors:true}` through an owned session or a short-lived helper `claude -p` in `<app data dir>/usage-helper`; fallback `~/.claude.json` `cachedUsageUtilization` (≤ 1 h, same account, the account id compared in-process only) | 5-hour and weekly windows, reset times, extra usage (`claude/usage.rs`, `/api/usage*`, `planUsage` frame) |

Key facts: `<encoded-cwd>` = absolute path with every non-alphanumeric char → `-`.
The *same* `projects` store is written by CLI, VS Code/JetBrains, and Claude
Desktop — reading it captures all surfaces for free. Transcript `tool_use`/
`tool_result` are content blocks inside `message.content`, **not** top-level event
types. Token usage is in `message.usage` (de-duplicate by `message.id`, keep the
last record). Tail with a byte offset, split on `\n`, buffer the trailing partial
line; lines can be tens of MB, so never read a whole file. A main transcript
flushes each API message only when it completes (no live token stream for
foreign sessions); subagent files are written per block. Transcripts are pruned
after 30 days. `~/.claude/daemon/roster.json`, `history.jsonl` and `daemon.log`
exist but nothing reads them (`home.rs` only knows their paths).

The roster no longer spawns `claude agents --json` (its shape drifts and it is
slow): pidfiles give status and liveness is `kill(pid, 0)` plus the process
start time (`ps -o lstart=`, darwin). Status and activity are derived by
`claude/activity.rs` from the pidfile, the owned control stream, hook events
and the transcript tail; summaries are built by `claude/summaries.rs` and
published as WebSocket diffs by `server/monitor.rs`. Every id from a request is
validated with `claude::ids` and files are found through the index, never built
from request input.

Ultracode of a session we don't own is in its transcript and, at most, its
launch argv (`--effort ultracode`, an inline `--settings '{"ultracode":…}'`);
hooks and pidfiles never carry it. On each regular human prompt Claude Code
writes `ultra_effort_enter` (`full` when it turned on or the last marker is
gone, e.g. after an auto-compaction; `sparse` about every 10 messages while it
stays on) or `ultra_effort_exit` when it turned off, and nothing otherwise. So a
regular prompt *without* a marker confirms the last state, and the value is as
of the last message (`claude/index/agg.rs` `UltraReplay`; don't reduce it to
"last marker wins" or "no marker means unknown"). Task notifications, relays,
`/loop` and cron fires never get a marker, so they confirm nothing. A restart
turns ultracode off (VS Code doesn't switch it back on), so for a live process
started after the last message it is unknown (`restarted`, the earlier value
only a hint) unless its argv says. Thinking is never reported back either:
owned sessions show what we set, others their launch flags or
`alwaysThinkingEnabled`.

Claude's own Anthropic sign-in is read via `claude auth status --json` in
`claude/auth.rs` and checked once at startup (`state.refresh_claude_auth`), so a
signed-out account is one line in the banner rather than every turn returning the
string "Not logged in · Please run /login".

**Never export `CLAUDE_CONFIG_DIR` onto a child process you spawn.** Setting it
breaks the CLI's credential resolution *even when it points at the default
`~/.claude`*: every turn then answers "Not logged in · Please run /login".
Children inherit it when the user set one themselves, which is the only case
where an override is wanted. (Verified against SDK 0.3.186 / CLI 2.1.186.)

## Architecture rules (enforced)

- **All data for both desktop and mobile flows through the axum server, never
  Tauri `invoke`.** Tauri `invoke` is allowed only for desktop-only OS concerns
  (e.g. opening System Settings for Full Disk Access). Dashboard data: HTTP/WS.
- **Who is asking is decided once per request.** Every listener's router is
  built with its `Ingress` (`server::context`); a middleware builds a
  `ClientContext {ingress, effective_ingress, peer, client_ip, local,
  on_machine, principal}`. `on_machine` (machine-local) = loopback listener +
  loopback peer + exactly one loopback `Host` + no proxy/tunnel header; `local`
  adds a local-or-absent `Origin`. Tunnels connect from 127.0.0.1, so never use
  the raw TCP peer. **Authorize by principal, not by location**: `local` only
  decides whether the desktop / local-script tokens and `?token=` are accepted;
  `ctx.may_dangerous()` is true for `Principal::Desktop`, a device holding the
  grant for its effective ingress (`allowDangerous` on the LAN,
  `allowDangerousInternet` elsewhere), or an anonymous *local* caller on the
  bridge port. Use the guard extractors in `server::auth` (`Authed`,
  `DesktopOnly` for admin routes, `DangerousOk`, `HookOnly`, `SidecarOnly`), and
  call `ctx.note_dangerous(..)` when a device passes a dangerous check you made
  by hand (audit + desktop notice).
- **The desktop token is per launch and in memory** (`server_info` invoke only).
  The on-disk `script-token` file is `Principal::LocalScript`: local only,
  data and control routes, never admin routes; dangerous actions only on the
  bridge's loopback port, like a tokenless local caller there. The legacy
  `token` file is deleted at startup and its value never accepted. Phones pair for
  `mcd_` device tokens (`server::pairing`, `server::devices`); internet
  pairings need the desktop to type the phone's 4 digits. Internet access is a
  separate loopback listener (`server::remote`, default 6726) behind a tunnel;
  LAN TLS binds private addresses only, with a name-constrained local CA
  (`server::tls`, `server::listeners`).
- **The bridge is a second consumer of the same server, not a second server.**
  `bridge::router` nests into the same axum app and reuses the same
  `ClientContext`, the dangerous-action policy and the pending store
  (`crate::pending`) — a bridge tool approval shows up as a dashboard card and
  either surface may answer it. Its extra listener binds **loopback only** and
  is **tokenless and origin-open by default** for machine-local requests
  (`ctx.on_machine`), so local tools and other websites on this machine can call
  it without setup; a tunnelled request needs a token, the tokenless mode turns
  off while a detected tunnel points at the port, and the LAN-reachable `/v1`
  mount always needs a token. Dangerous actions (`bypassPermissions`, transcript
  deletion, starting a sign-in flow, approving a dangerous tool call or attaching
  a standing permission grant to an approval, thread options that run code
  nobody approves, turns into a conversation whose approvals are off) need the
  dangerous right — for an anonymous bridge caller that means `local` (no
  foreign `Origin`).
  Whether a tool call is dangerous is decided server-side for every channel
  (`pending::classify_dangerous`: writes to `.claude/`, `.mcp.json`, shell
  startup files, git hooks, launch agents, our data dir, …); a channel's own
  flag can only raise it.
- Single broadcast bus (`tokio::sync::broadcast`) fans events to webview + all
  WS clients. Backpressure-aware.
- TLS + token auth are **mandatory** whenever the bind address is non-loopback.
  Dangerous actions are desktop-only by default; a device gets them only through
  a per-device grant set from the desktop (LAN and internet separately).
- **Never write under `~/.claude` from our code.** Changes there happen only
  through Claude Code itself on an explicit user action: `claude plugin
  install/uninstall` from the desktop, a standing grant the user chose, a
  transcript purge through the SDK. Tests and scripts must never run the plugin
  installer against the real config. One exception runs without a user action:
  the plan-usage helper (`claude/usage.rs`), a short-lived `claude -p` that gets
  `initialize` and `get_usage` and never a message. It runs while a dashboard is
  connected (or for a Refresh) when no owned session can answer, and while it
  runs Claude Code writes its pidfile under `~/.claude/sessions` (removed on
  exit) and refreshes its own caches in `~/.claude.json`. Our code still writes
  nothing there.

## Build / lint / test commands

```bash
npm install                 # Angular deps (root)
npm run lint                # Angular ESLint
npm run build               # Angular production build -> dist/
npm run tauri:dev           # run desktop app (dev)
npm run tauri:build         # package the app

cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test  --manifest-path src-tauri/Cargo.toml

npm run sidecar:build      # builds BOTH agent-bridge.js and bridge-host.js
npm run test:release       # release tooling: signatures, manifest merge, versions

# Releases are built locally on the Mac and the Windows desktop, never in CI:
# docs/RELEASING.md (scripts/release/release-{macos.sh,windows.ps1}).

# The bridge's live tests spend tokens and need `claude` signed in; they are
# #[ignore]d so `cargo test` never touches the network or starts the Node host.
cargo test --manifest-path src-tauri/Cargo.toml --test bridge -- --ignored
```

`just` targets mirror these (`just lint`, `just test`, `just build`, `just dev`).

## Coding conventions

- **Rust**: edition 2021, `rustfmt` clean, `clippy -D warnings` clean. `anyhow` at
  boundaries, `thiserror` for typed errors. Never `unwrap()` on I/O. Tolerant
  parsing for every `~/.claude` read — log and skip, never crash. `tracing` for
  logs (never write logs into `~/.claude`).
- **Angular**: latest, standalone components, signals for state, strict
  TypeScript, ESLint + Prettier clean. One API client used by desktop and mobile.
- **Commits**: Conventional Commits, small and well-scoped. Keep every commit
  green. Do not squash.
- **Secrets**: never commit tokens, passwords, or generated TLS certs. They are
  generated at runtime into the app config dir and `.gitignore`d.
