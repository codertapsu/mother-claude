# Auto-update (currently OFF)

Mother Claude releases do **not** auto-update yet:
`bundle.createUpdaterArtifacts` is `false` and the updater plugin is not wired in.
Users update by downloading a newer installer. This document is the checklist for
turning auto-update on in a later release.

The updater works by having the app periodically fetch a signed `latest.json`
manifest from the GitHub Release, compare versions, and download + verify the new
bundle against a public key baked into the app.

## Enabling it

1. **Generate a signing keypair** (once; keep the private key secret):
   ```bash
   npm run tauri -- signer generate -w ~/.tauri/mother-claude.key
   ```
   This prints a **public key** and writes the **private key** (optionally
   password-protected).

2. **Add the public key + endpoint** to `src-tauri/tauri.conf.json`:
   ```jsonc
   "plugins": {
     "updater": {
       "endpoints": [
         "https://github.com/codertapsu/mother-claude/releases/latest/download/latest.json"
       ],
       "pubkey": "<the public key from step 1>"
     }
   },
   "bundle": { "createUpdaterArtifacts": true }
   ```

3. **Add the plugin** to the Rust app:
   - `Cargo.toml`: `tauri-plugin-updater = "2"` (and `tauri-plugin-process = "2"`
     to relaunch after applying).
   - `src-tauri/src/lib.rs`: `.plugin(tauri_plugin_updater::Builder::new().build())`
     in the Tauri builder, plus the UI/flow that checks for and installs updates.

4. **CI secrets** (Settings → Secrets and variables → Actions → Secrets):
   - `TAURI_SIGNING_PRIVATE_KEY` — contents of `~/.tauri/mother-claude.key`.
   - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` — its password (`''` if none). The
     workflow already passes both to `tauri-action`.

5. **Emit the manifest**: set `includeUpdaterJson: true` on the `tauri-action`
   step in `.github/workflows/release.yml` so each release publishes `latest.json`
   alongside the installers.

6. Cut a release as usual ([RELEASING.md](RELEASING.md)). From then on, installed
   apps see the new `latest.json` and offer the update.

## `latest.json` shape (for reference)

`tauri-action` generates this; you don't hand-write it:

```json
{
  "version": "0.2.0",
  "notes": "See the release page.",
  "pub_date": "2026-06-24T00:00:00Z",
  "platforms": {
    "darwin-aarch64": { "signature": "…", "url": "https://github.com/codertapsu/mother-claude/releases/download/v0.2.0/Mother.Claude_aarch64.app.tar.gz" },
    "darwin-x86_64":  { "signature": "…", "url": "…" },
    "windows-x86_64": { "signature": "…", "url": "…" }
  }
}
```

> Auto-update only makes sense once the macOS build is **signed + notarized**
> (otherwise Gatekeeper blocks the downloaded update). Set up
> [APPLE_SIGNING.md](APPLE_SIGNING.md) first.
