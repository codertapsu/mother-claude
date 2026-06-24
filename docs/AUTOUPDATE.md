# Auto-update

Mother Claude has an in-app auto-updater (enabled since v0.1.0). On the desktop
app it checks the GitHub Release `latest.json` on startup, and **Settings →
Software updates** lets you check, download, install, and relaunch. A new version
is offered with a header **⬆ Update** badge.

It does nothing in a phone browser (the browser can't replace the desktop binary)
and nothing until a release **newer** than the running version is published.

## How it's wired

| Piece | Where |
|---|---|
| Rust plugins | `tauri-plugin-updater`, `tauri-plugin-process` in `src-tauri/Cargo.toml`, registered in `src-tauri/src/lib.rs` |
| Capability | `updater:default`, `process:allow-restart` in `src-tauri/capabilities/default.json` |
| Config | `bundle.createUpdaterArtifacts: true` + `plugins.updater` (endpoints + `pubkey`) in `src-tauri/tauri.conf.json` |
| Endpoint | `https://github.com/codertapsu/mother-claude/releases/latest/download/latest.json` (always served from the latest published release) |
| UI | `src/app/core/updater.service.ts` + `src/app/shared/updater.component.ts` (desktop-only) |
| Manifest | `latest.json` assembled by `scripts/release/make-latest-json.sh` |
| Signing key | `~/.tauri/mother-claude-updater.key` (private, **never committed**); its public half is `plugins.updater.pubkey` |

The updater verifies every download against `pubkey`, so the **same private key**
must sign all platforms. It is provided as:
- the GitHub **secret** `TAURI_SIGNING_PRIVATE_KEY` (+ `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`,
  empty here) — used by the Windows CI build, and
- the env var `TAURI_SIGNING_PRIVATE_KEY` exported locally for the macOS build.

> One-time setup: add the private key (`cat ~/.tauri/mother-claude-updater.key`) as
> the repo secret **`TAURI_SIGNING_PRIVATE_KEY`**, and leave
> `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` unset (the key has no password). Keep the key
> file backed up — losing it means existing installs can't verify future updates.

## How a release produces `latest.json`

macOS is built locally and Windows in CI, so the manifest is assembled from both:

1. **Windows CI** (`includeUpdaterJson: true`, `createUpdaterArtifacts: true`) builds
   and uploads `…-setup.exe` + `…-setup.exe.sig` to the tag's draft release.
2. **macOS local** (`scripts/release/macos-release.sh`, with `TAURI_SIGNING_PRIVATE_KEY`
   exported) builds `…aarch64.app.tar.gz` + `.sig`, uploads it, then runs
   `make-latest-json.sh`, which reads the real asset names off the release and writes
   one `latest.json`:
   ```json
   {
     "version": "0.1.0",
     "pub_date": "…",
     "platforms": {
       "darwin-aarch64": { "signature": "<.app.tar.gz.sig contents>", "url": "https://github.com/codertapsu/mother-claude/releases/download/v0.1.0/Mother.Claude_0.1.0_aarch64.app.tar.gz" },
       "windows-x86_64": { "signature": "<-setup.exe.sig contents>",  "url": "https://github.com/codertapsu/mother-claude/releases/download/v0.1.0/Mother.Claude_0.1.0_x64-setup.exe" }
     }
   }
   ```
   The script self-verifies (each platform has a signature and its referenced asset
   exists on the release) before finishing.

> The macOS updater downloads the **`.app.tar.gz`**, not the `.dmg` (the dmg is only
> the first-install download). Both are on the release.

## Rotating the key

`npm run tauri -- signer generate -w ~/.tauri/mother-claude-updater.key -p ""`, put
the new `.key.pub` contents in `plugins.updater.pubkey`, replace the
`TAURI_SIGNING_PRIVATE_KEY` secret, and ship a release. Installs older than that
release can no longer auto-update (they'd verify against the old key) — so rotate
only when necessary.
