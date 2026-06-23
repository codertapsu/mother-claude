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

## Full Disk Access (macOS)

The packaged GUI app needs **Full Disk Access** to read `~/.claude/projects`
(a TCC grant separate from your terminal). Settings shows the status and a button
to open the right pane: System Settings → Privacy & Security → Full Disk Access.

## Configuration (env)

| Var | Default | Purpose |
|---|---|---|
| `MOTHER_CLAUDE_HOST` | `0.0.0.0` | Bind host. Loopback ⇒ HTTP only; non-loopback ⇒ +TLS on LAN IPs. |
| `MOTHER_CLAUDE_PORT` | `6725` | Port. |
| `CLAUDE_CONFIG_DIR` | `~/.claude` | Claude config dir to read. |
| `MOTHER_CLAUDE_CLI` | `claude` | Path to the Claude binary. |
| `MOTHER_CLAUDE_SIDECAR` | unset | `1` ⇒ use the Node Agent SDK sidecar (Path A) for owned sessions. |
| `MOTHER_CLAUDE_ALLOW_REMOTE_DANGEROUS` | unset | `1` ⇒ allow remote clients to approve dangerous actions / `rm`. |
| `MOTHER_CLAUDE_WEB_DIR` | autodetect | Override the built SPA directory. |

## Optional: Path A sidecar (rich permission gating)

```bash
cd sidecar && npm install && npm run build
MOTHER_CLAUDE_SIDECAR=1 npm run tauri:dev
```

This drives owned sessions through the Claude Agent SDK with `canUseTool` and a
custom `ask_user` MCP tool, so every tool and question routes to the dashboard.

> Never expose port 6725 to the public internet. See [SECURITY.md](SECURITY.md).
