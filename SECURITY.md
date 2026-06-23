# Security

> Stub — expanded by the `docs` commit.

Mother Claude exposes a control plane for your Claude Code sessions over the LAN.
Treat it as sensitive.

- **TLS by default** for any non-loopback bind (self-signed cert generated at
  runtime; fingerprint shown in the UI).
- **Token auth** on every `/api/*`, `/ws`, `/hooks/*` request. Optional dashboard
  password. Device pairing via QR.
- **Dangerous actions** (`bypassPermissions` / `--dangerously-skip-permissions`,
  irreversible lifecycle) are **local-desktop-only by default**.
- **Never expose port 6725 to the public internet.** For access beyond a trusted
  LAN, use **Tailscale / WireGuard**.
- Secrets (tokens, passwords, TLS certs) are generated at runtime into the app
  config dir and are **never committed**.
