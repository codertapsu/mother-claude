# Releasing Mother Claude

Mother Claude ships as desktop installers for **macOS** (arm64 + x64) and
**Windows** (x64). Every release is published as a **draft** GitHub Release that a
human reviews and publishes. You can build a release **locally** (the default — no
CI minutes) or in **CI**, per OS.

> Runtime prerequisite for end users: the **Claude Code CLI** (`claude`) installed
> and signed in. Mother Claude reads `~/.claude` and drives the CLI; it is not a
> standalone tool.

---

## The three version files (keep in sync)

| File | Field |
|---|---|
| `package.json` | `"version"` |
| `src-tauri/tauri.conf.json` | `"version"` |
| `src-tauri/Cargo.toml` | `[package] version` |

Bump all three at once:

```bash
node scripts/release/set-version.mjs 0.2.0   # a leading "v" is fine
```

The Git tag (`v0.2.0`) is what triggers a CI release and names the GitHub Release.

---

## One-time setup

1. **macOS signing secrets** (for a distributable, notarized `.dmg`) — see
   [APPLE_SIGNING.md](APPLE_SIGNING.md). Without them, macOS still builds, just
   **unsigned** (users right-click → Open). Add under **Settings → Secrets and
   variables → Actions → Secrets**:
   `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`,
   `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`.

2. **Per-OS CI toggles** (only if you want CI to build instead of your machine) —
   under **Settings → Secrets and variables → Actions → Variables**:
   `RELEASE_CI_MACOS=true` and/or `RELEASE_CI_WINDOWS=true`. Unset ⇒ that OS is
   built **locally** (CI provisions no runner for it — no surprise 10×-billed
   macOS minutes).

3. **Auto-update** is **off** for now. Enabling it is a separate, documented step:
   [AUTOUPDATE.md](AUTOUPDATE.md).

---

## Recommended split: macOS local + Windows CI

macOS Actions minutes are billed 10×, and signing is simplest against the
Developer ID cert already in your Mac's login keychain — so **build macOS
locally** and let **CI build Windows**. Both legs target the **same tag's draft
release** (the local upload and CI's tauri-action converge on it), so the order
doesn't matter.

```bash
# --- macOS (local, on your Mac) ---
export APPLE_SIGNING_IDENTITY="Developer ID Application: Khanh Hoang (4XVYLW8RXS)"
export APPLE_ID="hoangduykhanh.dn@gmail.com"
export APPLE_PASSWORD="<app-specific password>"
export APPLE_TEAM_ID="4XVYLW8RXS"
RELEASE_TAG=v0.1.0 bash scripts/release/macos-release.sh   # build + notarize + upload

# --- Windows (CI) ---
# one-time: set repo Variable RELEASE_CI_WINDOWS=true (leave RELEASE_CI_MACOS unset)
git tag v0.1.0 && git push origin main --tags              # CI builds Windows only → same draft
```

`macos-release.sh` runs `npm run tauri:build` (which builds the sidecar + Angular,
signs with the keychain identity, notarizes, and staples the `.dmg`), verifies the
signature, and uploads to the draft. `APPLE_CERTIFICATE` is **not** needed locally
— the cert comes from your keychain; it's only a CI secret. Then review + publish
the draft.

## Cut a release — locally (manual steps)

No CI minutes spent. Build on each OS you ship and upload to the same draft.

```bash
# 1. Bump + commit + tag
node scripts/release/set-version.mjs 0.2.0
git commit -am "release: v0.2.0"
git tag v0.2.0 && git push origin main --tags     # tag push is harmless: CI builds
                                                   # only the OSes whose RELEASE_CI_* is true

# 2. Build the installers
npm ci
npm run tauri:build            # macOS: .dmg/.app under src-tauri/target/release/bundle/
                               # Windows: .exe/.msi under src-tauri/target/release/bundle/

# 3. Upload to the draft (creates it if needed; replaces same-named assets)
#    macOS / Linux:
RELEASE_TAG=v0.2.0 bash scripts/release/release-upload.sh upload \
  src-tauri/target/release/bundle/dmg/*.dmg

#    Windows (PowerShell):
$env:RELEASE_TAG="v0.2.0"; pwsh scripts/release/release-upload.ps1 -Upload `
  src-tauri/target/release/bundle/nsis/*-setup.exe `
  src-tauri/target/release/bundle/msi/*_en-US.msi
```

For a **notarized** local macOS build, export the Apple env vars before
`npm run tauri:build` — Tauri signs + notarizes automatically (see
[APPLE_SIGNING.md](APPLE_SIGNING.md#local-signed-build)).

Then **review and publish**: <https://github.com/codertapsu/mother-claude/releases>.

---

## Cut a release — in CI

1. Set `RELEASE_CI_MACOS` / `RELEASE_CI_WINDOWS` to `true` for the OSes you want
   CI to build (one-time).
2. Bump the version, commit, and **push the tag**:
   ```bash
   node scripts/release/set-version.mjs 0.2.0
   git commit -am "release: v0.2.0" && git tag v0.2.0
   git push origin main --tags
   ```
3. The **Release** workflow builds the enabled OSes, signs/notarizes macOS (if the
   Apple secrets exist), and uploads to a **draft** Release.
4. **Review and publish** the draft.

A manual **Run workflow** (`workflow_dispatch`) builds **every** OS regardless of
the toggles — handy for a one-off full build.

---

## Troubleshooting

- **macOS build dies importing a cert** — `APPLE_CERTIFICATE` is set but empty, or
  the `.p12`/password is wrong. The workflow only exports the Apple env when the
  secret is non-empty; double-check the base64 and password.
- **Notarization fails / "The binary is not signed"** — verify `APPLE_TEAM_ID`,
  `APPLE_ID`, and the **app-specific** `APPLE_PASSWORD`; the signing identity must
  be a *Developer ID Application* cert. See [APPLE_SIGNING.md](APPLE_SIGNING.md).
- **The build job was skipped** — every OS is set to local (`RELEASE_CI_*` unset).
  That's expected; build locally or set a toggle. Use **Run workflow** to force a
  full CI build.
- **Windows SmartScreen warning** — expected; Windows installers are unsigned for
  now (More info → Run anyway). Optional Authenticode setup is in
  [APPLE_SIGNING.md](APPLE_SIGNING.md#windows-authenticode-optional).
- **Phone can't reach a published build** — unrelated to releasing; check pairing
  and that the laptop is awake on the same Wi-Fi (see the app's *Good to know*).
