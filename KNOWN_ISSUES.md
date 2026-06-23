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

## Experimental tier (`--features experimental`, off by default)

- PTY-driving `claude attach <id>` (Ink discards piped `\n`, so a real PTY is
  required) and the reverse-engineered "CCR v1" transport are **unsanctioned and
  unstable across versions**. The "no authentication" claim for CCR v1 is
  unverified — verify independently before relying on it. Gated behind a UI
  "uses unstable, unsupported internals" confirmation.

## Status of stages

See git history for what each commit delivered. Any stage that could not be
completed in full is implemented to a working subset with the remainder noted
here as it is discovered during the build.
