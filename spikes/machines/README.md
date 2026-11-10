# Machines — Phase 0 spikes

Throwaway tools that answer the questions of Phase 0 in
[docs/PLAN-MACHINES.md](../../docs/PLAN-MACHINES.md) §12. **Nothing here is
part of the app**: the app's gates (`cargo … --manifest-path src-tauri/…`,
`npm run lint`) never build it, and no release ships it. Results are in
[RESULTS.md](RESULTS.md).

| Spike | Here | Question |
|---|---|---|
| S1 transport | `mc-spike/` (Rust) | iroh between machines: addressing by key, the app's HTTP + WebSocket stack over QUIC streams, throughput, the three relay settings |
| S2 code | `code/roundtrip.mjs` (Node) | snapshot / transfer / apply of a dirty working tree, byte-exact, both transfers and both landings |
| S3 Claude Code | commands in RESULTS.md | resuming a moved session: the move note, the recorded system prompt, another folder |
| S4 UX | `UX.md` | the screens to agree on before Phase 1 |
| S5 setup | `mc-spike checks` | what each check of §5.14 can detect, read-only, as a standard user |

## Build

```sh
cargo build --release --manifest-path spikes/machines/mc-spike/Cargo.toml
# Windows: the same command in a Developer PowerShell (MSVC), from the repo root.
```

The binary is `spikes/machines/mc-spike/target/release/mc-spike` (`.exe` on
Windows). Build output and `out/` are git-ignored.

## What it touches

- **Keys**: `<data dir>/mc-spike/<name>.key` (macOS `~/Library/Application
  Support`, Windows `%APPDATA%`), 0600 on unix; paired peers in
  `<name>.allowed` beside it. Delete the folder to forget everything.
- **Mother Claude**: only with `serve --forward`, and then only `GET
  /api/sessions` and `GET /api/health` on `http://127.0.0.1:6725`, as a local
  script (the app's `script-token`, read from its data dir and never printed),
  for a peer that paired with the one-time code. `get` prints a count of
  sessions by status, never their content.
- **Network**: n0's public relays and DNS unless `--relay none` or your own
  relay; mDNS on the LAN; `checks` sends one datagram to the gateway's discard
  port and one mDNS query.
- Never a config directory, a project, a token in output, or an account's
  email (`checks` reduces `claude auth status` to signed in or not).

## Commands

```sh
mc-spike local [--only direct,mdns,own-relay,public-relay,n0-dns] [--mib 256] [--relay-mib 4]
mc-spike checks > out/checks-<os>.json
mc-spike serve --pair-code auto [--forward http://127.0.0.1:6725]   # prints its id and the code
mc-spike get   <ID> /api/sessions --pair-code <CODE>                # from the other machine
mc-spike bench <ID> --mib 64                                        # once paired
```

`--relay public` (default) | `none` | `https://your-relay…`; `--lookup n0,mdns`
(defaults: public → `n0,mdns`, otherwise `mdns`); `--name` picks the key
(`spike` for serve, `client` for get and bench). `MC_SPIKE_LOG=iroh=debug`
shows the library's logs.

## The Windows half

Steps, in RESULTS.md § "Still to run", for a Claude session on the Windows
desktop (or by hand).
