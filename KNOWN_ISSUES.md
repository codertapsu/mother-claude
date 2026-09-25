# Known Issues & Caveats

Mother Claude leans on **undocumented, research-preview internals** of Claude
Code. Everything here is contained behind the `src-tauri/src/claude/` adapter so
a single module absorbs version churn.

## Environment this was built against

- Claude Code `2.1.185` originally; re-verified against `2.1.276` on macOS
  (Apple M4 Pro, `aarch64-apple-darwin`). On `2.1.276`, `agents --json` also
  returns `name` and `status` (`idle`/`busy`).
- Node v24.15.0, Rust 1.96.0, git 2.50.1, Xcode CLT present.

## Agent SDK / CLI gotchas found while building the HTTP bridge

- **Exporting `CLAUDE_CONFIG_DIR` breaks authentication**, even when it points at
  the default `~/.claude`. A child spawned with it set answers every turn with
  `Not logged in · Please run /login`, while the identical child without it works.
  Credentials appear to resolve differently once the override is present (the
  CLI also relocates `~/.claude.json` into the config dir in that mode). So
  `bridge/host.rs` never sets it and lets the child inherit whatever the user
  set. Verified against SDK `0.3.186` (bundled CLI `2.1.186`).
- **`resume` for a session id with no transcript fails the whole query** with
  `No conversation found with session ID: …` — it does *not* start a fresh
  session. A brand-new conversation must use `Options.sessionId` to pre-assign
  its id; `resume` is only for a genuine continuation. The Path A sidecar passed
  `resume` unconditionally and so failed on every fresh session until this was
  fixed (`sidecar/src/agent-bridge.ts`, `MC_RESUME`).
- **An unusable account is reported as a *successful* turn.** The SDK emits
  `subtype: "success"` with `is_error: true` and the failure text in `result`.
  Checking `subtype` alone reports a completed turn containing a sentence where
  the answer should be; check `is_error` too.
- **The SDK ships its own CLI.** `@anthropic-ai/claude-agent-sdk@0.3.186` bundles
  Claude Code `2.1.186` (~216 MB) and runs *that*, not whatever `claude` is on
  `PATH` (here, `2.1.276`), unless `Options.pathToClaudeCodeExecutable` says
  otherwise. Two versions are live in one app; expect small behavioural drift.
- **Images must not be tiny.** A 2×2 PNG is rejected upstream with an untyped
  "an image in the conversation could not be processed"; 200×200 works. The
  bridge validates dimensions itself so the caller gets a typed error.

## Schema / CLI differences from the implementation brief

- **`claude agents --json` has no `state` or `waitingFor` field** (2.1.185) and
  spawning it on every refresh is slow, so the live roster is no longer built
  from it: `~/.claude/sessions/<pid>.json` pidfiles (never the `*.key` files next
  to them) give pid, status (`busy` / `idle` / `waiting` + `waitingFor`),
  entrypoint, tmux target and job id; liveness is `kill(pid, 0)` plus the
  process start time. Activity is derived from those, the owned control
  stream, hook events and the transcript tail.
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

## Live-session synchronization (surfaced in the UI)

- **VS Code (and other live editor) sessions are not synced in real time.** The
  dashboard reads the append-only transcript files Claude writes under
  `~/.claude/projects`. A live VS Code session may buffer and not flush its newest
  messages until it goes idle or is closed, so the app can lag behind the editor.
  To pick up the very latest before taking a session over, **close and reopen
  VS Code (or its Claude panel)** so the transcript is flushed; "Continue here"
  then resumes (`claude --resume <id>`) from that saved point.
- The desktop app is a **local bridge, not a cloud**: the embedded server and any
  owned/resumed `claude` processes run on the laptop, which must stay awake.
- After "Continue here", driving the session from both the original editor and the
  app can fork the conversation; use one driver at a time.

These are presented to end users in-app under **Settings → Good to know** and on
each foreign session's detail view.

## Fundamental limitations (by design)

- **No private channel into foreign sessions.** The cc-daemon control socket
  (`~/.claude/daemon/control.key`, `daemon/dispatch`, `/tmp/cc-socks/*.sock`) is
  authenticated with a rotating key over an undocumented protocol; we never speak
  it. Foreign sessions are controlled only through documented surfaces: the
  optional foreign-control **plugin** (hooks), **tmux** panes, and the
  experimental PTY `claude attach` for background jobs. Each session's `control`
  capabilities say which apply; without any, it is watch-only with "Continue
  here".
- **macOS Full Disk Access (TCC):** the packaged GUI app needs FDA granted in
  System Settings to read `~/.claude/projects` and friends, even though the dev
  terminal can read them as the owning user. First-run check surfaces this.
- **Hook caveats.** A held `PermissionRequest` hook runs *in parallel* with the
  session's own dialog (VS Code, terminal): whoever answers first wins, and the
  phone's card is withdrawn when the tool runs or the turn moves on (verified on
  2.1.282). Relayed messages arrive as hook `additionalContext` at the next tool
  call or turn end (or wake an idle session through `asyncRewake`), so they do
  not appear in the session's own window — only Claude's reply does. Sessions
  started before the plugin was installed need `/reload-plugins` or a restart.
  Headless `claude -p` runs are never held or woken.

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
  the server always serves plain HTTP on `127.0.0.1:<port>` (desktop webview,
  hook plugin, local scripts) and TLS only on private LAN addresses (phones),
  with a certificate from a local CA name-constrained to private ranges. Public
  addresses are never bound; the internet is reached through a tunnel to the
  local-only tunnel port.
- **`claude --bg` does not exist** on 2.1.185 — owned/background sessions are
  spawned via the headless `claude -p` path; foreign lifecycle uses
  `claude stop|respawn|rm`.
- **Owned sessions use the stream-json control protocol** (Path B): the
  installed `claude -p` with `--permission-prompt-tool stdio` sends every
  approval, question, plan and elicitation to Mother Claude as a control
  request, the way VS Code drives it. The Agent SDK sidecar (Path A) remains as
  a legacy option behind `MOTHER_CLAUDE_SIDECAR=1` (text only).
- **Hook token**: the foreign-control plugin reads a per-launch token from
  `hook.curlrc` (0600) in the app data dir; nothing is written to
  `~/.claude/settings.json`. See SECURITY.md.
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
