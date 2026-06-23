# Mother Claude — AI coding guidance

> This file is auto-loaded by Claude Code. Read it before making changes.

## What this is

**Mother Claude** is a Tauri v2 (Rust) + Angular desktop app that monitors and
controls every local Claude Code session/agent/service from one dashboard, and
serves that *same* dashboard over the LAN (default `0.0.0.0:6725`) so it can be
driven from a phone. Three layers:

1. **Rust core** (`src-tauri/src/claude/`) — an adapter over `~/.claude` plus
   session control.
2. **Embedded axum server** (`src-tauri/src/server/`) — REST + WebSocket on the
   Tokio runtime. **All** dashboard data flows through here.
3. **Angular SPA** (`src/`) — desktop webview *and* phone browser, one code path.

## The one hard invariant: owned vs foreign sessions

Full two-way control (inject answers / approve permission prompts live) is only
reliable for sessions **Mother Claude launched itself** ("owned"). There is no
supported way to inject input into a *foreign* live session (started in another
terminal/editor and held by the Claude Code supervisor — its control socket uses
a rotating key and an undocumented protocol).

- **Owned sessions** → full control via headless `claude -p --output-format
  stream-json --input-format stream-json` (Path B) or the Agent SDK sidecar
  (Path A, `canUseTool` + `ask_user` MCP tool).
- **Foreign sessions** → monitoring (read transcripts + receive hook events) and
  lifecycle (`stop`/`respawn`/`rm`). Live control is also enabled **by default**
  via PTY-driving `claude attach` (the `experimental` Cargo feature is now
  `default`), but it is **best-effort/unstable** — the UI labels foreign control
  as experimental. Runtime opt-out: `MOTHER_CLAUDE_FOREIGN_INJECTION=0`.
- Only PTY-`attach` foreign injection is implemented; the "CCR v1" transport is a
  status probe only. Never speak the cc-daemon control socket directly. Owned
  sessions remain the path to *reliable* control.

**Design the product around owning sessions.** Do not block on foreign injection.

## Data sources (adapter contains all version drift)

Everything lives under `CLAUDE_CONFIG_DIR` or `~/.claude`. Keep all reads in
`src-tauri/src/claude/` so schema churn is contained to one place. Make every
deserialized field tolerant (`#[serde(default)]`, capture unknowns) — these are
undocumented research-preview internals.

| Source | Path / command | Gives |
|---|---|---|
| Session list | `claude agents --json [--all] [--cwd <p>]` | `pid, cwd, kind, startedAt, sessionId` |
| Per-session state | `~/.claude/jobs/<id>/state.json` | live state (may be absent) |
| Running roster | `~/.claude/daemon/roster.json` | `{proto, supervisorPid, updatedAt, workers}` |
| Daemon health | `claude daemon status` | pid, version, uptime |
| **Transcripts (all surfaces)** | `~/.claude/projects/<encoded-cwd>/<id>.jsonl` | append-only NDJSON, tailable |
| Command history | `~/.claude/history.jsonl` | command usage |
| MCP inventory | `~/.claude.json`, project `.mcp.json` | configured MCP servers |
| Daemon log | `~/.claude/daemon.log` | supervisor logs |
| Worktrees | `<project>/.claude/worktrees/<name>` | per-session isolated edits |
| Live events | **Hooks** (`http` handler → our server) | PreToolUse/PostToolUse/Notification/Stop |

Key facts: `<encoded-cwd>` = absolute path with every non-alphanumeric char → `-`.
The *same* `projects` store is written by CLI, VS Code/JetBrains, and Claude
Desktop — reading it captures all surfaces for free. Transcript `tool_use`/
`tool_result` are content blocks inside `message.content`, **not** top-level event
types. Token usage is in `message.usage`. Tail with a byte offset, split on `\n`,
buffer the trailing partial line. Transcripts are pruned after 30 days.

This Claude Code (`2.1.185`) returns **no `state` field** from `agents --json` —
state is *derived* from transcript activity + `state.json` + hook events.

## Architecture rules (enforced)

- **All data for both desktop and mobile flows through the axum server, never
  Tauri `invoke`.** Tauri `invoke` is allowed only for desktop-only OS concerns
  (e.g. opening System Settings for Full Disk Access). Dashboard data: HTTP/WS.
- Single broadcast bus (`tokio::sync::broadcast`) fans events to webview + all
  WS clients. Backpressure-aware.
- TLS + token auth are **mandatory** whenever the bind address is non-loopback.
  Dangerous approvals (`bypassPermissions`) are local-desktop-only by default.

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
