# Architecture

Authoritative invariants also live in [CLAUDE.md](CLAUDE.md).

## Three layers

```
 ┌──────────────────────────┐      HTTP/WS (token)      ┌───────────────────┐
 │ Angular SPA (src/)        │ ───────────────────────▶ │ axum server       │
 │  desktop webview + phone  │ ◀─────────────────────── │ (src-tauri/server)│
 └──────────────────────────┘     one shared path        └─────────┬─────────┘
                                                                    │
                                                          broadcast bus
                                                                    │
                                                        ┌───────────▼───────────┐
                                                        │ claude/ adapter        │
                                                        │  ~/.claude + claude CLI │
                                                        └────────────────────────┘
```

1. **Rust core** (`src-tauri/src/claude/`) — the *only* code that knows the
   `~/.claude` layout/formats. Modules: `home` (paths + encoded-cwd), `schema`
   (tolerant serde), `transcript` (byte-offset tailer), `watcher` (debounced
   notify), `registry` (merge `agents --json` + transcripts + `state.json` →
   `Session`), `git` (libgit2 diffs/log/worktrees), `control` (owned-session
   spawn + lifecycle), `experimental` (PTY/CCR, feature-gated).
2. **Embedded axum server** (`src-tauri/src/server/`) — REST + WebSocket on the
   Tokio runtime. `http` (handlers), `ws` (fan-out), `monitor` (refresh loop),
   `auth` (token + dangerous gate + pairing), `tls` (rcgen/rustls). **All**
   dashboard data flows through here so desktop and mobile share one client.
3. **Angular SPA** (`src/`) — standalone components + signals. `core/`
   (ConfigService, ApiService=fetch, RealtimeService=WS) and `pages/` (sessions,
   session-detail, services, settings).

## Data flow

`~/.claude` (+ `claude` CLI) → `claude/` adapter → `tokio::sync::broadcast` bus →
axum WS/REST → webview & LAN clients. The `monitor` task refreshes the registry
on a 3s interval **and** on debounced filesystem changes, broadcasting a
`sessions` snapshot and live `transcript` deltas (per-session tailers started at
EOF, so history stays a REST concern).

### Why a `Session`'s state is *derived*

`claude agents --json` (2.1.185) reports `{pid, cwd, kind, startedAt, sessionId}`
with **no** state field. `registry::build_registry` derives state: explicit
`jobs/<id>/state.json` > live `pending` > running + recent-activity → working,
else idle/completed. Surface comes from the transcript `entrypoint`.

## Networking model

`serve()` always binds plain **HTTP on `127.0.0.1:<port>`** (used by the desktop
webview, foreign-session hooks, and the local sidecar — no self-signed-cert
friction). For a non-loopback config it *additionally* binds **TLS on each
detected LAN IP** at the same port for phones (bound to specific IPs so they
don't collide with the loopback bind).

## Owned vs foreign control

- **Owned** (spawned by us, id chosen via `--session-id`, tracked in `owned`):
  full control. Path B = headless `claude -p` stream-json subprocess (default).
  Path A = Node Agent SDK sidecar with `canUseTool` + `ask_user` (opt-in).
- **Foreign**: monitor + lifecycle only (`claude stop|respawn|rm`). Live
  injection is experimental-only and off by default.

## Permission bridge

`POST /api/sessions/:id/permission-request` (sidecar-facing) registers a oneshot
resolver, sets `pending`, broadcasts, and blocks. The dashboard's
`POST .../permission|/answer` resolves it. Dangerous approvals are gated to the
local desktop (`auth::dangerous_blocked`).
