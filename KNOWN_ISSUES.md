# Known Issues & Caveats

Mother Claude leans on **undocumented, research-preview internals** of Claude
Code. Everything here is contained behind the `src-tauri/src/claude/` adapter so
a single module absorbs version churn.

## Environment this was built against

- Claude Code `2.1.185` on macOS (Apple M4 Pro, `aarch64-apple-darwin`).
- Node v24.15.0, Rust 1.96.0, git 2.50.1, Xcode CLT present.

## Schema / CLI differences from the implementation brief

- **`claude agents --json` has no `state` or `waitingFor` field.** On this
  version it returns objects of the form
  `{pid, cwd, kind, startedAt, sessionId}`. Session *state*
  (working / needs-input / idle / completed / failed / stopped) is therefore
  **derived** by the registry from transcript activity (recency + last event
  type + `stop_reason`), `~/.claude/jobs/<id>/state.json` when present, and live
  hook events — not read directly. `--all` adds completed sessions.
- **`claude --bg "<task>"` does not exist** on 2.1.185. Background/owned sessions
  are spawned through the headless path (`claude -p --output-format stream-json
  --input-format stream-json`). Lifecycle subcommands that *do* exist and are
  used: `claude stop <id>`, `claude respawn <id>|--all`, `claude rm <id>`,
  `claude attach <id>`.
- `~/.claude/jobs/` may contain only `pins.json` with no per-session
  `state.json` until background jobs are running, so `state.json` parsing must
  tolerate total absence.
- `~/.claude/daemon/roster.json` shape observed: `{proto, supervisorPid,
  updatedAt, workers}` where `workers` is a **map**, not an array.
- Transcript `tool_use` / `tool_result` are **content blocks inside
  `message.content`**, not top-level NDJSON event types. Observed top-level
  `type`s also include `last-prompt`, `ai-title`, `attachment`,
  `queue-operation`, `permission-mode`, `mode` beyond the documented set.

## Fundamental limitations (by design)

- **Foreign live-session answer injection is unavailable.** The cc-daemon control
  socket (`~/.claude/daemon/control.key`, `daemon/dispatch`) is authenticated
  with a rotating key over an undocumented protocol; auto-update rotates the key.
  We never speak it. Foreign sessions get monitoring + lifecycle only. The UI
  disables injection controls and labels these sessions accordingly.
- **macOS Full Disk Access (TCC):** the packaged GUI app needs FDA granted in
  System Settings to read `~/.claude/projects` and friends, even though the dev
  terminal can read them as the owning user. First-run check surfaces this.
- **Hook caveats** (documented Claude Code bugs we route around): a `PreToolUse`
  hook returning `allow` does **not** reliably suppress the native interactive
  prompt, and `PermissionRequest` hooks do not fire in `-p` mode. We use hooks
  for *events* and *deny* gating, not as a remote-approve bus for foreign TUI
  sessions.

## Foreign-session injection (`experimental` tier — now ON by default)

The brief shipped PTY/CCR foreign control off-by-default; it is now **enabled by
default** at the project owner's request (`default = ["experimental"]`, runtime
opt-out `MOTHER_CLAUDE_FOREIGN_INJECTION=0`, or `--no-default-features`).

- PTY-driving `claude attach <id>` (Ink discards piped `\n`, so a real PTY is
  required) is **unsanctioned and unstable across versions**. It types into the
  session's TUI, so it is **best-effort**: free-text instructions and question
  answers are reliable; permission-prompt selection (allow≈"1", deny≈"2") is a
  guess that depends on the prompt's option layout, and `claude attach` only
  works for sessions the daemon can attach to (background sessions; attaching to
  some live interactive surfaces may fail).
- The reverse-engineered "CCR v1" transport remains **not implemented** (status
  probe only); its "no authentication" claim is unverified.
- Foreign injection only targets sessions that are present and **running** in the
  registry; unknown ids are rejected (never spawns a stray `claude attach`).

## Build-time decisions & deviations from the brief

These are intentional choices made while implementing on this machine; each keeps
the product working and the commits green.

- **Loopback HTTP + per-LAN-IP TLS** (instead of a single `0.0.0.0` TLS bind):
  a self-signed cert can't be verified by the Tauri webview or by Claude Code's
  hook HTTP client, so the server always serves plain HTTP on `127.0.0.1:<port>`
  (desktop webview, hooks, local sidecar) and TLS on the LAN IPs (phones). This
  is what makes both the desktop dashboard and foreign-hook ingestion work
  out of the box.
- **`claude --bg` does not exist** on 2.1.185 — owned/background sessions are
  spawned via the headless `claude -p` path; foreign lifecycle uses
  `claude stop|respawn|rm`.
- **Path B headless permission prompts**: in `-p` mode, permission prompts and
  `PermissionRequest` hooks do not reliably surface, so remote *approval* of
  owned sessions is best done via the **Path A sidecar** (canUseTool/ask_user),
  which is optional and not part of the green gate (it needs
  `@anthropic-ai/claude-agent-sdk` installed and runtime auth). The Rust
  permission *bridge* it talks to is fully implemented and tested.
- **Hook token**: installing hooks embeds the literal token in
  `~/.claude/settings.json` (foreign sessions don't inherit our env). See
  SECURITY.md.
- **QR is rendered as inline SVG** (qrcode crate) rather than a PNG via the
  `image` crate — crisper on mobile and one fewer dependency.
- **Transcript view** caps rendering to the last ~800 events with auto-scroll
  rather than using CDK virtual scrolling (variable-height items). Fine for the
  live-tail use case; revisit for very long histories.
- **`ng test`** (karma) needs a headless Chrome; it is present on this machine.
  The enforced per-commit Angular gates are `npm run lint` and `npm run build`
  (plus `cargo fmt`/`clippy -D warnings`/`test`).

## Status of stages

All 16 commits in the plan were completed and kept green (cargo
fmt/clippy/test + Angular lint/build). The experimental tier and the Path A
sidecar are the only optional/unsanctioned pieces, both off by default.
