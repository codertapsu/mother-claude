---
description: Walk the local release runbook for a version — preflight, then the per-machine build steps.
argument-hint: [version, e.g. 0.4.0]
---

Cut a Mother Claude release for version **$1** (or ask which version, if not
given).

**Installers are built locally, never in CI.** Do not add a tag trigger to
`.github/workflows/release.yml` or enable it. The full runbook is
[docs/RELEASING.md](../../docs/RELEASING.md).

## What to do

1. **Check the current state** — `git status`, `git log --oneline -5`, and
   `node scripts/release/set-version.mjs --check`. Report where things stand
   before changing anything.

2. **Bump the version** in every spot: `node scripts/release/set-version.mjs $1`.
   Show the diff; it touches `package.json`, `package-lock.json`,
   `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock` and `src-tauri/tauri.conf.json`,
   one line each (two in `package-lock.json`).

3. **Run every gate** — `cargo fmt --check`, `cargo clippy --all-targets -- -D
   warnings`, `cargo test` (all with `--manifest-path src-tauri/Cargo.toml`),
   `npm run lint`, `npm test`, `npm run test:release`. Do not proceed on a
   failure.

4. **Commit and tag.** Ask before tagging; a pushed tag is awkward to retract.
   Use `chore(release): v$1` as the commit subject, and **no AI attribution** in
   the message.

5. **Report the two build commands**, and say clearly which machine each runs
   on. Do not run either yourself without being asked — they sign, notarize,
   and upload:

   ```bash
   # On the Mac:
   git checkout v$1
   source scripts/apple-credentials.env
   RELEASE_TAG=v$1 bash scripts/release/release-macos.sh
   ```

   ```powershell
   # On the Windows desktop:
   git fetch --tags origin; git checkout v$1
   $env:RELEASE_TAG = 'v$1'
   pwsh scripts\release\release-windows.ps1
   ```

6. **Remind about the last step**: the two legs run **in parallel** on their own
   machines and upload to the _same_ draft. Before publishing, check both
   finished — the draft needs **eight** assets (three from the Mac, four from
   Windows, plus the shared `latest.json`) and every update must verify:

   ```bash
   RELEASE_TAG=v$1 bash scripts/release/release-upload.sh verify
   ```

   A human then writes the notes and publishes it on the Releases page.

If anything in the preflight is already wrong — dirty tree, disagreeing version
spots, a failing gate — stop and report it rather than working around it.
