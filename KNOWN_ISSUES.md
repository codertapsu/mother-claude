# Known Issues & Caveats

Mother Claude leans on **undocumented, research-preview internals** of Claude
Code. Everything here is contained behind the `src-tauri/src/claude/` adapter so
a single module absorbs version churn.

## Environment this was built against

- Claude Code `2.1.185` originally; re-verified against `2.1.276`, and the remote
  access and foreign-control work against `2.1.282` (CLI and the VS Code
  extension of the same version) on macOS (Apple M4 Pro, `aarch64-apple-darwin`).
- Node v24, Rust 1.96.0, git 2.50.1, Xcode CLT present.

## Remote access and foreign sessions: current limits

These are the honest edges of "control every session from the phone". The user
guide is [docs/REMOTE.md](docs/REMOTE.md); each session's channel badge shows
which of these apply to it.

### Sessions started elsewhere (the hooks plugin)

- **No model, effort or slash-command changes.** Hooks cannot initiate any of
  them. They work for a session running in **tmux** (typed as `/model`,
  `/effort`, which also change the user's default for new sessions) or after
  **Continue here** forks it into an owned session.
- **The permission mode changes only together with an approval** (the hook
  answers with a `setMode` update, e.g. when approving a plan). Hooks and tmux
  sessions therefore show no mode picker in the session header, and
  `POST /api/sessions/{id}/mode` answers `409 not_supported` for them.
- **Interrupt lands at the next tool boundary.** The hook answers
  `continue: false` at the next `PreToolUse` / `PostToolUse` / `Stop`. A long
  stretch of thinking or writing without tool calls runs on until then; there
  is no mid-generation interrupt (an external SIGINT kills the whole process
  instead, so Mother Claude never sends one).
- **Relayed messages are invisible in the session's own window.** A message sent
  from the phone reaches the model as hook `additionalContext` (while it works)
  or as the wake listener's output (when idle). The VS Code panel or terminal
  shows only Claude's reply, never the message. Mother Claude's own timeline
  shows it as a relayed message.
- **Delivery timing.** A busy session gets the message at its next tool call or
  at the end of the turn. An idle session is woken only if its wake listener
  (`mc-wake`, started when a turn ends) is connected, so a session that has not
  finished a turn since the plugin loaded keeps the message queued until it
  does. The wake arrives framed as hook feedback; tested with Haiku, a more
  cautious model may hesitate over it, which the per-session relay marker is
  there to help with.
- **Approvals race the desk.** A held `PermissionRequest` runs in parallel with
  the session's own dialog (VS Code, terminal): whoever answers first wins and
  the other side's prompt is withdrawn (verified on 2.1.282). A very fast answer
  from the phone can still make the desk dialog flash.
- **`bypassPermissions` sessions ask almost nothing.** Only questions, plans and
  the few actions no mode auto-approves reach the phone. Messages, typed text
  and keys sent to such a session need a device allowed dangerous actions.
- **Sessions started before the install** do not report until `/reload-plugins`
  is typed in them or they restart.
- **Headless `claude -p` / SDK runs** are never woken, and their
  `PermissionRequest` is held only while a dashboard is connected, for at most
  `MOTHER_CLAUDE_HEADLESS_HOLD_SECS` (default 120 s), so a script is never left
  hanging.

### tmux, background jobs and takeover

- **tmux typing is refused while a dialog is open** (`409 session_waiting`), so
  it never answers one by accident. Keys (Enter, Esc, arrows, digits), a leading
  `!` and slash commands outside a small allowlist need a device allowed
  dangerous actions. The pane is found from the pidfile's `tmux` target on the
  default socket or under `${TMUX_TMPDIR:-/tmp}/tmux-<uid>/`; set
  `MOTHER_CLAUDE_TMUX` if the binary is not on the usual paths.
- **PTY `claude attach` reaches background (`--bg`) jobs only**, by their short
  job id. VS Code, JetBrains, Desktop and terminal sessions are not hosted by
  the supervisor and cannot be attached to.
- **Foreign sessions update message by message.** Claude Code writes a main
  transcript record only when the API message completes, so a long reply appears
  when it is done. Owned sessions stream word by word; subagent transcripts are
  written per block and look live.
- **Continue here on a live session always forks** (a new session id): the
  original keeps running where it is. Two processes appending to one transcript
  would fork it silently, so Mother Claude never resumes a live session in
  place. Use one driver at a time.

### Pairing, tunnels and the certificate

- **Pairing is per address.** Browser storage is per site, so a phone paired at
  `https://192.168.x.x:6725` starts unpaired at the tunnel URL or the Tailscale
  address. A one-time code from **Settings › Home Screen app** (on a paired
  address) adds the other address to the same device.
- **The iPhone Home Screen app has its own storage** and pairs separately, the
  same way.
- **A Tailscale address (`100.x`) counts as the internet**: Same Wi-Fi codes are
  refused there, and the device's internet grant applies. Tailscale Serve is the
  smoother path.
- **iOS and the LAN certificate.** The LAN certificate comes from a local CA the
  phone does not know. Safari lets you tap through the warning for pages but
  refuses the WebSocket, so the app falls back to **Limited** (polling every few
  seconds) until the CA profile is installed and fully trusted.
- **Only some tunnels are detected**: ngrok (agent API), cloudflared quick
  tunnels (metrics port) and Tailscale Serve/Funnel. Named Cloudflare tunnels and
  other proxies need the public URL typed in by hand.
- **Quick-tunnel URLs change on every run**, which unpairs phones (their storage
  belongs to the old address) and breaks push subscriptions.
- **Switching Internet access off cuts only the tunnel port.** A tunnel pointed
  at 6725 keeps working (as remote, device-token-only traffic).
- **ngrok's free interstitial page** can intercept API calls; the app detects the
  HTML answer and asks you to open the page once in the browser.

### Push notifications

- **Push needs a trusted HTTPS origin.** It works through a tunnel's address, or
  on the LAN once the phone trusts the Mac's CA. On an untrusted LAN address the
  app can only alert while it is open (sound, vibration, the waiting count in
  the tab title).
- **iPhone and iPad** support Web Push only for a Home Screen app (iOS 16.4 or
  later), paired and enabled from inside it.
- **The Mac sends every notification** through the browser's push service, so it
  must be awake and online. A notification tapped away from home opens the
  address it was enabled on; a LAN address will not load there.
- The desktop app does not use push; it shows everything live.

### Always

- **Your Mac is the server.** Sessions, owned processes and the tunnel all run on
  it, so it has to stay awake (plugged in, sleep disabled) for the phone to
  reach anything.

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
  `PATH`, unless `Options.pathToClaudeCodeExecutable` says otherwise. The HTTP
  bridge and the Path A sidecar therefore run a different CLI version than owned
  Path B sessions (which run the installed `claude`); expect small behavioural
  drift.
- **Images must not be tiny.** A 2×2 PNG is rejected upstream with an untyped
  "an image in the conversation could not be processed"; 200×200 works. The
  bridge validates dimensions itself so the caller gets a typed error.

## Schema / CLI differences from the implementation brief

- **`claude agents --json` is no longer used.** It has no `state` or
  `waitingFor` field (2.1.185; 2.1.276 adds `name` and `status`), its shape
  drifts, and spawning it on every refresh is slow. The live roster comes from
  the `~/.claude/sessions/<pid>.json` pidfiles (never the `*.key` files next to
  them): pid, status (`busy` / `idle` / `waiting` + `waitingFor`), entrypoint,
  tmux target and job id. Liveness is `kill(pid, 0)` plus the process start
  time. Activity is derived from those, the owned control stream, hook events
  and the transcript tail.
- **`entrypoint` is not a reliable surface detector.** A child inherits
  `CLAUDE_CODE_ENTRYPOINT` from its parent, so a `claude -p` started from a VS
  Code terminal can report `claude-vscode`.
- **Background sessions are addressed by their short job id.** `claude --bg`
  did not exist on 2.1.185; on current versions background jobs have a short id
  (pidfile `jobId`, `jobs/<short>/state.json` `daemonShort`), and that, not the
  session uuid, is what `claude stop|respawn|rm|logs|attach` take. Mother Claude
  manages these jobs but spawns its own sessions through the headless stream-json
  path, not `--bg`.
- `~/.claude/jobs/` may contain only `pins.json` with no per-job `state.json`
  until background jobs run, so its parsing must tolerate total absence.
- `~/.claude/daemon/roster.json` was observed as `{proto, supervisorPid,
  updatedAt, workers}` with `workers` a **map**, not an array. Nothing reads it
  any more.
- Transcript `tool_use` / `tool_result` are **content blocks inside
  `message.content`**, not top-level NDJSON event types. Observed top-level
  `type`s also include `last-prompt`, `ai-title`, `custom-title`, `attachment`,
  `queue-operation`, `permission-mode`, `mode`, `cost-state` and
  `bridge-session` beyond the documented set.
- The `idle_prompt` notification did not fire in SDK-hosted sessions; idle comes
  from the pidfile `status` and `Stop` events.

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
- **Mother Claude never edits `~/.claude/settings.json`.** Foreign control is a
  plugin managed through `claude plugin install/uninstall`; the old checked-in
  HTTP hooks (which sent an empty token on 2.1.282) and the settings-editing
  hook installer are gone (`POST /api/hooks/install` answers `410`).

## Accounts and backups (docs/ACCOUNTS.md, docs/BACKUP.md)

- **One process reads one config directory.** "The shared `~/.claude` plus an
  account's own directory" cannot be told to the CLI: a directory account's
  conversations live in its directory, and continuing one elsewhere copies it.
- **A token account's terminal sessions on macOS** are recognised only through
  the hooks plugin: macOS 27 no longer shows another process's environment
  (`KERN_PROCARGS2`, `ps -E`), so the hook hashes the token inside the session.
  Without the plugin, or before its first hook, such a session shows the
  Default's chip. On Windows it is always the Default's (§11.6).
- **A token account's plan usage** comes from its sessions' own
  `rate_limit_event`s: a `setup-token` token is inference-only, so the CLI's
  `get_usage` is expected to answer "unavailable" for it, and the meter shows
  nothing until the account's first turn.
- **Removing an account** forgets it; its directory, its Keychain sign-in and
  its backup set stay.
- **Sync is not live.** Backups are pushed and pulled, not mirrored; a
  conversation continued on two laptops is kept as two copies (a conflict file
  beside the local one), never merged.
- **A restored session older than `cleanupPeriodDays`** is deleted again by
  Claude Code's next cleanup unless that setting is raised; the restore says
  so first.

## Foreign-session injection (experimental PTY)

The brief shipped PTY/CCR foreign control off by default; it is now compiled in
by default at the project owner's request (`default = ["experimental"]`;
runtime opt-out `MOTHER_CLAUDE_FOREIGN_INJECTION=0`, or build with
`--no-default-features`).

- PTY-driving `claude attach <short>` (Ink discards piped `\n`, so a real PTY is
  required) is **unsanctioned and unstable across versions**. It only reaches
  background (`--bg`) jobs, addressed by their short job id; a session without
  one is rejected (`not_a_background_session`), never attached to by uuid.
- It types plain text only: control characters and escapes are stripped, a
  leading `!` (shell mode) is refused, and every attach or injection needs the
  desktop or a device allowed dangerous actions.
- The reverse-engineered "CCR v1" transport remains **not implemented** (status
  probe only); its "no authentication" claim is unverified.

## Build-time decisions & deviations from the brief

These are intentional choices made while implementing on this machine; each keeps
the product working and the commits green.

- **Loopback HTTP + per-LAN-IP TLS** (instead of a single `0.0.0.0` TLS bind):
  the server always serves plain HTTP on `127.0.0.1:<port>` (desktop webview,
  hook plugin, local scripts) and TLS only on private LAN addresses (phones),
  with a certificate from a local CA name-constrained to private ranges. Public
  addresses are never bound; the internet is reached through a tunnel to the
  local-only tunnel port.
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
- **Transcript view** loads the newest 150 events and pages older ones in with
  **Load earlier** (the server reads backwards, at most 300 events per page)
  rather than using CDK virtual scrolling (variable-height items).
- **`ng test`** (karma) needs a headless Chrome; it is present on this machine.
  The enforced per-commit Angular gates are `npm run lint` and `npm run build`
  (plus `cargo fmt`/`clippy -D warnings`/`test`).

## Optional and unsanctioned pieces

- The **experimental PTY tier** is compiled in and on by default (see above).
- The **Path A sidecar** is off unless `MOTHER_CLAUDE_SIDECAR=1`.
- The **foreign-control plugin** is off until installed from the desktop app.
- **Internet access** is off until switched on under Remote.
