# Plan: a browser for Claude Code, and teams of sessions

**Status: proposal, 2026-11-06. Nothing in this document is built.** It is the
feasibility analysis and the implementation plan for two requests: a Mother
Claude browser extension that lets Claude Code work in the user's real Chrome,
and session segmentation with task delegation, model selection, resource
management, non-development workflows and remote management. It also answers
whether the current stack (Tauri + Rust + Angular) should stay.

Everything marked **verified** was checked on 2026-11-06 against Claude Code
2.1.318 installed on this Mac (its `--help`, its binary's strings, the bundled
Agent SDK 0.3.304), the official docs (URLs in Appendix A), Chrome 156, and
this repository. Everything marked **spike** is plausible but unverified and
is a Phase 0 task. Everything marked **not today** cannot be done with the
documented platform.

---

## 0. Summary and verdicts

| Requirement | Verdict | What exists today | What Mother Claude builds |
|---|---|---|---|
| 1. Browser extension driven by Claude Code | **Feasible.** | Claude Code already ships *Claude in Chrome* (`claude --chrome`): the official extension, exposed to the model as the MCP server `claude-in-chrome`. It needs a claude.ai sign-in (Pro/Max/Team/Enterprise), excludes API keys and `setup-token` accounts, is off in `bypassPermissions` sessions and under WSL, and its transport is undocumented. | A Mother Claude extension (Chrome/Edge first) that connects to the desktop app over loopback, with an MCP server every Claude Code surface reaches through the already-installed Mother Claude plugin. It works for every account kind, for owned `claude -p` sessions, and its approvals show on the desktop and the phone. Claude in Chrome stays supported side by side. |
| 2.1 Automated model selection | **Feasible, with honest limits.** | Subagents take `model: sonnet\|opus\|haiku\|fable\|<id>\|inherit` and `effort`; sessions take `--model`, `--effort`, `--fallback-model a,b,c`, `--max-budget-usd` (print mode); Mother Claude already switches an owned session's model/effort live. | A routing policy (role defaults, task class, escalation ladder, budget state) and the measurement loop that makes it honest. "Complexity" is estimated by rules plus a Haiku classifier, never known. |
| 2.2 Session segmentation (CEO + specialists) | **Feasible — not the way the docs' "agent teams" do it.** Agent teams are experimental and **interactive only**: a `-p` session (the kind Mother Claude owns) never spawns teammates. | **Cross-session messaging** (v2.1.224+, on by default, local socket, every provider): every local session, `-p` included, has an inbox; `SendMessage`/`ListAgents` reach it by name; a `-p` worker takes messages unattended with `crossSessionInbound: accept`. | Named owned sessions per role, Mother Claude's task board as MCP tools, delegation over cross-session messages, a scheduler that starts role sessions on demand. Each role session is a normal Claude Code session (resumable in VS Code, visible to the phone). |
| 2.3 Resource management, parallelism | **Feasible.** The scarce resources are plan usage windows, dollars, repository contention and attention — not CPU/GPU. | Mother Claude already tracks the 5-hour and weekly windows, per model family (`usage.rs`); a session costs 190–450 MB RSS here (measured); GPU is irrelevant (no local inference). | An admission controller: RAM headroom, plan windows per model family, per-project caps, priorities, per-worker budgets. |
| 2.4 Cost and quality optimisation | **Feasible as a measured loop; the illustrative numbers are not a policy.** | Transcript usage is already indexed per session and agent. | Cost per *completed* task per model/effort, quality signals (tests, review verdicts, retries), reports, then threshold tuning. Anthropic's guidance: tune effort first; a cheaper model that needs more turns is not cheaper; caches are model-scoped. |
| 2.5 Workflows beyond development | **Feasible by design, not by a new engine.** | Roles are agent definitions; tools are MCP servers; the browser extension covers web work; files are artifacts. | Workflow templates (roles, tools, folders, acceptance checks) that are not coding-specific. |
| 2.6 Remote management | **Mostly exists.** | Pairing, LAN TLS, tunnels, push, per-device dangerous grants, relays, approvals on the phone. | Team view, task board, assignment, browser-action approvals, budget alerts on the phone; new dangerous classes. |
| 3. Native apps per OS | **Not recommended.** | The Rust core and the Angular SPA already run on both systems, one code path for desktop and phone. | Keep the stack; add native *pieces* behind Rust (Windows Credential Manager, native messaging registration if ever needed). |

The decisions that differ from the request's starting assumptions:

1. **Don't go native.** Nothing requested needs it, and a second UI would cost the one-SPA-for-desktop-and-phone invariant (§3).
2. **Build the extension as a complement to Claude in Chrome, not a replacement**, and expose it through MCP so it works in every surface and for every account kind (§5).
3. **Don't build "Session 0..N" on agent teams.** Build it on named `-p` sessions plus cross-session messaging plus a task board in Mother Claude, with Mother Claude as the scheduler (§6). Agent teams stay available to the user in interactive sessions.
4. **Schedule by plan windows and budgets, not by CPU/GPU** (§9).
5. **Start model routing static and measured; escalate on evidence; don't build a cost cascade before measuring the top model at lower effort** (§7).

---

## 1. Feasibility, requirement by requirement

### 1.1 Browser extension (request 1)

What the platform already does — **verified**:

- `claude --chrome` / `--no-chrome`, the `/chrome` dialog with an *Enabled by default* setting shared with VS Code (`@browser` in its prompt box). The first enable installs a native messaging host manifest `com.anthropic.claude_code_browser_extension.json` (macOS `~/Library/Application Support/Google/Chrome/NativeMessagingHosts/`, Linux `~/.config/google-chrome/NativeMessagingHosts/`, Windows `HKCU\Software\Google\Chrome\NativeMessagingHosts\`) for the extension id `fcoeoabgfenejglbffodgkkbkcdhcgfn`, and the docs require network access to `bridge.claudeusercontent.com`. The transport beyond that is undocumented (the CLI's own names for it are `chrome_bridge_*`, `chrome_native_host_*`).
- The model sees an MCP server named `claude-in-chrome` (a reserved name). Tool names seen in the CLI binary: `navigate`, `read_page`, `find`, `form_input`, `get_page_text`, `screenshot`, `computer` (click, type, scroll, zoom…), `javascript_tool`, `read_console_messages`, `tabs_context_mcp`, `tabs_create_mcp`, `tabs_close_mcp`, `gif_creator`, `upload_image`, `browser_batch`. Only the last two are in the docs.
- Browsers: Chrome and Edge named; Brave, Arc, Vivaldi, Opera detected (the Mac binary also looks for Chrome Canary and Chromium). WSL unsupported. It shares the browser's signed-in state and opens tabs in a per-session tab group.
- Account rules: a direct Anthropic plan signed in with `/login`. An API key or a `claude setup-token` token keeps it off even with `--chrome` — so **Mother Claude's token accounts never get browser tools from Claude in Chrome**. It is also off in a `bypassPermissions` session ("Claude in Chrome is not wired into that configuration").
- Surfaces: CLI and VS Code documented; JetBrains and the Desktop app's Code tab not documented; `claude agents` and `claude remote-control` can enable it for the sessions they dispatch (`--[no-]chrome`). The CLI contains host-facing request kinds (`get_chrome_dialog`, `select_chrome_browser`, `chrome_setup_card_answer`, `chrome_permission_prompt`, `chrome_open_url`) described as "for a host that draws the dialog with its own widgets" — internal, undocumented. Whether an SDK host or a `claude -p` process can use them is a **spike**.
- Chrome 136+ ignores `--remote-debugging-port` for the default profile: a tool that wants the user's real, signed-in profile cannot attach by CDP from outside; it needs an extension (`chrome.scripting`, `chrome.debugger`) or Chrome's own `chrome://inspect` toggle. This rules out "just use Playwright/CDP against the running Chrome".
- MV3 service workers idle out after 30 s, but since Chrome 116 WebSocket activity resets the timer; Chrome's guidance is a keepalive every 20 s.

Why build our own extension at all, then:

| Need | Claude in Chrome | Mother Claude extension |
|---|---|---|
| Works with token accounts, API keys, Bedrock/Vertex | No | Yes (local only, no claude.ai dependency) |
| Works for Mother Claude's owned `claude -p` sessions | Spike (undocumented host requests) | Yes (MCP) |
| Works in `bypassPermissions` sessions | No | Yes, gated by Mother Claude's own approvals |
| Approvals for risky actions on the phone | No | Yes (`crate::pending`, same cards as tool approvals) |
| Per-site read-only / allow lists set by the user, audited | Extension's own settings | Mother Claude settings, audit lines, device grants |
| Several sessions sharing one browser without fighting | Per-session tab group | Browser leases per session (tab group per session) + a queue |
| Deterministic, token-cheap observation (ref-based accessibility snapshot) | `read_page`/`find` (unknown shape) | Designed for it (§5.3) |
| Other browsers (Firefox, Safari) | Chromium only | Chromium first; Firefox feasible; Safari later |
| Transport entirely local | No (`bridge.claudeusercontent.com`) | Yes |
| Zero build effort | Yes | No |

**Verdict: feasible, complementary.** Phase 1 builds the extension for Chrome/Edge/Brave (one MV3 code base), connected to Mother Claude over loopback, exposed to every Claude Code session through the plugin's MCP server. Claude in Chrome remains the zero-setup option for interactive sessions, and Mother Claude surfaces its state for owned sessions if the spike succeeds.

### 1.2 Automated model selection (2.1) — feasible with limits

**Verified** levers: subagent frontmatter `model` (`sonnet`, `opus`, `haiku`, `fable`, a full id, `inherit`) and `effort` (`low`…`max`), `permissionMode`, `maxTurns`, `isolation: worktree`, `memory`, `mcpServers`, `hooks`, `background`; the `--agents <json|file>` flag (file with `-p`, v2.1.281+); the resolution order per-call `model` → frontmatter → `CLAUDE_CODE_SUBAGENT_MODEL` → main model (`CLAUDE_CODE_SUBAGENT_MODEL_FORCE=1` overrides); session `--model`, `--effort`, `--fallback-model a,b,c` (up to three), `--max-budget-usd` (print mode, client-side estimate, subagent spend counts), `--max-turns`; live `set_model`/`set_permission_mode`/effort changes on owned sessions (Mother Claude's `POST /api/sessions/{id}/model|effort|mode`); `availableModels`/`deniedModels` in managed settings (v2.1.283+). Effort defaults: Opus 5.5, Sonnet 5.5, Haiku 5.5 default to `medium`; `effortLevel` in user settings is ignored by Opus 5.5 and later — pass `--effort` at launch.

What is **not today**: Claude Code has no "pick the model by task complexity" switch. The lead session chooses when it delegates (the Agent tool's `model` parameter, or which subagent it calls); the operator chooses at launch. So the automation lives in Mother Claude: role defaults, a task classifier, an escalation ladder, a budget gate, and a measurement loop (§7).

### 1.3 Session segmentation (2.2) — feasible, on cross-session messaging

**Verified**, and decisive:

- **Agent teams** (`CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1`): experimental; teammates are separate Claude Code instances shown in-process or in tmux/iTerm2 panes; **a `-p` session, SDK sessions included, does not spawn teammates**; a teammate inherits the lead's permission mode (`bypassPermissions` included) and cannot have its own; teammates are not worktree-isolated; `/resume` does not restore in-process teammates; one team per session; the docs suggest 3–5. Hook events `TeammateIdle`, `TaskCreated`, `TaskCompleted`. The shared Task tools are not on by default for Claude 5 models (`CLAUDE_CODE_ENABLE_TODO_TOOLS=1` opts in).
- **Cross-session messaging**: v2.1.224+ (macOS/Linux/WSL 2), v2.1.234+ native Windows; on by default; same-machine messages go over a per-session socket and never through Anthropic's servers; works on every provider; a `claude -p` session binds an inbox and appears in `ListAgents` (`--bare` does not); `crossSessionInbound: accept|hold|refuse` — a `-p` worker takes messages unattended with `accept` in its `--settings`; held messages in a `-p` session drop after `dialogExpiry` (5 min); `notify_when_idle` gives one notice when the watched session next goes idle (12-hour subscription); plain text up to ~1M characters; bursts are refused, loops throttled, 50 queued; a message from another session never approves anything and never changes configuration. Cross-machine needs Remote Control and a claude.ai sign-in.
- Sessions can be named at launch (`-n, --name`), run in a new git worktree (`-w, --worktree [name]`), run as an agent definition (`--agent <name>`, "overrides the 'agent' setting" — whether that applies a definition's model/tools/prompt to the whole main session is a **spike**), take extra directories (`--add-dir`), per-run settings (`--settings <file|json>`), MCP config (`--mcp-config`, `--strict-mcp-config`), and a budget (`--max-budget-usd`).
- Mother Claude already launches owned sessions with the stream-json control protocol (`claude/control.rs`, `SpawnRequest { cwd, prompt, model, effort, permission_mode, thinking, images, account }`), relays messages into foreign sessions through the hooks plugin, and tracks every session from its pidfile and transcript.

**Design**: each role is a named owned session; the CEO delegates with `SendMessage` (built in) and claims/updates work on Mother Claude's task board (MCP tools the plugin exposes); Mother Claude decides when a role session actually starts (capacity), with worktree isolation and a path-ownership guard on `PreToolUse` (§6, §8). Agent teams are not used by the orchestrator; a user who runs an interactive session may still use them, and Mother Claude shows the team's sessions like any others.

### 1.4 Resource management (2.3) — feasible; the request's premises need correcting

Measured here: three live VS Code sessions use 187, 390 and 445 MB RSS; the machine has 24 GB and 12 cores. A Claude Code session is a network-bound Node process; CPU is spent by what it runs (builds, tests), GPU by nothing. The limits that matter are the plan's 5-hour and weekly windows (Mother Claude reads `five_hour`, `seven_day`, `seven_day_opus`, `seven_day_sonnet`, `seven_day_overage_included`), API dollars on API-key accounts, repository contention, disk for worktrees, and the user's attention for approvals. §9 specifies the admission controller.

### 1.5 Cost and quality (2.4) — feasible as measurement, not as a formula

First-party API rates (Anthropic, cached 2026-10-06): Fable 5.1 $10/$50 per MTok, Opus 5.5 $4/$20, Sonnet 5.5 $2/$10, Haiku 5.5 $0.10/$0.50 (prompts ≤100K). On Pro/Max plans the cost is window consumption instead, and the Opus and Sonnet weekly windows are separate — which the scheduler can use. Anthropic's own cost guidance (the `claude-api` skill): effort is the first quality-trading lever; measure the most capable model at lower effort before building a multi-model cascade; prompt caches are model-scoped, so a cascade forfeits cache reuse; judge cost per completed task, not per request. The request's video-production figures are illustrative; the plan treats them as a hypothesis to test with the measurement loop in §7.

### 1.6 Beyond development (2.5) — feasible by construction

A role is an agent definition (prompt, model, effort, tools, MCP servers, memory, permission mode); a workflow is a set of roles plus folders, artifacts and acceptance checks; the browser extension is the tool that makes web-bound business work possible. §6.6 lists templates. Honest limit: quality judgement for non-code work needs a rubric and a reviewer role, not tests.

### 1.7 Remote (2.6) — mostly exists

Mother Claude already has device pairing (`mcd_` tokens), LAN TLS with a local CA, a tunnel listener with internet pairing, push notifications, per-device dangerous grants (LAN and internet separately), relays into sessions, and approvals answered from the phone. Claude Code's own **Remote Control** (claude.ai/code and the mobile app; Pro/Max/Team/Enterprise with a claude.ai sign-in; not API keys; not Bedrock/Vertex/Foundry) is a complement for one session at a time. §10 adds what the team features need.

### 1.8 Native apps (3) — not recommended; see §3.

---

## 2. The current architecture and its limits

What Mother Claude is today (verified in the repository and `CLAUDE.md`):

- **Rust core** (`src-tauri/src/claude/`): an adapter over `~/.claude` (transcripts, pidfiles, jobs, subagents, workflows), owned-session control (Path B: `claude -p` with the stream-json control protocol; `initialize`, `can_use_tool` as pending items, model/mode/effort/rename/stop-task/interrupt requests), accounts (Default, directory, token; Keychain on macOS, 0600 files elsewhere), backups, usage windows, activity derivation.
- **Axum server** (`src-tauri/src/server/`): every byte of dashboard data, REST + WebSocket diffs, a principal-based authorization model (`Desktop`, `Device`, `LocalScript`, `Hook`, `Sidecar`, anonymous bridge callers), pairing, LAN TLS, tunnel listener, push, the pending store (`crate::pending`) with server-side dangerous classification, audit lines.
- **Hooks plugin** (`src-tauri/assets/claude-plugin/`): user-scope plugin, exec-form hooks for SessionStart, UserPromptSubmit, PreToolUse (decide), PermissionRequest (held, first answer wins), PostToolUse, Notification, SubagentStart/Stop, PreCompact/PostCompact, Stop, SessionEnd; `mc-wake` for idle sessions. No MCP server.
- **HTTP bridge** (`/v1` and loopback port 5612): conversations, turns, SSE, tool approvals, images, direct Messages API; several API servers; a Node sidecar (`sidecar/`) holding the Agent SDK 0.3.304 (`query`, `createSdkMcpServer`, `tool`) behind `MOTHER_CLAUDE_SIDECAR=1`.
- **Angular SPA**: one code path for the desktop webview and the phone.

Limits the requests run into:

| Limit | Consequence | Addressed in |
|---|---|---|
| No MCP server of our own | Claude Code sessions cannot call Mother Claude (task board, browser) | §5.4, §6.2 |
| No browser channel | Nothing reaches a page | §5 |
| No notion of a team, role or task | Sessions are flat; delegation is the model's private business | §6 |
| Scheduler is per-session (backup idle rules, usage display) | No admission control across sessions | §9 |
| Model/effort are per-session switches, no policy | No routing, no escalation, no measurement | §7 |
| Hooks plugin predates `TeammateIdle`, `TaskCreated`, `TaskCompleted`, `WorktreeCreate/Remove`, `PreModelSwitch/PostModelSwitch`, `FileChanged`, `ConfigChange`, `Elicitation*` (33 events exist) | Blind to team and worktree events | §6.4 |
| Secrets on Windows are 0600 files | Weaker than macOS | §11 |
| Foreign sessions: monitoring and documented channels only (the hard invariant) | Orchestration must be built on *owned* sessions | §6 (by design) |

Nothing here is wrong for what the app does today; the additions are layers, not rewrites.

---

## 3. Stack decision: keep Tauri + Rust + Angular

| Criterion | Keep the stack | Native Swift + C#/WinUI apps |
|---|---|---|
| Capabilities needed by these requests | All reachable: loopback WebSocket, process control, Keychain/Credential Manager, file watching, TLS — from Rust crates | Same capabilities, written twice |
| The browser extension | Web technology either way; its counterpart is the server, which is Rust | Same |
| The phone | The SPA is the phone UI today; one code path | A second UI stack, or keep Angular only for the phone — then two UIs anyway |
| Team/scheduler/routing logic | Rust, shared | Written twice or hoisted into a shared Rust core (= Tauri without Tauri) |
| Performance | The hot paths (indexing, encryption, process supervision) are already Rust | No gain; the UI is not the bottleneck |
| Security | Principal model, dangerous classification, pending store exist | Rebuilt twice |
| Maintainability | One team, one release flow (macOS + Windows legs exist) | Two code bases, two release flows, two sets of bugs |
| Development cost | Additive | Months of parity work before any new feature |

**Where native pieces earn their place (inside Rust, no second app):** Windows Credential Manager for secrets (`keyring` crate or `windows-sys` `CredWriteW`), macOS Keychain (done), native messaging host registration if a later phase wants browser-side discovery (a manifest file on macOS/Linux, `HKCU` registry keys on Windows), launch-at-login. The one place a different language is already used — the Node sidecar — stays for the Agent SDK and may host a stdio MCP shim if HTTP MCP proves awkward (§5.4).

**Revisit this decision only if** a requirement appears that Tauri's webview cannot render (none here), or if the phone UI is split from the desktop UI for product reasons.

---

## 4. Target architecture

```
                 ┌──────────────────────────────── Chrome / Edge / Brave ────────────────────────────┐
                 │  Mother Claude extension (MV3)                                                   │
                 │   service worker: WebSocket client, command router, tab registry, leases         │
                 │   content scripts: snapshot (a11y tree + refs), read, fill, click, observe       │
                 │   chrome.debugger (optional): trusted input, screenshots, console, network       │
                 │   side panel: activity, Stop, per-site mode, pending approvals                   │
                 └───────────────▲──────────────────────────────────────────────────────────────────┘
                                 │ wss/ws loopback, paired token, Origin chrome-extension://<id>
┌────────────────────────────────┴───────────────── Mother Claude (Tauri app) ─────────────────────────────┐
│  axum server (existing)                                                                                   │
│   ├─ /ws/browser     extension channel (principal Extension)                                              │
│   ├─ /mcp            MCP server (Streamable HTTP): browser_*, task_*, team_*, project_* tools             │
│   ├─ /api/...        dashboard REST/WS (existing) + team, task board, routing, scheduler, browser audit   │
│   ├─ /hooks/*        hooks plugin events (existing) + TeammateIdle/TaskCreated/TaskCompleted/Worktree*    │
│   └─ /v1, :5612      HTTP bridge (existing)                                                               │
│  orchestration (new Rust modules)                                                                         │
│   ├─ teams: workflow templates, roles (agent definitions), launch plans                                   │
│   ├─ board: tasks, packets, claims, results, history                                                      │
│   ├─ router: model/effort policy, escalation, measurement                                                 │
│   ├─ scheduler: admission control (RAM, plan windows, budgets, caps, priorities)                          │
│   └─ browser: leases, dangerous-action classifier, approvals via crate::pending, audit                    │
│  claude core (existing): owned sessions (claude -p stream-json), index, usage, accounts, hooks, pending   │
└───────┬───────────────────────────────┬──────────────────────────────┬────────────────────────────────────┘
        │ spawn / control / transcripts  │ hooks (plugin)               │ MCP (plugin .mcp.json → /mcp)
┌───────▼───────┐   SendMessage   ┌──────▼────────┐   SendMessage   ┌───▼───────────┐
│ CEO session   │◄───────────────►│ Frontend role │◄───────────────►│ Backend role  │ … (each: claude -p,
│ (owned, named)│   local sockets │ (owned, named,│                 │ (owned, named,│  named, worktree,
└───────────────┘                 │  worktree)    │                 │  worktree)    │  budget, settings)
                                  └───────────────┘                 └───────────────┘
          Any other Claude Code session on the machine (VS Code, Desktop, terminal) reaches the same
          /mcp tools through the user-scope plugin, and the same browser.
                                             ▲
                                             │ LAN TLS / tunnel, paired devices, push (existing)
                                      ┌──────┴──────┐
                                      │   Phone     │  team view, task board, approvals, budgets
                                      └─────────────┘
```

Component responsibilities:

| Component | Owns | Never does |
|---|---|---|
| Extension | Page observation and actions in the user's browser; per-site mode; the Stop button | Decide what is dangerous (the server does); store page data beyond the request |
| `/ws/browser` | Authenticating the extension, routing commands, leases, timeouts | Accept a non-loopback peer or a non-extension Origin |
| `/mcp` | Tool schemas for sessions; mapping tools to board/browser/team operations; per-session identity | Bypass the pending store for dangerous actions |
| teams/board | Roles, tasks, packets, results; the single source of truth the CEO and workers share | Talk to the model (the sessions do) |
| router | Model/effort choice per task; escalation; measurement records | Switch a session's model without recording why |
| scheduler | Admission, queueing, caps, budget gates, preemption at turn boundaries | Kill a session mid-turn on its own |
| Phone | Everything the desktop shows, minus desktop-only admin | Dangerous actions without a grant |

---

## 5. Browser extension: architecture and integration

### 5.1 Communication: extension ⇄ Mother Claude

**Decision: a WebSocket to the existing loopback server, authenticated like a paired phone.** Native messaging was evaluated and kept as an optional discovery aid (below).

| | Loopback WebSocket (chosen) | Native messaging host |
|---|---|---|
| Discovery | The extension's options page: host:port (default `127.0.0.1:6725`), pairing code shown by the desktop | Automatic: Chrome launches the host from a registered manifest |
| Processes | None new; the axum server exists | A host process per connection, started by Chrome, which must then reach the app (a second hop) |
| Registration | None | Manifest per browser per OS (file on macOS/Linux, `HKCU` keys on Windows), app must keep them current |
| Security | Token + `Origin: chrome-extension://<id>` check + loopback peer; the server's `ClientContext` already models all three | Manifest's `allowed_origins` binds the extension id; stdio pipes |
| Keeps the service worker alive | Yes (Chrome 116+: socket activity resets the 30 s idle timer; keepalive every 20 s) | Port messages count as activity too |
| Multiple browsers (Chrome + Edge) | Each pairs separately | Each needs its own manifest location |
| Firefox / Safari later | Same code | Firefox supports it; Safari needs an app extension |

Protocol: JSON frames `{id, kind, …}` with request/response correlation, server→extension commands (`snapshot`, `click`, …) and extension→server events (`tab.updated`, `dialog.opened`, `download.started`, `user.stopped`). The extension holds the paired token in `chrome.storage.local` (the phone's `mcd_` tokens are the model; the extension gets its own prefix, e.g. `mcx_`). A detected tunnel never applies: the channel is loopback-only (`ctx.on_machine`), and `Origin` must be the extension's.

Optional, later: a tiny native messaging host (`mc-nmhost`) whose only job is to hand the extension the current port and a one-time pairing code, so pairing is a click. Not needed for Phase 1.

### 5.2 Extension internals (MV3)

- **Manifest**: `manifest_version: 3`; `minimum_chrome_version: "116"`; permissions `storage`, `tabs`, `tabGroups`, `scripting`, `activeTab`, `sidePanel`, `alarms`; **optional** `debugger`, `webNavigation`, `downloads`, `clipboardRead/Write`; **optional host permissions** `<all_urls>` requested per site at runtime (`chrome.permissions.request`) so a fresh install can do nothing until the user grants a site or "all sites". No remote code (Web Store policy); everything bundled.
- **Service worker**: WebSocket client with reconnect/backoff and a 20 s keepalive; command router; tab registry; lease table (session → tab group); a watchdog that times out commands (default 30 s, configurable per tool); event forwarding.
- **Content scripts** (injected on demand with `chrome.scripting.executeScript`, isolated world): DOM/accessibility snapshot with stable `ref`s (e.g. `e12`), text extraction (readability-style main content to Markdown), form filling (native setters + `input`/`change` events, React/Vue-safe), synthetic clicks for the common case, scroll-into-view, wait-for conditions (selector, text, URL change, network idle via `PerformanceObserver`), highlight of the acted element (visible to the user).
- **`chrome.debugger`** (optional permission, granted per browser once): trusted input events (`Input.dispatchMouseEvent`, `Input.dispatchKeyEvent`) for sites that ignore synthetic events, `Page.captureScreenshot` (viewport or element clip), `Runtime` console capture, `Network` summaries, file uploads via `DOM.setFileInputFiles`. Chrome shows an infobar while attached; the side panel explains it; detach when the lease ends.
- **Side panel**: live activity ("Session *Frontend* is reading `github.com/...`"), **Stop** (revokes the lease instantly, the server fails the in-flight command), per-site mode (`off` / `read` / `act`), pending approval cards (the same cards as the dashboard, answered here or on the phone), connection state, pairing.
- **Options page**: host/port, pairing, default site mode, "always ask before" categories.
- **Tab groups**: one colored group per session lease, named after the session, so the user sees whose tabs are whose. A session acts only inside its lease; the user's own tabs are never touched unless the user grants the current tab (`activeTab`).

### 5.3 Tool contract exposed to the model (`browser_*`)

Ref-based snapshots are the observation primitive (token-cheap, deterministic); screenshots are on demand. Names follow the `browser_<verb>` convention so prompts and skills written for other MCP browsers transfer; the implementation is ours.

| Tool | Input → Output | Notes |
|---|---|---|
| `browser_tabs` | `{action: list\|open\|select\|close, url?, tabId?}` → tabs in the lease | Opening acquires the lease |
| `browser_navigate` | `{url, waitUntil?}` → `{url, title}` | Blocked by site mode `off`; sensitive-site list → approval |
| `browser_snapshot` | `{scope?: viewport\|page\|selector, interactiveOnly?, maxNodes?}` → accessibility tree with `ref`s | Default `interactiveOnly: false`, capped; the cap is the main token lever |
| `browser_read` | `{mode: text\|markdown\|html, selector?, maxChars?}` → content | Main-content extraction by default |
| `browser_find` | `{text?, role?, name?, selector?}` → matching refs | Cheap alternative to a full snapshot |
| `browser_click` / `browser_hover` | `{ref\|selector, button?, trusted?}` → new snapshot delta | `trusted` uses `chrome.debugger` |
| `browser_type` | `{ref, text, submit?, clear?}` | Never types into a `password` field unless the user grants it per site; secret values come from the user's own paste, never from the model |
| `browser_select` / `browser_check` | `{ref, value\|values}` | |
| `browser_press` | `{key, modifiers?}` | |
| `browser_scroll` | `{ref?, direction?, amount?}` | |
| `browser_fill_form` | `{fields: [{ref, value}], submit?}` | One approval for the whole form when any field is sensitive |
| `browser_wait` | `{for: selector\|text\|url\|networkIdle\|ms, timeoutMs?}` | |
| `browser_screenshot` | `{scope?: viewport\|element, ref?}` → PNG (base64) | Needs `debugger`; allowed in `read` and `act` modes, refused in `off`; a per-site "no screenshots" setting overrides |
| `browser_eval` | `{expression}` → JSON | **Dangerous**: needs the dangerous right (desktop, or a device with the grant); audited |
| `browser_console` | `{since?}` → console lines | Needs `debugger` |
| `browser_download` / `browser_upload` | `{ref, path}` | Paths validated against the session's allowed directories; approval |
| `browser_dialog` | `{action: accept\|dismiss, text?}` | JavaScript dialogs block the page until handled |
| `browser_record` (later) | start/stop → a replayable macro saved as a skill | §16 |

Rules enforced server-side, whatever the model asks: site mode; the dangerous-action classifier (§10); one lease per session; command timeouts; snapshot size caps; redaction of values in `type=password` and fields named like secrets in every snapshot and read; no screenshots of pages in `read` mode when the user chose "no screenshots"; everything audited.

### 5.4 Integration with Claude Code: an MCP server through the plugin

**Verified**: a plugin declares MCP servers in `.mcp.json` at its root or `mcpServers` in `plugin.json`; stdio, SSE, HTTP and WebSocket servers are allowed, with `url`, `headers`, `headersHelper`; substitution covers `${CLAUDE_PLUGIN_ROOT}`, `${CLAUDE_PLUGIN_DATA}`, `${CLAUDE_PROJECT_DIR}` and `${user_config.KEY}` (`claude plugin configure <plugin> --values-stdin` saves values); tool names become `mcp__plugin_mother-claude_<server>__<tool>`; plugin servers rank after local/project/user entries and are toggled in `/mcp`. `claude mcp add --transport http <name> <url> --header "Authorization: Bearer …"` exists for a manual setup.

**Decision**: the Mother Claude plugin gains an MCP server entry. The plugin is a *local-directory marketplace in Mother Claude's own data dir*, loaded in place by the CLI and rewritten on every start (`claude/hooks.rs`: "later versions of the scripts reach every new session without reinstalling"), so the entry is written there with the rest of the plugin files — never with `claude plugin configure`, which would write into `~/.claude` in the background (forbidden: nothing writes to a config directory in the background, ever). The entry is `{"type":"http","url":"http://127.0.0.1:<port>/mcp","headersHelper":"${CLAUDE_PLUGIN_ROOT}/scripts/mc-mcp-headers"}`, the port baked in at each start; the helper, like `mc-hook`, reads a 0600 token file in the data dir and prints the `Authorization` header plus `X-MC-Pid: $PPID` (the calling CLI's pid, which the server maps to a session through the pidfiles). Fallback if `headersHelper` proves unreliable in a surface (**spike**): a stdio shim `mc-mcp` in the plugin's `scripts/` that reads the same runtime files (`hook.base`, a token file) and proxies stdio ⇄ HTTP; on Windows the hooks already run under the shell Claude Code uses there, and the shim follows the same path. A session started with the plugin disabled, or with `MOTHER_CLAUDE_OWNED=1`, gets the server another way: owned sessions through `--mcp-config` at launch.

The MCP server runs inside axum (`/mcp`, Streamable HTTP, JSON-RPC; the `rmcp` crate or a hand-rolled handler — decide in Phase 0 by a two-day spike). It identifies the calling session by the token plus the `X-MC-Session` header the plugin's `headersHelper` can add from `CLAUDE_SESSION_ID`, else by matching the caller's pid (same mechanism the hooks use to recognise a session). Owned sessions can also get the server through `--mcp-config` at launch, which needs no plugin.

Why MCP and not the stream-json control channel: MCP reaches **every** surface — VS Code, the Desktop app's Code tab, a terminal, `claude -p`, the Agent SDK (`mcpServers` option) — with the plugin installed once at user scope; the control channel reaches owned sessions only.

### 5.5 Browser compatibility

| Browser | Status | Notes |
|---|---|---|
| Chrome, Edge, Brave, Chromium (desktop) | Phase 1 | One MV3 build; each browser pairs separately; Edge Add-ons store later |
| Firefox | Phase 5 | MV3 with differences: `browser.*` promises, no `chrome.debugger` (trusted input, screenshots and console need `browser.tabs.captureVisibleTab` and content-script fallbacks), event pages instead of service workers |
| Safari | Future | Needs an Xcode-wrapped app extension, Apple distribution; no debugger API |
| Mobile browsers | Not today | No extension APIs with these capabilities |

---

## 6. Claude Code integration: roles, delegation, sessions

### 6.1 The shape of a team

A **workflow template** (JSON in `<app data dir>/workflows/*.json`, editable in the UI, exportable to a project as `.mother-claude/team.json`):

```json
{
  "name": "software-delivery",
  "roles": [
    {"id": "ceo", "title": "CEO", "agent": "ceo", "model": "opus", "effort": "high",
     "permissionMode": "default", "worktree": false, "paths": ["docs/**", "*.md"],
     "mcp": ["mother-claude"], "autostart": true},
    {"id": "frontend", "title": "Frontend", "agent": "frontend", "model": "sonnet", "effort": "medium",
     "permissionMode": "acceptEdits", "worktree": true, "paths": ["src/app/**"], "budgetUsd": 5},
    {"id": "backend", "title": "Backend & API", "agent": "backend", "model": "sonnet", "effort": "medium",
     "worktree": true, "paths": ["src-tauri/src/server/**", "src-tauri/src/bridge/**"]},
    {"id": "scout", "title": "Scout", "agent": "scout", "model": "haiku", "effort": "low",
     "permissionMode": "plan", "worktree": false, "readOnly": true}
  ],
  "integration": {"branch": "main", "mergeBy": "ceo", "gate": "npm test && cargo test"}
}
```

Each role maps to an **agent definition** (`.claude/agents/<role>.md`, or an `--agents` JSON written by Mother Claude for the launch) holding the charter prompt, tools, model, effort, memory scope and the role's own subagents for tiered work inside the session (a Sonnet implementer's `tests` subagent on Haiku, its `reviewer` on Opus).

### 6.2 Delegation: the CEO, the board, the workers

1. The user (desktop or phone) starts a workflow on a project. Mother Claude launches the **CEO** as a named owned session: `claude -p --input-format stream-json --output-format stream-json -n "<project>/ceo" --model opus --effort high --agents <roles.json> --settings '{"crossSessionInbound":"accept"}' --add-dir … --mcp-config <mother-claude>`. (Whether `--agent ceo` applies the definition to the main session is the Phase 0 spike; until then the charter goes in `--append-system-prompt`.)
2. The CEO plans and writes **task packets** on the board (`task_create {role, title, goal, acceptance, files, constraints, priority}`) — MCP tools, so the packets are structured and visible to the user.
3. Mother Claude's **scheduler** sees claimable tasks for roles with no live session and starts those role sessions when capacity allows (§9): named, in a worktree if the role says so, with `--max-budget-usd` for API-key accounts, `crossSessionInbound: accept`, and a `SessionStart` hook that injects the current assignment as `additionalContext` (≤10,000 chars) so the worker begins with its packet.
4. Workers `task_claim`, work, `task_update` (progress, questions), and `task_done {summary, changed files, branch}`; they may also `SendMessage` the CEO directly (local socket; the CEO's inbox accepts). The CEO gets a cross-session message from Mother Claude's own session identity ("board: task 12 done") or polls `task_list`.
5. The CEO reviews (itself, or a `reviewer` subagent on a higher tier), merges (`integration.mergeBy`), runs the gate, reports to the user. Mother Claude relays progress to the phone and turns `TaskCompleted`/`Stop`/idle into notifications.
6. When a role has no tasks for N minutes, the scheduler stops its session (the transcript stays; the role resumes later with `--resume`, a fork if another process holds it — the existing rule).

A message from a session never approves anything; a worker's permission prompts are the worker's own and show on the desktop/phone as today; `bypassPermissions` for a worker is a dangerous choice gated like every other.

### 6.3 Why not agent teams, workflows or the SDK as the orchestrator

| Option | Verdict | Why |
|---|---|---|
| Agent teams | No for owned sessions | Interactive only; teammates share the lead's permission mode; no worktrees; not restored by `/resume`; experimental |
| The `Workflow` tool (ultracode) | Yes, *inside* a role session | Fan-out of up to 16 concurrent subagents in one session, works in `-p` with an allow rule; no mid-run input; agents are not sessions |
| Agent SDK sessions from the sidecar | For helpers only | The SDK gives `agents`, `maxBudgetUsd`, `canUseTool`, `hooks`; but SDK sessions are not Claude Code sessions a user can open in VS Code, and teammates are not spawned there either; use it for the classifier and other non-interactive calls |
| Named `-p` sessions + cross-session messaging + board (chosen) | Yes | Every worker is a real session; messaging is local, on by default, every provider; Mother Claude keeps the capacity decisions and the audit trail |

Fallback if cross-session messaging changes under us: Mother Claude already relays text into sessions (`additionalContext` on PostToolUse/Stop, `mc-wake` for idle ones, bridge turns for owned ones). The board stays the source of truth either way; the adapter (`orchestration::transport`) hides the difference.

### 6.4 Hooks and events to add to the plugin

`TeammateIdle`, `TaskCreated`, `TaskCompleted` (for users who run interactive teams), `WorktreeCreate`/`WorktreeRemove`, `PreModelSwitch`/`PostModelSwitch` (the router's measurements), `FileChanged`, `ConfigChange` (the path guard watches for config edits), `Elicitation`/`ElicitationResult`. All observe-only except `PreToolUse` (the path guard, §8) and the existing `PermissionRequest`.

### 6.5 Interactive sessions

Everything above also serves sessions Mother Claude does not own: a VS Code session with the plugin sees the board and the browser through `/mcp`, can be assigned a role (the user picks it in the dashboard; Mother Claude injects the packet on the next `UserPromptSubmit`), and can message the CEO. It just cannot be started or stopped by the scheduler (the hard invariant).

### 6.6 Workflow templates beyond software

| Template | Roles | Tools that matter |
|---|---|---|
| Content production | planner (Opus), scriptwriter (Sonnet), designer (Sonnet + image MCP), producer (Sonnet + Bash: ffmpeg, Remotion), QA (Opus, rubric), publisher (Sonnet + browser) | browser, files, project MCPs |
| Research & analysis | lead (Opus), researchers ×N (Haiku/Sonnet, web + browser), synthesiser (Opus), fact-checker (Sonnet) | web search/fetch, browser, documents |
| Browser operations | operator (Sonnet, browser `act` on allow-listed sites), verifier (Haiku, browser `read`) | browser with approvals |
| Documentation | librarian (Haiku, index), writer (Sonnet), editor (Opus) | files |
| Business routines | scheduler via `/loop` or cloud scheduled tasks; roles as above | browser, email/calendar MCPs the user connects |

A template is data; the orchestrator does not know what "frontend" means.

---

## 7. Model selection and cost

### 7.1 Policy, in order of precedence

1. **User override** on a task or a role (dashboard/phone): absolute.
2. **Escalation state** of the task: after a failed gate or a reviewer's "needs deeper reasoning", the next tier (Haiku → Sonnet → Opus → Fable) and/or higher effort; de-escalate after two clean completions of the same task class.
3. **Budget gate** (§9): if the Opus weekly window or the project budget is past its threshold, Opus-tier tasks queue or run on Sonnet at `high` with a note; Fable only on explicit request.
4. **Task class** from the packet (`plan`, `architecture`, `review`, `implement`, `test`, `fix`, `search`, `summarise`, `classify`, `ops`) → the role template's default `(model, effort)`; the class is set by the CEO when it writes the packet, or by the Haiku classifier when a task arrives unclassified (e.g. from the phone).
5. **Role default**.

Defaults in the first template: planning/architecture/review `opus @ high`; implementation `sonnet @ medium` (Anthropic's recommended starting point for agentic coding on Sonnet 5.5); tests/fixes `sonnet @ medium`; search/logs/classification `haiku @ low`; audits and "it must be right" `opus @ xhigh` or `fable @ high` on request. `--fallback-model` chains per role (`opus,sonnet`) for availability.

### 7.2 What makes it honest: measurement

Per task: model, effort, input/output/cache tokens (already indexed per session and agent), wall time, turns, retries, gate result, reviewer verdict, escalations, user edits after completion. Reports: cost per completed task by class × model × effort; escalation rate; cache hit ratio (a cascade across models loses it). The first month runs **static defaults with escalation**; thresholds change only on data. Expectation to test, not assume: the top model at lower effort may beat a Sonnet→Opus cascade on cost per completed task (Anthropic's guidance), and Haiku 5.5 at $0.10/$0.50 makes "scout" roles nearly free.

### 7.3 Plan accounts vs API accounts

On Pro/Max the lever is the windows: Opus and Sonnet have separate weekly windows, so the router treats "Opus window at 85%" as a routing input, not just a display. On API-key accounts (`ANTHROPIC_API_KEY` in a directory account's environment) the lever is dollars: `--max-budget-usd` per worker session and a project daily cap; the estimate is client-side and may overshoot, so caps are soft and alerts are early.

---

## 8. Shared context, synchronisation, conflicts

- **Shared truth**: the project's `CLAUDE.md` and memory (per project path), role charters, the board (packets, results), artifacts in the repository. Workers get their packet by `additionalContext` at start and by `task_get` later; the CEO gets results by messages and `task_list`.
- **Isolation**: a worker that edits code runs in its own git worktree (`-w`); the scheduler creates/prunes worktrees under `<project>/.claude/worktrees/<role>` (Claude Code's own location) and checks disk space first (a worktree duplicates `node_modules`; the UI warns and offers a shared cache or `pnpm`).
- **Ownership guard**: each role's `paths` globs. The plugin's `PreToolUse` hook already asks Mother Claude to *decide*; for a role session the server answers `permissionDecision: deny` with the reason when `Edit`/`Write`/`MultiEdit`/`NotebookEdit` targets a file outside the role's paths, or a Bash command's write targets do (the existing dangerous-command parser extends to this). Shared files with one owner (lockfiles, `package.json`, CI config) belong to the CEO or a `release` role.
- **Integration**: workers never push to the integration branch; the CEO (or `integration.mergeBy`) merges after the gate; conflicts go back as tasks with the diff attached.
- **Memory across worktrees**: Claude Code keys auto-memory by project path, so a worktree's memory is separate (`CLAUDE_CODE_HOST_WORKTREE*` variables exist in the CLI — semantics unverified, **spike**). Mitigation: the CEO's session runs in the main checkout, and roles write durable knowledge to `docs/` through tasks, not to memory.
- **Simultaneous edits to one file** cannot happen under the guard; the remaining race is two roles touching the same *behaviour* — caught by the gate, surfaced as a task.
- **Context size**: packets carry acceptance criteria and file lists, not transcripts; a worker that needs history asks the CEO (`SendMessage`) rather than reading other sessions' transcripts (which the MCP server exposes read-only, `session_summary`, for the CEO only).

---

## 9. Resource management and parallel execution

Inputs the admission controller reads every few seconds:

| Signal | Source (exists?) | Use |
|---|---|---|
| Free RAM, load average | `sysinfo` crate (new) | `hardware_cap = clamp(floor((free_RAM − 2 GB) / 600 MB), 1, 16)`; load > cores → no new builds-heavy roles |
| Live session RSS | `ps`/`sysinfo` per pid (pids known) | Actual, not assumed, per-session cost |
| Plan windows | `usage.rs` (exists): `five_hour`, `seven_day`, `seven_day_opus`, `seven_day_sonnet` | `window_cap`: at ≥85% of the 5-hour window stop admitting; at ≥95% queue everything but the CEO; per-family weekly windows feed the router |
| Rate-limit events | owned streams' `rate_limit_event` (exists) | Immediate backpressure |
| Budgets | new: per project/day, per task, `--max-budget-usd` per worker on API accounts | Hard stop for workers, soft alert for the CEO |
| Repository | worktree disk use, open worktrees, gate running | `repo_cap` (default 4 editing roles) |
| User cap | setting, default 3 concurrent workers + CEO | The user's own ceiling; the request's "no more than three" is the default, not a constant |
| Priority | task priority P0–P3; CEO > workers; user-assigned tasks first | Order of the queue |

Decisions: `admit = min(hardware_cap, window_cap, repo_cap, user_cap)`; start the highest-priority claimable role; never preempt mid-turn — "pause" means not sending the next task and letting the turn end (the existing between-turns activity logic knows when); `interrupt` only on user action; stop idle roles after `idleStopMinutes`; the phone shows the queue and why each item waits ("Opus window 91%", "RAM headroom 1.1 GB").

Things deliberately not done: GPU accounting (nothing uses it); CPU throttling of sessions (the work is remote); killing sessions to free RAM (a stopped session loses its in-flight turn; queueing is enough).

---

## 10. Security, permissions, authentication, remote access

Threat model additions: a web page is untrusted input that reaches the model (prompt injection through page text); a session can be asked, by such a page, to act on *another* site; the extension holds the user's signed-in browser; several sessions and devices share one board.

Controls (all server-side, the extension and the phone are clients):

- **Principals**: `Extension` (paired token `mcx_…`, loopback peer, `Origin: chrome-extension://<id>`), sessions calling `/mcp` (script/mcp token plus session identity), devices as today. `ctx.may_dangerous()` unchanged: desktop, a device with the grant for its ingress, a local caller on the bridge port.
- **Site modes** per origin: `off` (default for unknown sites), `read`, `act`; "all sites read" and "all sites act" are explicit user choices with a warning; sensitive-site list (banking, payment, identity providers) defaults to `off`.
- **Dangerous browser actions** → `crate::pending` cards with the existing first-answer-wins semantics, answerable on the desktop or a granted device: navigation to or submission on a sensitive site; a form with password/card/SSN-like fields; `browser_eval`; file upload/download; clipboard; more than N actions per minute on one site (runaway loop); any action while the user pressed Stop in the last minute. A standing grant ("always allow on `example.com`") is a dangerous right itself, audited, revocable.
- **Secrets never flow through the model**: password fields are redacted in snapshots; `browser_type` into a password field needs the per-site grant and the value comes from the user's secure-input card, not from the tool input; cookies/storage are never exposed as tools.
- **Audit**: every browser action, approval, grant change, board mutation and model switch is an audit line (existing audit), with the session, principal and origin; the phone shows a per-session browser activity log.
- **Prompt-injection posture**: the page cannot change site modes, grants or the board (those are tools gated by principal, not by the model's request); the extension's side panel is the only place the user's own clicks count; the model's requests are always "asks".
- **Transport**: the extension channel is loopback only; LAN/tunnel exposure of `/ws/browser` is refused by the listener's `Ingress`; `/mcp` is reachable on loopback and, like `/v1`, on LAN only with a token.
- **Teams**: a worker's permission mode is set by the template; `bypassPermissions` for any role is a dangerous action; messages between sessions never approve; the ownership guard denies writes outside a role's paths; `ConfigChange` events are watched (a session editing `.claude/` or `.mcp.json` is already classified dangerous).
- **Remote**: unchanged pairing/TLS/tunnel model; new frames (team, board, browser activity) carry no page contents beyond titles and URLs unless the user opens the activity view; push notifications carry task titles only; budget alerts are notifications.
- **Data retention**: snapshots and screenshots are transient (held for the request, then dropped); the board and audit persist in the app data dir (0600 files as today); nothing is written under any account's config directory (the invariant).
- **Web Store**: the extension requests `debugger` as optional and justifies it in the listing (trusted input, screenshots); no remote code; a privacy policy stating local-only transport.

---

## 11. macOS and Windows

| Area | macOS | Windows |
|---|---|---|
| Rust core, server, scheduler, board | Same | Same (the 0.7.0 path work and process liveness fix already landed) |
| Secrets | Keychain (done) | **Add Credential Manager** (`keyring` crate or `CredWriteW`); keep 0600 files as the fallback |
| Owned session spawn | `claude` | `claude.exe`/`claude.cmd` resolution via `MOTHER_CLAUDE_CLI` (verify in Phase 0 on the Windows desktop) |
| Plugin scripts | `/bin/sh` (done) | Hooks run under the shell Claude Code uses on Windows (Git Bash); the HTTP MCP entry avoids a script entirely; the stdio shim, if needed, ships as the same script form the hooks use |
| Extension | Chrome/Edge/Brave | Same code; Edge is common on Windows — pair it too |
| Native messaging (if ever) | Manifest file | `HKCU\Software\Google\Chrome\NativeMessagingHosts` and the Edge key |
| Claude in Chrome | Supported | Supported natively; **not under WSL** |
| Worktrees | `git worktree` | Same; long paths and case-insensitivity already handled |
| Process memory | `ps` | `sysinfo` works on both; drop `ps` for it |
| Release | Mac leg (done) | Windows leg (done) |
| Testing | Everything runs here | Rust/TS unit tests run in CI-less local runs on the Windows desktop; the Windows-only paths get an integration checklist (§15) because nothing Windows-specific runs on the Mac |

Linux is not a target, and nothing above prevents it.

---

## 12. Limitations, risks, challenges

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Cross-session messaging semantics change (it is new) | Medium | Delegation breaks | Transport adapter with the existing relay as fallback; board is the truth |
| `crossSessionInbound: accept` via `--settings` for `-p` workers behaves differently than documented | Low | Workers ignore the CEO | Phase 0 spike; fallback relay |
| `--agent` does not apply a definition to the main session | Medium | Role charters via `--append-system-prompt` instead (works today) | Spike; no design dependency |
| Claude in Chrome from owned `-p` sessions is impossible | Medium | Only affects the "zero-build" complement | Our extension is the primary path |
| Chrome Web Store review of `debugger`/`<all_urls>` | Medium | Delayed public listing | Optional permissions; unpacked/side-loaded install from the desktop for early users; Edge Add-ons as a second channel |
| MV3 service worker killed despite keepalive (sleep, browser restart) | Certain, occasionally | Dropped commands | Idempotent commands, lease recovery, reconnect; the server fails fast and the model retries |
| Sites with bot defenses, CAPTCHAs, 2FA | Certain | Automation stops | Pause-for-user cards ("finish the login, then Continue"), never automate CAPTCHAs |
| Prompt injection from pages | Certain | Wrong actions | Site modes, approvals, ownership of grants by principal, audit |
| Token/cost blow-ups from parallel roles | High without caps | Windows exhausted, money spent | Admission control, budgets, alerts, idle stop; measurement before tuning |
| Worktree disk use | High on JS projects | Disk full | Space check, warnings, pruning, shared caches |
| Memory split across worktrees | Certain | Workers forget | Durable knowledge via `docs/` tasks; CEO in the main checkout; spike `CLAUDE_CODE_HOST_WORKTREE` |
| Quality of automated model selection | High at first | Wrong tier, retries | Static defaults + escalation + human override; data before automation |
| Firefox/Safari parity | Certain | Fewer capabilities there | Documented capability matrix per browser |
| Undocumented CLI internals used by spikes | Medium | Breakage on CLI updates | Only documented channels in shipped code; internals only behind feature flags with graceful failure |
| The experimental agent-teams feature evolves to support `-p` | Possible | Opportunity, not risk | Adapter can switch transports later |

---

## 13. Roadmap

Dependencies flow downward. Durations assume one developer directing Claude Code sessions, and are for planning, not promises.

| Phase | Scope | Depends on | Duration |
|---|---|---|---|
| **0. Spikes and foundations** | Verify every **spike**; MCP server skeleton at `/mcp`; plugin MCP entry; `sysinfo`; Windows Credential Manager | — | 1–2 weeks |
| **1. Browser extension MVP** | Extension (Chrome/Edge/Brave), `/ws/browser`, pairing, leases, `browser_*` tools, site modes, approvals, side panel, audit | 0 | 4–6 weeks |
| **2. Teams and the board** | Templates, roles, board tools, CEO/worker launch, cross-session delegation, ownership guard, worktrees, new hooks, team UI | 0 | 4–6 weeks (parallel with 1 after week 2) |
| **3. Routing and scheduling** | Policy engine, escalation, measurement, admission control, budgets, reports | 2 | 3–4 weeks |
| **4. Remote management** | Phone: team view, board, assignment, browser approvals and activity, budget alerts, push | 1, 2 | 2–3 weeks |
| **5. Beyond development, more browsers, polish** | Non-dev templates, `browser_record`, Firefox, Web Store/Add-ons listings, Windows polish | 1–4 | 4+ weeks, incremental |

---

## 14. Implementation checklists

### Phase 0 — spikes and foundations

- [ ] **Spike A — Claude in Chrome from an owned session**: launch `claude -p --input-format stream-json --chrome` under a claude.ai-signed-in Default account; observe whether the host receives `get_chrome_dialog`/`chrome_permission_prompt`-style requests and whether `mcp__claude-in-chrome__*` tools appear in `initialize`/`get_settings`. Record the exact subtypes. Outcome decides whether §5's "complement" includes owned sessions.
- [ ] **Spike B — plugin HTTP MCP**: add an `http` server with `${user_config.port}`/`${user_config.token}` headers to the plugin; `claude plugin configure --values-stdin`; confirm the tools appear in a terminal session, a VS Code session and (if available) the Desktop Code tab, on macOS and on the Windows desktop. If headers substitution fails anywhere, build the `mc-mcp` stdio shim and repeat.
- [ ] **Spike C — `-p` workers on cross-session messaging**: two owned sessions, one with `--settings '{"crossSessionInbound":"accept"}'`; `ListAgents` from the other; `SendMessage` by name; `notify_when_idle`; measure latency; confirm nothing transits the network (`lsof`/no outbound). Test the `hold` default class mismatch (`bypassPermissions` ↔ `default`).
- [ ] **Spike D — `--agent <role>` semantics** for a main session: model, tools, prompt, permission mode; compare with `--append-system-prompt` + `--agents`.
- [ ] **Spike E — MV3 prototype**: unpacked extension, WebSocket to a throwaway loopback server, `browser_snapshot` + `browser_click` on a signed-in site (GitHub), keepalive, service worker restart recovery, `chrome.debugger` trusted click on a site that ignores synthetic clicks.
- [ ] **Spike F — `CLAUDE_CODE_HOST_WORKTREE*`** and memory behaviour across worktrees.
- [ ] **Spike G — Windows**: `claude.exe` spawn of owned sessions; Edge native messaging key path (for the record); extension pairing on Windows Chrome and Edge.
- [ ] Decide `rmcp` vs hand-rolled Streamable HTTP after a two-day trial; land `/mcp` with one tool (`mc_ping`) behind a feature flag.
- [ ] Add `sysinfo`; replace `ps`-based memory reads; expose `/api/host` (RAM, load, per-session RSS).
- [ ] Windows Credential Manager store in `secrets.rs`; migration from 0600 files; tests on the Windows desktop.
- [ ] Write the ADRs for decisions 1–5 of §0 into `docs/` (short).

Exit criteria: every spike has a written result; `/mcp` serves a tool to a VS Code session through the plugin on both OSes.

### Phase 1 — browser extension MVP

Extension
- [ ] Repository layout `extension/` (TypeScript, no framework or Preact; esbuild), `npm run ext:build`, lint, unit tests (Vitest), e2e (Playwright launching Chrome with the unpacked extension).
- [ ] Manifest with optional permissions; options page with host/port and pairing; `chrome.storage.local` token.
- [ ] Service worker: socket client, reconnect, 20 s keepalive, command router, timeouts, tab registry, leases, tab groups.
- [ ] Content scripts: snapshot with refs, `read`, `find`, `click`, `type`, `select`, `check`, `scroll`, `wait`, form fill with framework-safe events, element highlight.
- [ ] Debugger-backed `trusted` input, `screenshot`, `console`, `upload`; attach/detach lifecycle; infobar explanation.
- [ ] Side panel: activity, Stop, site mode switch, approval cards, connection state.
- [ ] Redaction of secret fields in snapshots/reads.
- [ ] Packaging for side-load from the desktop (zip + instructions) before any store.

Server
- [ ] `Principal::Extension`, `mcx_` tokens, pairing flow reusing `server::pairing`, revocation in Remote › Devices.
- [ ] `/ws/browser` on loopback only; Origin check; frame protocol; per-command timeouts; leases; queue per browser.
- [ ] `browser::danger` classifier; sensitive-site list; approvals through `crate::pending` with the existing cards and `note_dangerous`; standing grants as dangerous settings.
- [ ] `/mcp` `browser_*` tools (§5.3) with schemas; session identity per call; site-mode enforcement; snapshot size caps; audit lines.
- [ ] Settings UI: site modes, sensitive sites, "always ask" categories, screenshot policy.
- [ ] Docs: `docs/BROWSER.md` (setup, modes, approvals, limits), plugin changes, `CLAUDE.md` section.

Exit criteria (acceptance): a VS Code session, a terminal session and an owned session each complete "open GitHub, find the newest issue in repo X, read it, post a comment draft without submitting" on the user's signed-in Chrome, with the submit blocked pending an approval that is answered from the phone; Stop in the side panel aborts within 1 s; snapshot of a typical page ≤ 4,000 tokens with `interactiveOnly: true`.

### Phase 2 — teams and the board

- [ ] Data model: `workflows/*.json`, `board.json` per project (tasks, packets, results, history), 0600, atomic writes via `persist`.
- [ ] `/mcp` tools: `task_create|list|get|claim|update|done`, `team_roles`, `team_request_role`, `session_summary` (CEO only), `project_info`.
- [ ] Launcher: role → `claude -p` argv builder (name, model, effort, permission mode, `--agents`, `--settings` with `crossSessionInbound`, `--add-dir`, `--worktree`, `--max-budget-usd`, `--mcp-config`); account environment as today; `SessionStart` packet injection.
- [ ] Transport adapter: cross-session messaging (primary), relay fallback; CEO notifications on board events.
- [ ] Ownership guard in the `PreToolUse` decision path; Bash write-target extraction reuse; tests for globs, worktree paths, Windows paths.
- [ ] Worktree management: create/prune, disk check, `.claude/worktrees/<role>`; `WorktreeCreate/Remove` hooks.
- [ ] Hooks plugin: add `TeammateIdle`, `TaskCreated`, `TaskCompleted`, `WorktreeCreate`, `WorktreeRemove`, `PreModelSwitch`, `PostModelSwitch`, `FileChanged`, `ConfigChange`, `Elicitation`, `ElicitationResult` (observe); bump plugin version; reinstall flow.
- [ ] Idle stop and resume (`--resume`, fork rule unchanged).
- [ ] UI: workflow templates editor, team view (roles, sessions, state, queue), board (kanban: todo/claimed/review/done), packet editor, "assign to role".
- [ ] Interactive sessions: "take role X" for a foreign session (packet on next prompt), board visible via MCP.
- [ ] Docs: `docs/TEAMS.md`; `CLAUDE.md` section (the invariant restated: only owned sessions are scheduled).

Exit criteria: on this repository, template `software-delivery` with CEO + frontend + backend + scout completes a three-task feature (one per role) end to end: packets written by the CEO, workers in worktrees, guard denies one out-of-scope edit (verified in the audit), CEO merges after `npm test && cargo test`, the phone shows every state change.

### Phase 3 — routing and scheduling

- [ ] Router: policy table, task classes, escalation ladder, de-escalation, budget gate, `--fallback-model` per role, live `set_model`/effort for running roles between turns.
- [ ] Haiku classifier via the sidecar (Agent SDK or a direct Messages call with `claude-haiku-5-5`), only for unclassified tasks; its cost recorded.
- [ ] Measurement records per task; reports (cost per completed task by class × model × effort; escalation rate; cache hit ratio); export CSV.
- [ ] Scheduler: `sysinfo` headroom, plan windows per family, rate-limit backpressure, repo cap, user cap, priorities, idle stop, queue reasons.
- [ ] Budgets: per project/day, per task, per worker; alerts (desktop notice, push).
- [ ] UI: queue with reasons, budgets, reports; phone: read-only reports + alerts.
- [ ] Docs: `docs/ROUTING.md` with the policy and what the numbers mean.

Exit criteria: a synthetic load of 12 tasks across 4 roles never exceeds the user cap or 85% of the 5-hour window in a simulation driven by the fixture fake CLI (`tests/fixtures/fake-claude.py`) with injected `rate_limit_event`s; the report shows cost per completed task per model; an escalation is triggered by two failed gates and recorded.

### Phase 4 — remote management

- [ ] Phone: team view, board, assign/reprioritise, packet editing (ManageData-class routes), browser approvals and activity, budget alerts, queue reasons.
- [ ] Push: task done/failed/blocked, approval needed, budget threshold, role idle-stopped.
- [ ] Grants: new dangerous classes (browser `act` on sensitive sites, `browser_eval`, standing site grants, `bypassPermissions` roles) under the existing per-device LAN/internet grants.
- [ ] Audit viewer on the phone (read-only).
- [ ] Docs: `docs/REMOTE.md` additions.

Exit criteria: start a workflow on the Mac, leave, and from the phone: see progress, answer a browser approval, re-prioritise a task, receive the "done" push; every action audited with the device's name on the desktop.

### Phase 5 — beyond development, browsers, polish

- [ ] Templates: content production, research, browser operations, documentation; example projects.
- [ ] `browser_record` → macro → skill file in the project.
- [ ] Firefox port with the capability matrix; Edge Add-ons and Chrome Web Store listings.
- [ ] Windows polish: Credential Manager everywhere, Edge pairing by default, native messaging discovery host if pairing friction is reported.
- [ ] Cross-machine teams through Mother Claude's own relay (Mac ↔ Windows) — design only unless requested.

---

## 15. Testing, validation, benchmarks, acceptance

| Layer | How | Gate |
|---|---|---|
| Rust (server, orchestration, router, scheduler) | Unit + integration tests with the fixture home and the fake CLI; property tests for the ownership guard and path globs on both path forms | `cargo test`, clippy `-D warnings`, `check-windows.sh` |
| Extension | Vitest for snapshot/ref logic; Playwright e2e with the unpacked extension against local fixture pages (forms, SPAs, dialogs, shadow DOM, iframes) | `npm run ext:test` |
| MCP | Schema tests; MCP Inspector session; a Claude Code `-p` smoke test with `--mcp-config` calling each tool against fixtures | nightly on the Mac |
| Cross-session delegation | Two fake-CLI sessions exchanging board events; real-CLI smoke test behind `#[ignore]` (spends tokens) | manual before release |
| Security | Origin/token/ingress matrix for `/ws/browser` and `/mcp`; classifier corpus (sensitive forms, eval, uploads); redaction corpus; audit completeness | `cargo test` |
| Scheduler | Simulation harness: synthetic RAM/window/rate-limit signals, assertions on admissions and preemption-at-turn-boundary | `cargo test` |
| Windows | The same suites on the Windows desktop before each release; the Windows checklist (spawn, Credential Manager, Edge pairing, worktrees) | release gate |
| Benchmarks (tracked per release) | Snapshot tokens per fixture page (target ≤ 4k interactive-only, ≤ 12k full); action round trip (target p50 < 150 ms, p95 < 500 ms on loopback); session RSS; scheduler decision latency; cost per completed task by class | `docs/BENCHMARKS.md` |

Acceptance criteria per phase are in §14; the release gate adds: no new route without a principal test; no dangerous action without a pending-store path; every new file under `~/.claude` is a user action (the invariant).

---

## 16. Future enhancements

- **Managed Agents for cloud workers**: Anthropic-hosted sessions with sandboxes could run roles that need no local files (research, drafting), scheduled deployments included; Mother Claude would show them like sessions. Depends on account type.
- **Remote Control interop**: show Remote Control sessions, hand a role to claude.ai/code.
- **Cross-machine teams** over Mother Claude's own channel (the backup pairing of machines is a start).
- **Evaluation harness** for routing: labelled task sets, periodic re-measurement when models change.
- **Recorded macros as skills**, shared through the plugin marketplace.
- **Safari** via an app extension if demand appears.
- **Shared board across users** (a team of people, not just sessions) — out of scope until the single-user design is proven.

---

## Appendix A — verified facts and sources

Local verification (2026-11-06): Claude Code 2.1.318 `--help`; the CLI binary's strings (`com.anthropic.claude_code_browser_extension`, `NativeMessagingHostsDirs`, `chrome-extension://fcoeoabgfenejglbffodgkkbkcdhcgfn`, `mcp__claude-in-chrome__*` tool names, `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1`, `teammateMode` values, `TeammateIdle`/`TaskCompleted`, the `chrome_*` host request kinds, "Claude in Chrome is not wired into [bypass permissions]", WSL unsupported); `claude mcp add --help`, `claude plugin --help`, `claude agents --help`, `claude remote-control --help`, `claude daemon --help`; `~/.claude/teams/*/config.json`; the bundled Agent SDK 0.3.304 type definitions; this repository (`CLAUDE.md`, `claude/control.rs`, `claude/owned.rs`, `server/routes/mod.rs`, `claude/usage.rs`, `claude/hooks.rs`, the plugin assets, `claude/secrets.rs`, `sidecar/`); Chrome 156 installed; three live sessions' RSS; 24 GB / 12 cores.

Official documentation (fetched by the verification agents on 2026-11-06):

- Claude in Chrome: https://code.claude.com/docs/en/chrome.md · CLI reference: https://code.claude.com/docs/en/cli-reference.md · VS Code: https://code.claude.com/docs/en/vs-code.md · Desktop: https://code.claude.com/docs/en/desktop.md · Help Center: https://support.claude.com/en/articles/12012173-getting-started-with-claude-in-chrome
- MCP and plugins: https://code.claude.com/docs/en/mcp.md · https://code.claude.com/docs/en/plugins/components.md · https://code.claude.com/docs/en/plugins/manifest-reference.md · SDK MCP: https://code.claude.com/docs/en/agent-sdk/mcp.md
- Subagents: https://code.claude.com/docs/en/sub-agents.md · SDK subagents: https://code.claude.com/docs/en/agent-sdk/subagents.md
- Agent teams: https://code.claude.com/docs/en/agent-teams.md · Cross-session messaging: https://code.claude.com/docs/en/cross-session-messaging.md · Tools: https://code.claude.com/docs/en/tools-reference.md · Agents: https://code.claude.com/docs/en/agents.md
- Workflows: https://code.claude.com/docs/en/workflows.md · Model config: https://code.claude.com/docs/en/model-config.md · Settings: https://code.claude.com/docs/en/settings-reference.md
- Hooks: https://code.claude.com/docs/en/hooks.md · Scheduled tasks: https://code.claude.com/docs/en/scheduled-tasks.md · Cloud scheduled tasks: https://code.claude.com/docs/en/routines.md · Desktop scheduled tasks: https://code.claude.com/docs/en/desktop-scheduled-tasks.md
- Remote Control: https://code.claude.com/docs/en/remote-control.md · Claude Code on the web: https://code.claude.com/docs/en/claude-code-on-the-web.md
- Agent SDK: https://code.claude.com/docs/en/agent-sdk/typescript.md · https://code.claude.com/docs/en/agent-sdk/python.md
- Chrome platform: remote debugging change in Chrome 136 — https://developer.chrome.com/blog/remote-debugging-port · MV3 WebSockets and the Chrome 116 idle-timer change — https://developer.chrome.com/docs/extensions/mv3/tut_websockets and https://developer.chrome.com/blog/chrome-116-beta-whats-new-for-extensions
- Model prices: Anthropic first-party rates as cached in the `claude-api` reference (2026-10-06).

## Appendix B — unverified (spikes) and not-today, in one list

Spikes: Claude in Chrome from `claude -p`/SDK hosts; plugin HTTP MCP header substitution in every surface and on Windows; `--agent` on a main session; `crossSessionInbound: accept` for `-p` workers end to end; `CLAUDE_CODE_HOST_WORKTREE*` semantics; Web Store review outcome for optional `debugger`; Firefox parity; Desktop app Code tab honouring user-scope plugins and `/chrome`; Chrome 146's `chrome://inspect` auto-connect as an alternative to an extension (one unverified community report).

Not today: agent teams from `-p`/SDK sessions; Claude in Chrome for token accounts, API keys, Bedrock/Vertex/Foundry, `bypassPermissions` sessions, WSL; CDP into the default Chrome profile; a Claude Code switch that picks a model by task complexity; cross-machine session messaging without Remote Control and a claude.ai sign-in; mobile browsers.
