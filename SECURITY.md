# Security

Mother Claude exposes a control plane for your Claude Code sessions over the LAN.
Treat it as sensitive: anyone with the token can read transcripts and drive owned
sessions.

## Authentication

- A random **64-char API token** is generated on first run and persisted (mode
  `0600`) in the app data dir. It is required on every `/api/*`, `/ws`, and
  `/hooks/*` request — via `Authorization: Bearer`, `?token=` (needed for browser
  WebSockets), or an `mc_token` cookie. The static SPA is served without auth so a
  phone can load the app and then authenticate.
- The desktop webview gets the token from the `server_info` invoke; phones get it
  from the pairing QR / link, or by pasting it in Settings.

## TLS

- Any **non-loopback** bind serves **TLS** (self-signed cert generated with
  `rcgen`, SANs = `localhost` + loopback + detected LAN IPs), persisted in the
  config dir. The SHA-256 **fingerprint** is shown in Settings — verify it on the
  phone the first time. Loopback (`127.0.0.1`) is plain HTTP and is what the
  desktop webview, hooks, and the local sidecar use.
- We use the `ring` rustls provider (no aws-lc C toolchain dependency).

## Device pairing

Settings renders an SVG **QR** encoding `https://<lan-ip>:6725/#/pair?token=…`.
Scanning it pairs a phone in one step. The token can be rotated by deleting the
`token` file in the app data dir and restarting.

## The dangerous-action gate

Irreversible or high-risk actions are **restricted to the local desktop**
(loopback peer) by default:

- approving a **dangerous** permission (e.g. `bypassPermissions` /
  `--dangerously-skip-permissions`),
- **removing** a session (`rm`, deletes the worktree),
- installing hooks into your user settings,
- experimental PTY attach/inject.

Set `MOTHER_CLAUDE_ALLOW_REMOTE_DANGEROUS=1` to permit these from remote clients
(not recommended). Normal monitoring and answering ordinary prompts work
remotely.

## Hooks & secrets

`POST /api/hooks/install` writes Mother Claude's hook block into your
`~/.claude/settings.json` (backing it up first) so *foreign* sessions emit events
to us. Because foreign sessions don't inherit our env, the **literal token** is
embedded in that hook header. This is your own machine's config (not committed),
but be aware the token is then readable there.

Tokens and generated TLS certs are created at runtime into the app config dir and
are **never committed** (`.gitignore` excludes `*.pem`, `*.key`, `*.token`).

## Network exposure

- **Never expose port 6725 to the public internet.** It is a remote-control
  surface for code execution.
- For access beyond a trusted LAN, use **Tailscale** or **WireGuard** and bind to
  the VPN interface (or keep `0.0.0.0` behind the VPN's firewall). The token +
  TLS are a second layer, not a substitute for not being on the open internet.

## Reporting

This is a research-preview-grade tool over undocumented internals. Audit before
relying on it in any shared environment.
