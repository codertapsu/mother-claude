# Architecture

> Stub — expanded by the `docs` commit. Authoritative invariants live in
> [CLAUDE.md](CLAUDE.md).

## Three layers

1. **Rust core** (`src-tauri/src/claude/`) — adapter over `~/.claude`
   (path/encoded-cwd resolution, tolerant schemas, transcript tailer, fs
   watcher, session registry, git diffs, session control).
2. **Embedded axum server** (`src-tauri/src/server/`) — REST + WebSocket on the
   Tokio runtime, bound `0.0.0.0:6725` by default. **All** dashboard data flows
   through here so desktop (webview) and mobile (browser) share one path.
3. **Angular SPA** (`src/`) — standalone components + signals; one API client.

## Data flow

`~/.claude` (+ `claude` CLI) → adapter → `tokio::sync::broadcast` bus → axum
WS/REST → webview & LAN clients.

## Owned vs foreign

- **Owned** (spawned by us): full two-way control.
- **Foreign**: monitor + lifecycle only; live injection is experimental-only.
