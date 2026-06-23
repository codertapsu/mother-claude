# Mother Claude

One dashboard — desktop **and** phone — to monitor and control every local
Claude Code session, agent, and service. Tauri v2 (Rust core) + Angular front end
+ an embedded `axum` HTTP/WebSocket server that serves the same dashboard over
your LAN so you can drive Claude from your phone while you step away.

- See every session and its derived state (working / needs-input / idle /
  completed / failed / stopped) across **all** surfaces — terminal CLI, VS Code /
  JetBrains, Claude Desktop — by reading the shared `~/.claude/projects` store.
- Read live transcripts (tailed JSONL), token/cost usage, tools, and per-session
  Git diffs / history / worktrees.
- **Control** sessions: spawn new ones, send instructions, approve/deny
  permission prompts, answer questions, and stop / respawn / remove.

## The one hard constraint (read this)

Full two-way control is reliable only for sessions **Mother Claude launches
itself** ("owned"). There is no supported way to inject a typed answer or
permission decision into a *foreign* live session (started elsewhere and held by
the Claude Code supervisor — its control socket uses a rotating key over an
undocumented protocol). So:

- **Owned sessions** → full control (spawn, message, approve/deny, answer).
- **Foreign sessions** → full **monitoring** (transcripts + hook events) and
  **lifecycle** (stop / respawn / rm), but **no live answer injection** — except
  the experimental tier (off by default). The UI clearly marks foreign sessions
  as read-only for injection.

Design intent: make the dashboard the *launcher* so "full control" is the norm.
See [ARCHITECTURE.md](ARCHITECTURE.md) and [KNOWN_ISSUES.md](KNOWN_ISSUES.md).

## Prerequisites (macOS, Apple Silicon)

- Xcode Command Line Tools — `xcode-select --install`
- Node 20+ — `node --version`
- Rust + the native target — `rustup target add aarch64-apple-darwin`
  (and `rustup component add rustfmt clippy`)
- Claude Code CLI on `PATH` — `claude --version` (built against `2.1.185`)

## Quick start

```bash
npm install
npm run tauri:dev        # launch the desktop app (dev)
```

On start, the console prints the dashboard URLs and the API token:

```
  Mother Claude dashboard
  Local:  http://127.0.0.1:6725
  LAN:    https://192.168.x.x:6725  (scan the QR in Settings)
  Token:  <random token>
```

### Open it on your phone

1. Connect the phone to the **same Wi-Fi**.
2. On the desktop, open **Settings** → scan the pairing **QR** (it encodes
   `https://<lan-ip>:6725/#/pair?token=…`).
3. The cert is self-signed: your phone warns once — verify the **fingerprint**
   shown in Settings, then trust it.

From the phone you can monitor everything and, for **owned** sessions, spawn,
send instructions, and approve/deny prompts.

## Build & quality gates

```bash
npm run build            # Angular production build -> dist/mother-claude/browser
npm run lint             # Angular ESLint
npm run tauri:build      # package the macOS app

cargo fmt   --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test  --manifest-path src-tauri/Cargo.toml
```

`just ci` runs them all. The experimental tier:
`cargo clippy --manifest-path src-tauri/Cargo.toml --features experimental -- -D warnings`.

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
| `MOTHER_CLAUDE_HOST` | `0.0.0.0` | Bind host. Loopback ⇒ HTTP only; non-loopback ⇒ +TLS on LAN IPs. |
| `MOTHER_CLAUDE_PORT` | `6725` | Port. |
| `CLAUDE_CONFIG_DIR` | `~/.claude` | Claude config dir to read. |
| `MOTHER_CLAUDE_CLI` | `claude` | Path to the Claude binary. |
| `MOTHER_CLAUDE_SIDECAR` | on | `0` ⇒ disable the Path A sidecar and use the headless path. |
| `MOTHER_CLAUDE_ALLOW_REMOTE_DANGEROUS` | unset | `1` ⇒ allow remote clients to approve dangerous actions / `rm`. |
| `MOTHER_CLAUDE_WEB_DIR` | autodetect | Override the built SPA directory. |

## The Path A sidecar (built & run automatically)

The Node Agent SDK sidecar — which drives owned sessions through `canUseTool` and
a custom `ask_user` MCP tool so every tool and question routes to the dashboard —
is now a first-class component:

- `npm run tauri:dev` and `npm run tauri:build` **build it for you** (their
  `beforeDev`/`beforeBuild` hooks run `npm run sidecar:build`); `tauri:build`
  also bundles it into the `.app` so the packaged app ships it.
- The Rust core uses the sidecar **automatically** whenever it's present, and
  falls back to the headless `claude -p` path if it isn't built, `node` is
  missing, or you set `MOTHER_CLAUDE_SIDECAR=0`.

So there is no manual step — a single `npm run tauri:dev` starts the embedded
server, builds the sidecar, and runs the desktop app. (The SDK is large, so the
sidecar adds ~275&nbsp;MB to a packaged build.)

> Never expose port 6725 to the public internet. See [SECURITY.md](SECURITY.md).
