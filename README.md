# Mother Claude

One dashboard — desktop **and** phone — to monitor and control every local
Claude Code session, agent, and service. Tauri v2 (Rust core) + Angular front end
+ an embedded `axum` HTTP/WebSocket server that serves the same dashboard over
your LAN so you can drive Claude from your phone while you step away.

> **Full two-way control is reliable only for sessions Mother Claude launches
> itself ("owned").** Sessions started elsewhere ("foreign") get full monitoring
> and lifecycle control (stop / respawn / rm) but not live answer injection.
> See [ARCHITECTURE.md](ARCHITECTURE.md) and [KNOWN_ISSUES.md](KNOWN_ISSUES.md).

## Prerequisites (macOS, Apple Silicon)

- Xcode Command Line Tools — `xcode-select --install`
- Node 20+ — `node --version`
- Rust + the native target — `rustup target add aarch64-apple-darwin`
- Claude Code CLI on `PATH` (`claude --version`)

## Quick start

```bash
npm install
npm run tauri:dev        # desktop app (dev)
```

Then open the dashboard on your phone: connect to the same Wi-Fi, scan the
pairing QR in **Settings**, and load `https://<lan-ip>:6725`.

Full docs: [ARCHITECTURE.md](ARCHITECTURE.md) · [SECURITY.md](SECURITY.md) ·
[KNOWN_ISSUES.md](KNOWN_ISSUES.md). (Build instructions are filled in by the
`build(tauri)` commit.)
