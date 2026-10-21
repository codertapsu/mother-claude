# Mother Claude

One dashboard — desktop **and** phone — to monitor and control every local
Claude Code session, agent, and service. Tauri v2 (Rust core) + Angular front end
+ an embedded `axum` HTTP/WebSocket server that serves the same dashboard over
your LAN — or from anywhere through a tunnel you run (Tailscale, ngrok,
cloudflared) — so you can drive Claude from your phone while you step away,
without turning on Claude's own Remote Control.

- See every session on the Mac and what it is doing right now (working /
  waiting for you / idle / ended / failed / stopped, and the tool it is running)
  across **all** surfaces — terminal CLI, tmux, VS Code / JetBrains, Claude
  Desktop, background jobs — from the live pidfiles and the shared
  `~/.claude/projects` store.
- Read live transcripts, token / cost / context usage, todos, tools and diffs,
  and per-session Git history and worktrees.
- **Control** sessions: spawn new ones, send instructions (with images and
  files), approve/deny permission prompts, answer questions and plans, switch
  model / mode / effort, interrupt, continue or fork, and stop / respawn / remove.
- Watch every running **agent**: subagents, workflow runs and background shells,
  with counts per session and their own live transcripts.
- **Sessions started elsewhere**: approve, answer and message them from the
  phone through an optional Claude Code plugin, or type into them in tmux.
- **Phone-first**: an Inbox of everything waiting for you, pairing by QR code,
  an installable Home Screen app, and Web Push when a session needs you or
  finishes.

## Download

Prebuilt installers for **macOS** (Apple silicon) and **Windows** (x64) are on
the [Releases page](https://github.com/codertapsu/mother-claude/releases).

- **macOS** — open the `.dmg` and drag the app to Applications. It is signed
  and notarized. Grant **Full Disk Access** when prompted (see
  [Permissions](#permissions-macos)). Intel Macs are not supported yet.
- **Windows** — run the `…-setup.exe` (or the `.msi`, for managed installs). The
  installers are currently unsigned, so SmartScreen shows a warning →
  **More info → Run anyway**.

**Requires the [Claude Code CLI](https://claude.com/claude-code)** (`claude`)
installed and signed in on the same machine — Mother Claude reads `~/.claude` and
drives the CLI. To build from source or cut a release yourself, see
[Build & quality gates](#build--quality-gates) and [docs/RELEASING.md](docs/RELEASING.md).

The desktop app **updates itself** — it checks for new versions on launch and you
can install one from **Settings → Software updates**. See
[docs/AUTOUPDATE.md](docs/AUTOUPDATE.md).

## The one hard constraint (read this)

Full two-way control is reliable only for sessions **Mother Claude launches
itself** ("owned"). There is no supported way to inject a typed answer or
permission decision into a *foreign* live session (started elsewhere and held by
the Claude Code supervisor — its control socket uses a rotating key over an
undocumented protocol). So:

- **Owned sessions** → full, reliable control over the installed `claude`'s
  stream-json control protocol (spawn, message with images, approve/deny,
  answer, plans, model / mode / effort, interrupt, stop a task), streamed token
  by token.
- **Foreign sessions** → full **monitoring** (transcripts, pidfiles, hook
  events), plus control through documented channels only:
  - the optional **foreign-control plugin** (installed from **Remote** with
    `claude plugin install`, never by editing your settings): approve and answer
    prompts from the phone in parallel with the session's own dialog, relay
    messages at the next stopping point, interrupt at the next tool call;
  - **tmux** sessions can be typed into (and answered with keys);
  - **background jobs** → lifecycle (stop / respawn / rm, by short job id), a
    terminal view and an experimental PTY injection that needs the dangerous
    right (`MOTHER_CLAUDE_FOREIGN_INJECTION=0` to disable).
  - Anything else: **“Continue here”** takes the session over as an owned one
    (a live session is forked, an ended one resumes in place). Switch devices —
    don't drive both at once.

  Each session shows what it supports and why (`control` capabilities); the
  full matrix is in [docs/REMOTE.md](docs/REMOTE.md#what-you-can-do-with-each-session).

Design intent: make the dashboard the *launcher* so "full control" is the norm.
See [ARCHITECTURE.md](ARCHITECTURE.md) and [KNOWN_ISSUES.md](KNOWN_ISSUES.md).

## Good to know (a short version is in the app → Settings → Good to know)

- **Your laptop is the brain.** Mother Claude runs on your laptop and bridges it
  to your phone — it isn't a cloud. Keep the laptop **awake** (plugged in, sleep
  disabled); if it sleeps, your phone loses the connection.
- **Sessions started elsewhere update message by message.** Claude Code writes a
  reply to its transcript when the reply is complete, so a long answer in a VS
  Code or terminal session appears when it is done (sessions Mother Claude owns
  stream word by word). *"Continue here"* picks up from what is on disk; a
  session that is still running elsewhere is forked, not taken over.
- **One driver at a time.** Continuing a session that is still open in an
  editor or terminal creates a fork: from then on there are two separate
  conversations. Carry on in one of them and close the other.
- **Local first, internet when you choose.** Pair phones with a one-time QR code
  from **Remote**. Internet access is off until you turn it on and point a
  tunnel at it; internet pairings always wait for you to type the phone's 4
  digits on the Mac. See [docs/REMOTE.md](docs/REMOTE.md).
- **Several Claude accounts at once.** **Settings → Accounts** adds accounts
  beside the one `~/.claude` is signed in to, each with a name of your choice:
  a *token account* (recommended: the same projects and conversations, another
  subscription — Mother Claude runs `claude setup-token` and keeps the token in
  the macOS Keychain) or a *directory account* (a fully separate Claude Code
  setup such as `~/.claude-work`, with its own sign-in, settings and plugins).
  Pick the account per project or per session, and when one reaches its limit,
  **Continue on…** another. See [docs/ACCOUNTS.md](docs/ACCOUNTS.md).
- **Move data anywhere, in bulk.** **Select** on the Sessions list, or *Move or
  copy data…* on an account, moves or copies any number of conversations,
  whole projects or everything an account has — between accounts, between the
  shared `~/.claude` and an account's own folder, and to or from cloud storage
  — plus settings, skills, agents, commands and MCP servers between accounts.
  Each move is previewed first and can be undone for seven days; it works from
  the phone too, on the LAN or through a tunnel.
- **Backups and a second laptop.** **Settings → Backup & sync** keeps encrypted
  backups of each account in a folder you choose (iCloud Drive, Google Drive…),
  and any number of further storages for archives and shared sets; the same
  folder carries conversations to another laptop. See
  [docs/BACKUP.md](docs/BACKUP.md).

## Prerequisites (macOS, Apple Silicon)

- Xcode Command Line Tools — `xcode-select --install`
- Node 24+ (Angular 22 needs ≥ 22.22.3) — `node --version`
- Rust + the native target — `rustup target add aarch64-apple-darwin`
  (and `rustup component add rustfmt clippy`)
- Claude Code CLI on `PATH` — `claude --version` (verified against `2.1.282`)
- Optional: `tmux`, to type into sessions that run in a tmux pane

## Quick start

```bash
npm install
npm run tauri:dev        # launch the desktop app (dev)
```

On start, the console prints the dashboard URLs (tokens only as fingerprints):

```
  Mother Claude dashboard
  Local:  http://127.0.0.1:6725
  LAN:    https://192.168.x.x:6725  (pair from Remote in the app)
  TLS fingerprint (SHA-256): AB:CD:…
  Token:  sha256:1a2b3c4d (the desktop app holds it)
  Claude: you@example.com (max)
  API:    http://127.0.0.1:6725/v1  (not started — start it on the API screen)
```

### Open it on your phone

1. Connect the phone to the **same Wi-Fi**.
2. On the desktop, open **Remote → Pair a device → Same Wi-Fi** and scan the
   **QR** with the phone's camera. It encodes a one-time code (valid 5 minutes,
   in the URL fragment — never a token). You can also type the code on the
   phone's Pair page.
3. The phone warns once about the Mac's certificate; the Pair page shows how to
   trust it (needed for live updates on iOS, the Home Screen app and push).

Away from home, turn on **Remote → Internet access**, start one of the tunnel
commands shown there, and pair with an **Internet** code: the phone shows 4
digits and you type them on the Mac. Details in [docs/REMOTE.md](docs/REMOTE.md).

## Build & quality gates

```bash
npm run build            # Angular production build -> dist/mother-claude/browser
npm run lint             # Angular ESLint
npm run tauri:build      # package the macOS app

cargo fmt   --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test  --manifest-path src-tauri/Cargo.toml
```

`just ci` runs them all. The `experimental` feature (PTY attach) is on by
default; check the build without it too:
`cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --no-default-features -- -D warnings`.

## Permissions (macOS)

On first run the app shows a short **onboarding** screen that detects any missing
OS permission and walks you through granting it:

- **Full Disk Access** (required) — to read `~/.claude/projects` (a TCC grant
  separate from your terminal). The guide deep-links to the right System Settings
  pane and offers "Reveal app in Finder" so you can add it, then "Re-check".
- **Local Network** (optional) — so your phone can reach the dashboard; macOS
  prompts the first time a device connects, just click Allow.

You can skip and revisit anytime under **Settings → Permissions**.

## Configuration (env)

| Var | Default | Purpose |
|---|---|---|
| `MOTHER_CLAUDE_HOST` | `0.0.0.0` | Loopback ⇒ HTTP on `127.0.0.1` only; `0.0.0.0` ⇒ also TLS on every private LAN address; one private address ⇒ only that one. |
| `MOTHER_CLAUDE_PORT` | `6725` | Port. |
| `CLAUDE_CONFIG_DIR` | `~/.claude` | Claude config dir of the **Default** account. Other accounts are added in the app ([ACCOUNTS.md](docs/ACCOUNTS.md)). |
| `MOTHER_CLAUDE_SECRET_STORE` | Keychain (macOS) | `file` ⇒ keep token accounts' tokens and the backup passphrase in `0600` files in the app data dir instead of the login Keychain (for builds whose changing signature would make the Keychain ask on every launch). |
| `MOTHER_CLAUDE_CLI` | `claude` | Path to the Claude binary. |
| `MOTHER_CLAUDE_SIDECAR` | off | `1` ⇒ drive owned sessions with the legacy Path A Agent SDK sidecar instead of the stream-json control protocol. |
| `MOTHER_CLAUDE_FOREIGN_INJECTION` | on | `0` ⇒ disable the experimental PTY injection into background jobs. |
| `MOTHER_CLAUDE_TMUX` | autodetect | Path to the `tmux` binary. |
| `MOTHER_CLAUDE_HEADLESS_HOLD_SECS` | `120` | How long a headless (`claude -p`/SDK) session's permission request is held for the dashboard. |
| `MOTHER_CLAUDE_ALLOW_REMOTE_DANGEROUS` | unset | `1` ⇒ paired devices on the LAN may take dangerous actions without a per-device grant (never through a tunnel). |
| `MOTHER_CLAUDE_TUNNEL_PORT` | `6726` | Local-only port the tunnel forwards to while Internet access is on. |
| `MOTHER_CLAUDE_DEVICE_IDLE_DAYS` | `30` | Paired devices unused this long are unpaired. |
| `MOTHER_CLAUDE_VAPID_SUBJECT` | public URL | `mailto:` or `https:` contact sent with Web Push. |
| `MOTHER_CLAUDE_WEB_DIR` | autodetect | Override the built SPA directory. |
| `MOTHER_CLAUDE_BRIDGE` | on | `0` ⇒ disable the HTTP bridge entirely. |
| `MOTHER_CLAUDE_BRIDGE_AUTOSTART` | off | `1` ⇒ start the API at launch (headless/CI) instead of from the app; `all` ⇒ every saved API server too, each closed to tunnels. |
| `MOTHER_CLAUDE_BRIDGE_ACCOUNT` | unset | The autostarted API's Claude account (an id or a name); one that does not exist starts nothing. |
| `MOTHER_CLAUDE_BRIDGE_ACCOUNT_ONLY` | off | `1` ⇒ the autostarted API runs as that account only. |
| `MOTHER_CLAUDE_BRIDGE_PORT` | `5612` | Dedicated loopback API port; `0` ⇒ serve only `/v1`. |
| `MOTHER_CLAUDE_BRIDGE_REQUIRE_TOKEN` | **off** | `1` ⇒ require the token on the loopback bridge port too. `never` ⇒ start with the API screen's *Open to tunnels* switch on: no token from anyone who can reach that port, through a tunnel too ([BRIDGE.md](docs/BRIDGE.md#what-the-open-default-actually-means)). |
| `MOTHER_CLAUDE_BRIDGE_CWD` | home dir | Default working directory for API conversations. |
| `MOTHER_CLAUDE_BRIDGE_ORIGINS` | unset | Unset ⇒ any browser origin may call the bridge; set ⇒ strict allowlist. |
| `ANTHROPIC_API_KEY` | unset | Enables the bridge's direct Messages API routes. |
| `MOTHER_CLAUDE_LOG` | `info` | `tracing` filter for the app's logs (never written under `~/.claude`). |

## The Node sidecar (built automatically)

The `sidecar/` package builds two Node programs on the Agent SDK:

- `bridge-host.js` — the runtime behind the Claude HTTP bridge (below). It
  starts on the bridge's first use.
- `agent-bridge.js` — the legacy **Path A** driver for owned sessions
  (`canUseTool` + an `ask_user` MCP tool). Owned sessions use the installed
  `claude` over the stream-json control protocol (Path B) by default; set
  `MOTHER_CLAUDE_SIDECAR=1` to use Path A instead.

`npm run tauri:dev` and `npm run tauri:build` **build it for you** (their
`beforeDevCommand`/`beforeBuildCommand` run `npm run sidecar:build` /
`sidecar:bundle`), and `tauri:build` bundles it into the app. So a single
`npm run tauri:dev` starts the embedded server, builds the sidecar and runs the
desktop app. (The SDK is large, so the sidecar adds ~275&nbsp;MB to a packaged
build.)

## Drive Claude over HTTP

Mother Claude also exposes Claude on this machine as a **local HTTP API** —
conversations, turns, live event streams, steering, interruption, tool-approval
callbacks, image and document input, plus a direct Messages API path for vision,
the Files API and programmatic tool calling.

```bash
curl -sS http://127.0.0.1:5612/chat -H 'Content-Type: application/json' \
  -d '{"message":"What does this project do?","cwd":"/path/to/repo"}'
```

Open the app's **API** screen, pick the defaults you want (working directory,
model, effort, permissions, and which conversation messages join), and press
**Start**. Nothing listens until you do. After that there is no token and no
setup for programs on this machine — curl, Postman, Swagger and a web app you
are building all just work. It is loopback-only; a request that came through a
tunnel needs a token, and the LAN mount always does.

A message with no `thread_id` joins the current conversation, so a client that
only sends `{"message": "…"}` keeps one continuous thread; pass
`createNewChat: true` to branch off a new one.

With the app running, open the **[API reference](http://127.0.0.1:5612/docs/)**
(Swagger, vendored — no CDN) or the **[console](http://127.0.0.1:5612/example/)**,
a runnable page that starts a conversation, streams it, and lets you approve tool
calls. There is a JS client at `/client.mjs` and an OpenAPI 3.1 document at
`/openapi.json`.

The same routes are served at `/v1` on the main app port, so a phone on your LAN
can use them over TLS. A conversation created through the API shows up in the
dashboard like any other session, and its approval prompts appear on both.

With more than one Claude account you can run **an API server per account**:
*Add API server* on the API screen gives one a port of its own (5613, 5614, …),
its own defaults and, with *Only this account*, an account it runs as and
nothing else — another account's conversations do not exist through it, so you
can put one account's API behind a tunnel without opening the others. Every
server is also reachable through the main port at `/servers/<id>/` (and on the
LAN at `/servers/<id>/v1`, with a token).

Full reference: **[docs/BRIDGE.md](docs/BRIDGE.md)**.

> For access from outside your network, use **Remote → Internet access** and a
> tunnel to its local-only port (6726) — never a raw port forward. Devices need
> their own pairing and never get dangerous actions through a tunnel unless you
> grant them. The bridge port (5612) binds loopback only; a tunnel pointed at it
> switches its tokenless mode off — and so for every other API server's port,
> each on its own. See [SECURITY.md](SECURITY.md) and
> [docs/REMOTE.md](docs/REMOTE.md).
