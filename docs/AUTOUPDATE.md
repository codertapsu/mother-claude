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
| Manifest | `latest.json`, merged by both release legs (`scripts/release/update-manifest.mjs`) and checked before publishing (`release-upload.sh verify` → `check-manifest.mjs`) |
| Signing key | `~/.tauri/mother-claude-updater.key` (private, **never committed**, key id `43FDB9942AC8CABC`); its public half is `plugins.updater.pubkey` |

The updater verifies every download against `pubkey`, so the **same private key**
must sign all platforms. Both release scripts load it from
`~/.tauri/mother-claude-updater.key` (on Windows
`%USERPROFILE%\.tauri\mother-claude-updater.key`), or take
`TAURI_SIGNING_PRIVATE_KEY` from the environment; it has no password. Keep the
key file backed up — losing it means no existing install can verify another
update. (The `TAURI_SIGNING_PRIVATE_KEY` repository secret only matters if CI
ever builds again.)

## How a release produces `latest.json`

Both halves of a release are built locally, on two machines, in either order
([RELEASING.md](RELEASING.md)). Each leg signs its update artifacts and **merges**
its entries into the `latest.json` already on the draft — never over it:

- **macOS** (`scripts/release/release-macos.sh`) adds `darwin-aarch64` →
  `Mother.Claude.app.tar.gz`.
- **Windows** (`scripts/release/release-windows.ps1`) adds `windows-x86_64-nsis`
  and `windows-x86_64` → the `-setup.exe`, and `windows-x86_64-msi` → the `.msi`.
  The updater looks up `{os}-{arch}-{installer}` before `{os}-{arch}`, so each
  install is updated by the installer that made it.

```json
{
  "version": "0.4.0",
  "pub_date": "2026-10-13T06:00:00Z",
  "notes": "What's new in 0.4.0: https://github.com/codertapsu/mother-claude/releases/tag/v0.4.0",
  "platforms": {
    "darwin-aarch64":      { "signature": "<.app.tar.gz.sig>", "url": "…/v0.4.0/Mother.Claude.app.tar.gz" },
    "windows-x86_64":      { "signature": "<-setup.exe.sig>",  "url": "…/v0.4.0/Mother.Claude_0.4.0_x64-setup.exe" },
    "windows-x86_64-msi":  { "signature": "<.msi.sig>",        "url": "…/v0.4.0/Mother.Claude_0.4.0_x64_en-US.msi" },
    "windows-x86_64-nsis": { "signature": "<-setup.exe.sig>",  "url": "…/v0.4.0/Mother.Claude_0.4.0_x64-setup.exe" }
  }
}
```

`notes` is what **Settings → Software updates** shows under "update available".
The merge refuses to write a signature that does not verify against its archive
under `pubkey`, or one that records another version (tauri-plugin-updater 2.13
refuses those), and drops every entry left from another version. Before
publishing, `release-upload.sh verify` downloads the manifest and every asset it
names and checks each signature the way the updater does.

> The macOS updater downloads the **`.app.tar.gz`**, not the `.dmg` (the dmg is only
> the first-install download). Both are on the release.

## Rotating the key

`npm run tauri -- signer generate -w ~/.tauri/mother-claude-updater.key -p ""`, put
the new `.key.pub` contents in `plugins.updater.pubkey`, copy the new key to the
Windows desktop, and ship a release. Installs older than that release can no
longer auto-update (they'd verify against the old key) — so rotate only when
necessary.
