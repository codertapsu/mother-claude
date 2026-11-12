# Mother Claude's MCP server

Every Claude Code session on this computer can call Mother Claude's own tools
(the browser, the task board, teams, your other computers, and Mother Claude
itself) through one MCP server inside the desktop app. This document is the
reference for that server: how sessions reach it, how it knows who is
calling, what it refuses, and what was verified on the real CLI (Spike B,
plan §14 Phase 0).

Code: `src-tauri/src/mcp/` — `mod.rs` (the tool contract, `mc_ping`),
`server.rs` (`/mcp`, `state.mcp`), `jsonrpc.rs` (framing, versions),
`identity.rs` (the calling session), `plugin.rs` (the plugin entry, the
helper, the token file), `api.rs` (the dashboard's panel), `session_tool.rs`
(`mc_session`). Tests: `src-tauri/tests/mcp.rs`.

## What it is

* **MCP Streamable HTTP** at `POST http://127.0.0.1:<port>/mcp` (the dashboard
  port, 6725 by default), on the **loopback listener only**: the LAN and
  tunnel listeners answer `404`, the bridge ports do not have the route.
* **JSON-RPC 2.0**, one message or a batch per POST, answered as
  `application/json`; no SSE (`GET /mcp` is `405`). Notifications and client
  responses get `202` with an empty body. `DELETE /mcp` ends an MCP session
  (`204`).
* Methods: `initialize`, `ping`, `tools/list` (one page; any cursor is
  -32602), `tools/call`; `notifications/cancelled` stops an in-flight call of
  the same client. Anything else is -32601 (Claude Code's `server/discover`
  probe among them, which is how it falls back to `initialize`). An unknown
  tool or malformed params is -32602, a malformed message -32600, unreadable
  JSON -32700 (HTTP 400), a body over 16 MiB HTTP 413.
* **Protocol versions** `2025-11-25` (what Claude Code 2.1.319 asks for),
  `2025-06-18`, `2025-03-26`, `2024-11-05`: the client's when supported, else
  the newest. `initialize` returns `capabilities: {tools: {listChanged:
  false}}` and `serverInfo` `mother-claude`. Answers are shaped for the
  version in `MCP-Protocol-Version` (else the MCP session's, else
  `2025-03-26` as the transport says): no `title` or `structuredContent`
  before 2025-06-18, no `annotations` before 2025-03-26. An unknown version
  in the header is never refused (the CLI's discovery probe sends a 2026 one).
* `initialize` issues an `Mcp-Session-Id`; later requests may send it or not,
  and an unknown one is accepted (an app restart forgets them all, and
  sessions keep working).
* **A tool's failure is a result** (`isError: true`, text `"<message>
  (<code>)"`), never a protocol error. Each call runs under a 120 s ceiling
  (`timeout`) unless its tool names a longer one (`Tool::limit`:
  `machine_run` 60 minutes, `machine_wait` 11, `machine_output` 20), a
  cancellation (`cancelled`) and a panic guard (`internal`, debug builds;
  release builds abort on a panic, so a tool must never panic).
* **Off switch**: Services › Claude Code tools (desktop only, persisted in
  `<app data>/mcp-settings.json`). Off: every `/mcp` request gets `503
  mcp_disabled`, the plugin entries and the token file are removed, owned
  sessions get no plugin. On by default.

## How each kind of session reaches it

The tools appear to the model as **`mcp__plugin_mother-claude_mc__<tool>`**
in every session (plugin `mother-claude`, server `mc`). MCP tools are
deferred: the model loads one with ToolSearch first, and in the `default`
permission mode Claude Code asks before calling it (an owned session's ask is
a dashboard card; a foreign session's is its own dialog).

| Session | Path |
|---|---|
| Terminal, VS Code, JetBrains, Desktop (foreign) | The installed Mother Claude plugin (Remote › Foreign-session control). The CLI reads the plugin from our data dir in place, so the `.mcp.json` written there reaches every session started afterwards; running sessions need `/reload-plugins` or a restart. |
| Sessions Mother Claude launches (owned, Path B) | Their own copy of the plugin, loaded inline: `claude -p … --plugin-dir <app data>/mcp-plugin/mother-claude` (`claude::control::headless_command`). Works for every account kind, installed plugin or not. |
| Path A sidecar sessions, HTTP bridge conversations | Not yet (the Agent SDK takes `plugins: [{type: "local", path}]`; to do). |

**Why owned sessions use the plugin and not `--mcp-config`**: one name
everywhere. `--mcp-config` would name the tools `mcp__mc__<tool>` there, and
wherever the plugin is installed too the session would get both servers.
Verified on 2.1.319: an inline plugin (`--plugin-dir`) **shadows** an
installed plugin of the same name — the session's `system/init` lists one
`mother-claude`, `source: mother-claude@inline` — so there is exactly one `mc`
server and one copy of the hooks (whose scripts exit at once in owned
sessions, `MOTHER_CLAUDE_OWNED=1`). The owned copy has its own folder because
a folder at `<app data>/claude-plugin/` is foreign control's sign that the
plugin was installed (`claude::hooks::on_start` then asks the CLI about it,
and `claude plugin list` writes the CLI's own `.claude.json`); it is created
only by installing foreign control.

### Files

```text
<app data>/claude-plugin/mother-claude/.mcp.json          the installed plugin's entry
<app data>/claude-plugin/mother-claude/scripts/mc-mcp-headers   (Windows: mc-mcp-headers.ps1)
<app data>/mcp-plugin/mother-claude/…                     owned sessions' copy (whole plugin; no hooks on Windows)
<app data>/mcp.token                                      0600, `mcm_…`, new every launch
<app data>/mcp-settings.json                              0600, {"enabled": bool}
```

Written at every start and when the setting changes (`mcp::plugin::apply`),
and right after foreign control installs the plugin; the installed plugin's
entry only in a folder foreign control created. Never with `claude plugin
configure`, never under `~/.claude` or any account's config directory. The
plugin's version is the app's plus `+mcp.1` (`claude::hooks::PLUGIN_VERSION`),
so the desktop can tell when sessions load an older layout.

The entry:

```json
{ "mcpServers": { "mc": {
    "type": "http",
    "url": "http://127.0.0.1:6725/mcp",
    "headersHelper": "\"${CLAUDE_PLUGIN_ROOT}/scripts/mc-mcp-headers\""
} } }
```

The quotes matter: Claude Code runs `headersHelper` through a shell after
substituting the plugin root, and the data dir has a space in it on macOS
(`Application Support`). The port is the one of this start; a session keeps
the URL it started with.

`mc-mcp-headers` (POSIX `sh`, like `mc-hook`, the data dir baked in) prints
one JSON object and always exits 0:

* `Authorization: Bearer mcm_…` from `mcp.token` (validated: `mcm_` +
  base64url; nothing else is ever sent);
* `X-MC-Pid: $PPID` — the CLI itself (it starts the helper directly);
* `X-MC-Session: $MOTHER_CLAUDE_SESSION` only when `MOTHER_CLAUDE_OWNED=1`
  and the value is a canonical session id. Mother Claude sets both on the
  Path B children it launches. Unlike the hook scripts, the helper does not
  exit early for owned sessions.

### On Windows

Claude Code runs `headersHelper` through `cmd.exe` there (verified on
2.1.317), which cannot run an sh script: the CLI then connected with no
token, got a 401, fell back to OAuth discovery and reported "HTTP 503 trying
to load OAuth metadata from …/.well-known/oauth-authorization-server". So on
Windows the helper is its PowerShell twin, `scripts\mc-mcp-headers.ps1`
(UTF-8 with a BOM, which Windows PowerShell 5.1 needs to read a data dir
with a non-ASCII letter), and the entry starts Windows PowerShell on it, both
by full paths:

```json
"headersHelper": "\"C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe\" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File \"C:\\…\\scripts\\mc-mcp-headers.ps1\""
```

It prints the same headers. As `cmd.exe` stays between the CLI and the
helper, `X-MC-Pid` is the first ancestor that is not a shell (`cmd.exe`,
`powershell.exe`, `pwsh.exe`), looked up through WMI; a Git Bash `$PPID`
would be 1 there. The hooks get PowerShell twins too
(`mc-hook.ps1`, `mc-wake.ps1`, docs in `claude::hooks`), and owned sessions'
copy carries no hooks on Windows: each would cost a PowerShell start (about
0.15 s) only to exit at once.

## Identity

`identity::resolve` fills the `Caller` every tool gets (`ToolCtx.caller`):

1. `X-MC-Pid` must name a running process (`kill(pid, 0)`) **that holds the
   connection** the request came on: the process or one of its descendants
   owns the socket whose local port is the request's peer port (`ps` +
   `lsof`, the hooks' binding check, `verify_process`; cached per
   connection for a minute, about 30 ms when it runs). The token file is
   readable by any process of the user — a session's own Bash included — so
   without this a `curl` in one session could name another session's pid;
   with it, it can only be itself. A mismatch makes the caller nobody (no
   pid, no session); a check that cannot run (no `lsof`, Windows) changes
   nothing. Verified on the real CLI: its MCP client holds the connection
   itself.
2. The pid's session and cwd come from the live roster's record for that
   pid, else `sessions/<pid>.json` read right then from each account's
   config directory (`claude::live::read_pidfile_of`; only `.json`, never a
   key file).
3. `X-MC-Session` counts only for a session Mother Claude owns
   (`claude::owned::is_owned`), and only when nothing contradicts it: the
   pid's own pidfile wins (a `claude` started inside an owned session
   inherits its variables), and an owned session whose process the roster
   knows must be that pid. (Before the roster has seen an owned session's
   pidfile — its first seconds — the claim rests on the pid's binding
   alone.)
4. Otherwise the caller has no session (`session_id: None`); tools that need
   one say so (`mc_session` answers `unknown_session`).

`session_id` is always a canonical session id (`claude::ids::is_session_id`)
or `None`, and `cwd` the session's working directory. `Caller.owned` is
whether the session is Mother Claude's. A tool may key per-session state on
the caller (a browser lease, the session an approval is raised for), but the
decision on anything dangerous stays an approval through `crate::pending`,
which the `Mcp` principal can never answer.

## Security

* **Principal `Mcp`**: the `mcm_…` token, a fresh random value every launch,
  written only to `mcp.token` (0600) and never to a log, a frame, an error,
  an audit line or an API answer. Accepted only on the loopback listener from
  a machine-local client (`ctx.local`), and only `/mcp` admits it: every
  dashboard guard (`Authed`, `DesktopOnly`, `ManageData`, `HookOnly`,
  `SidecarOnly`, the `/api` principal layer), the WebSocket first frame and
  the bridge mounts refuse it. `may_dangerous()` is false for it.
* **Order of refusals** on `/mcp`: not the loopback listener, or a tunnelled
  or proxied request, or a non-loopback `Host` → 404; an `Origin` other than
  `http(s)://localhost|127.x|[::1]` (Tauri's and extensions' included) → 403
  `forbidden_origin`; the setting off → 503; no or a wrong credential → 401
  (no `WWW-Authenticate`), another principal's → 403 `wrong_credential`.
* **No dangerous rights of its own**: a tool's dangerous effect is an
  approval through `crate::pending` (desktop, or a device with the grant),
  never this principal's right; the principal reaches no `/api` route, so it
  can never answer a pending item either.
* **Identity is bound to the connection** (see Identity): a session cannot
  pass itself off as another by naming its pid.
* **Audit**: a call to a tool not annotated `readOnlyHint: true` is an audit
  line (`mcp.call`: tool, session, owned, ok, duration — never arguments).
  The last 50 calls of every kind are kept in memory for the panel.
* **Token rotation**: a new token every launch. Claude Code caches the
  helper's headers per connection; after an app restart its first call gets
  401, it re-runs the helper (reading the new token), reconnects and retries
  once — the model never sees it (verified).

## The tools

Each area registers its own tools and documents them:

| Tools | Area | Reference |
|---|---|---|
| `mc_ping`, `mc_session` | Mother Claude itself | `mcp/mod.rs`, `mcp/session_tool.rs` |
| `browser_*` | the user's browser, through the extension | [BROWSER.md](BROWSER.md), [BROWSER-PROTOCOL.md](BROWSER-PROTOCOL.md) |
| `task_*`, `team_*`, `session_summary` | a team's board | [TEAMS.md](TEAMS.md) "Tools" |
| `machine_*` | the user's other computers | below |

### Machines

`src-tauri/src/machines/work/mcp.rs` (docs/PLAN-MACHINES.md Phase 4; tests
in `tests/work.rs`; the user guide is [MACHINES.md](MACHINES.md) "Running
commands on another computer"). A session runs a command on another of the
user's paired computers at its own code: Mother Claude snapshots the
session's project, sends what the other computer lacks, and that computer
runs the command in a kept work folder of its own (a worktree of its
checkout; its checkout is never touched).

| Tool | Input | Answer |
|---|---|---|
| `machine_list` | `{}` | JSON, also as `structuredContent`: `{machinesOn, thisComputer: {name, os, shell}, machines: [{id, name, os, osVersion, shell, shellLabel, online, path, theirGrants: {run: {lan, internet}} \| null, mayRun, why, versions}]}`; `path` is `lan`, `internet`, `relay` or null, `why` says why `mayRun` is false |
| `machine_run` | `{machine, command, cwd?, timeout?, artifacts?}` | text: where it ran (computer, shell, work folder), how it ended (exit code and duration; cancelled; out of time; could not run, and why), the output's last 64 KiB, the files `artifacts` matched with their sizes — or, after 10 minutes, *still running* with the run's id and its latest output |
| `machine_wait` | `{id, timeout?}` | the same, after waiting up to `timeout` seconds (600 by default and at most) |
| `machine_output` | `{id, from?, max?}` | a page of the whole output, kept a day on the computer that ran it: bytes `from`–`to` of `size`, and the call for the next page while there is more |
| `machine_output` | `{id, artifact}` | one file the run listed (64 MiB at most) brought into the project at `.mother-claude/artifacts/<run>/<path>`, and where it is |
| `machine_cancel` | `{id}` | stops the run there with its whole process tree; answers how it stands |

The arguments:

* `machine` — a paired computer's name (any case) or id.
* `command` — one command line, at most 8000 bytes, for that computer's
  shell (`machine_list`'s `shell`: `zsh`, `pwsh`, `powershell`, `bash`,
  `sh`).
* `cwd` — a folder inside the project, relative to its top (`/` or `\`;
  never absolute, `..` or `.git`); by default the folder the session works
  in.
* `timeout` — seconds, 1–7200; 1800 by default. It counts from when the run
  reaches that computer, a wait for its work folder included.
* `artifacts` — a glob relative to the project's top (`target/release/*.exe`;
  one without a `/` matches at any depth, `**` any number of folders); up to
  100 files are listed, once the command ended by itself.
* `from` — a byte offset, negative from the end; `max` — 1024–262144 bytes,
  65536 by default.

How a call goes:

* **The caller** must be a session the server can name (see Identity),
  working in a git repository: that repository's top is the project, the
  session's folder in it the default `cwd` (otherwise `unknown_session`,
  `not_a_repository`). A run belongs to the session that asked; another
  session reaches it only from the same project (`denied`).
* **Before anything is asked**: Machines on here (`machines_off`), the
  computer paired (`unknown_machine`), and — else `not_allowed`, saying
  why — online, on a build that runs commands ("Update Mother Claude on …")
  and holding this one's `run` grant on the path in use (saying where to
  turn it on). That computer then says where the project is there; without
  a checkout it refuses (`work_no_checkout`), and an older build that does
  not know the route gives `update_needed`.
* **The approval**: a `crate::pending` item (`source: "machine"`,
  `toolName: "machine_run"`, its `input` the computer, checkout, folder,
  command, shell, time limit and glob), dangerous when
  `pending::sensitive_command` flags the command, with the suggestion
  *Always allow `<prefix>` on … for this project* (`kind:
  "machineRunGrant"`, `standing: true`) when a grant could cover the
  command — unless a standing grant covers it already. Denied: `denied`
  (the model is told not to retry, with the user's note); unanswered for
  10 minutes: `approval_timeout`. A dangerous command also needs the other
  computer's `dangerous` grant (`dangerous_needed`). That computer
  classifies the command itself too, before the approval (its verdict comes
  back with *where it would run*): a flag on either side makes the approval
  dangerous, and that computer checks again when the run starts.
* **Long calls.** A `machine_run` call may take an approval (up to 10
  minutes), the code's transfer and the run (waited for up to 10 minutes),
  so the server lets it run up to 60 minutes; `machine_wait` up to 11 and
  `machine_output` up to 20 (a large file). `machine_list` and
  `machine_cancel` keep the 120 s ceiling.
* **Answers are plain text**, but for `machine_list`: the model reads a
  run's result as it would a terminal's. A failure is an `isError` result
  with its code (above, or the other computer's own: `grant_missing`,
  `machines_off`, `run_failed`, …).
* `machine_list` and `machine_wait` are read-only, so they write no
  `mcp.call` line; every run is audited by Machines on both computers
  (MACHINES.md, SECURITY.md "Running commands").

## The panel (Services › Claude Code tools)

`GET /api/mcp` (desktop, paired devices, local scripts): on/off, the tool
count and names, sessions heard from in the last 15 minutes, whether owned
sessions get the tools, the last calls (time, session, tool, ok/error — no
arguments); for the desktop also the installed plugin (installed, the
version sessions load, the version this app writes, `outdated`), unless the
request says `?plugin=false`. The panel polls every 5 s with it: the plugin's
state may run `claude plugin list` (cached 30 s), which writes the CLI's own
`.claude.json`, so only a page load, the toggle and the foreign-control page
read it. `PUT /api/mcp/settings {"enabled": bool}` — the desktop only.

The hint "Update the plugin to give sessions Mother Claude's tools" shows
when the plugin is installed but sessions load another version than this app
writes (`folderVersion` of `claude plugin list --json`, else its `version`).

## For tool authors

Register in `mcp::registry()` (one line per area: core, browser, teams,
machines); see the contract in `mcp/mod.rs`. A tool:

* gets `ToolCtx { state, caller }` — check `caller.session_id` / `owned`
  when it matters, and say so with a `ToolError` when it cannot tell;
* returns `ToolOutput` (`text`, `json`, images) or a `ToolError` with a code
  (`invalid_arguments`, `not_found`, `denied`, `timeout`, `unavailable`, …);
* sets `annotations.read_only_hint: Some(true)` when it changes nothing (it
  then skips the audit line);
* is added with `Registry::add_with_limit(def, Some(limit), handler)` when
  one call may outlast the server's 120 s ceiling — it waits for a person or
  a build, as `machine_run` does — which sets `Tool::limit`; `Registry::add`
  keeps the ceiling, and inside it a tool still keeps its own, shorter
  limits;
* raises approvals through `crate::pending` for anything dangerous, and
  never panics.

## Troubleshooting

| Symptom | Cause / fix |
|---|---|
| `/mcp` in a session shows `plugin:mother-claude:mc` failed, `ECONNREFUSED` | Mother Claude was not running when the session started (the CLI tries three times, then gives up). Start the app, then reconnect in `/mcp` or restart the session. |
| Failed with `Dynamic Client Registration rejected` or an OAuth error | The helper sent no valid token, so Claude Code tried OAuth against the server (`/.well-known/oauth-*`, `POST /register`). Check that `<app data>/mcp.token` exists (the setting is on) and that the entry quotes the helper. |
| No `mcp__plugin_mother-claude_mc__…` tools in a foreign session | Foreign control is not installed for that account, or the session started before the entry existed: `/reload-plugins`. |
| Calls fail with `mcp_disabled` | Services › Claude Code tools is off. |
| A `machine_*` tool says Machines is off, *Update Mother Claude on …*, or that a computer has not given this one the Run permission | See [MACHINES.md](MACHINES.md) "Troubleshooting". |
| A tool says it cannot tell which session is calling | The CLI's pidfile was not readable yet, or the session runs under a config directory Mother Claude does not know. |
| Sessions break after changing `MOTHER_CLAUDE_PORT` | The URL is fixed when a session starts: restart the session. |

## Spike B: verified on the real CLI (2026-11-06)

Claude Code 2.1.319 (terminal) and 2.1.318 (the binary the VS Code
extension bundles), macOS, `claude -p`, haiku, about $0.003 a run. The
`#[ignore]`d tests in `tests/mcp.rs` repeat it:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --test mcp -- --ignored --nocapture
MOTHER_CLAUDE_CLI=/path/to/claude cargo test … --test mcp -- --ignored   # another build
```

* **End to end**: a throwaway server on an ephemeral port, the plugin written
  into a temp data dir with a space in its path and loaded with
  `--plugin-dir`: `plugin:mother-claude:mc` connected, the model was offered
  `mcp__plugin_mother-claude_mc__mc_ping` and `…__mc_session`, loaded
  `mc_ping` through ToolSearch and called it, and the server named the
  session from `X-MC-Pid`. An owned session launched by `ControlRegistry::
  spawn` loaded its copy inline and was named by `X-MC-Session`
  (`owned: true`).
* **`headersHelper` runs once per connection**, not per request: every
  request of a connection (`server/discover`, `initialize`,
  `notifications/initialized`, the SSE `GET`, `tools/list`, `tools/call`)
  carried the same headers. It runs again on each reconnection: after a 401
  (token rotated) the CLI re-ran it, reconnected and retried the call once,
  transparently. With the server down at start it tried three times, then
  reported `failed: ECONNREFUSED` without holding the session up.
* **The CLI holds its MCP connection itself**: the binding check of the
  request's peer port against the CLI's pid answered `Verified`.
* **`$PPID` is the CLI** (the helper's parent process is `claude`). The
  helper's environment has `CLAUDE_CODE_MCP_SERVER_NAME`
  (`plugin:<plugin>:<server>`), `CLAUDE_CODE_MCP_SERVER_URL`,
  `CLAUDE_PLUGIN_ROOT` and the CLI's own variables — **no session id**, so the
  plan's "`X-MC-Session` from `CLAUDE_SESSION_ID`" is not possible; owned
  sessions get `MOTHER_CLAUDE_SESSION` from us, and foreign ones are known by
  their pid.
* **It runs through a shell**: an unquoted helper path with a space never ran;
  quoted (`"\"${CLAUDE_PLUGIN_ROOT}/…\""`) it did, `$PPID` unchanged.
* **No headers → OAuth**: on 401 the CLI fetched
  `/.well-known/oauth-protected-resource/mcp`, `…/oauth-protected-resource`,
  `…/oauth-authorization-server`, `…/openid-configuration` and posted
  `/register`, then failed with "Dynamic Client Registration rejected".
* **Handshake**: first `POST server/discover` (id
  `server-discover-probe-1`, `MCP-Protocol-Version: 2026-07-28`); after
  -32601, `initialize` with `protocolVersion: "2025-11-25"` and no version
  header; then `MCP-Protocol-Version` and `Mcp-Session-Id` on every request.
  `User-Agent: claude-code/2.1.319 (sdk-cli)`.
* **Workspace trust** does not gate a plugin's helper: it ran in an untrusted
  temp cwd (the CLI's trust gate for `headersHelper` is for repository
  `.mcp.json` files).
* **Inline shadows installed**: `--plugin-dir` with a plugin named
  `mother-claude` replaced the installed `mother-claude@mother-claude-local`
  for that session (one plugin in `system/init`, `source: …@inline`).
* **In place**: the installed plugin loads from the app's folder
  (`system/init` path = the folder, version = its manifest's). `claude plugin
  list --json` reports the stale install record as `version` (0.3.0 here)
  plus `readFromFolder` and `folderVersion` (what sessions load).
* `claude plugin list` is **not read-only**: in an empty config directory it
  created `.claude.json` and a backup.
* A plugin version with build metadata (`0.7.0+mcp.1`) passes `claude plugin
  validate`.

**Not verified**: the VS Code extension's own UI, JetBrains and the Desktop
app's Code tab (they run the same CLI and load user-scope plugins, but were
not driven); Windows (the helper is a `sh` script run by the shell Claude Code
uses there, Git Bash; quoting a Windows path is untested); directory
accounts' installs (they read the same folder in place by design); a session
that started while the app was closed reconnecting by itself (it does not, in
`-p`; interactive `/mcp` reconnect was not tried).

**Decided in Phase 0**: a hand-rolled handler over axum rather than the
`rmcp` crate — a few hundred lines, no new dependency, and it reuses the
`ClientContext`, the guards and the error envelope; tools-only, JSON
answers, no SSE.
