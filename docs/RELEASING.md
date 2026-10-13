# Releasing Mother Claude

Mother Claude ships desktop installers for **macOS** (Apple silicon `.dmg`) and
**Windows** (x64 `-setup.exe` and `.msi`).

**Every artifact is built locally.** The maintainer has two machines — a MacBook
and a Windows desktop — and each builds its own half. Both upload to the _same_
draft GitHub Release on `codertapsu/mother-claude`, in whichever order suits; a
human reviews and publishes it. This is the arrangement Gitweaver uses, and the
scripts in `scripts/release/` are its tooling adapted to this project.

> Runtime prerequisite for end users: the **Claude Code CLI** (`claude`) installed
> and signed in. Mother Claude reads `~/.claude` and drives the CLI; it is not a
> standalone tool. The release notes say so.

---

## Why CI does not build installers

`.github/workflows/release.yml` is kept, inert, behind two brakes:

1. **No `push:` trigger.** Pushing `v0.4.0` starts nothing. (Until v0.4.0 it
   did: a tag push started a CI build for every OS whose switch was on, and
   `RELEASE_CI_WINDOWS` was.)
2. **The `setup` job refuses the run** unless an operating system's switch
   (`RELEASE_CI_MACOS` / `RELEASE_CI_WINDOWS`, repository variables, exactly
   `true`) is on — and a manual "Run workflow" does not count as one.

Signing is also simplest locally: the Developer ID certificate is already in the
Mac's login keychain, so no `.p12` has to be base64'd into a secret.

---

## The version spots

Nothing in the build makes these agree, so one script owns all of them:

| File | Field |
|---|---|
| `package.json` | `"version"` |
| `package-lock.json` | `"version"` and `packages[""].version` |
| `src-tauri/Cargo.toml` | `[package] version` |
| `src-tauri/Cargo.lock` | the `mother-claude` package's version |
| `src-tauri/tauri.conf.json` | `"version"` — what the installer reports |

```bash
node scripts/release/set-version.mjs 0.4.0     # write all of them (a leading "v" is fine)
node scripts/release/set-version.mjs --check   # verify they agree
```

The lockfiles are in the list because the first `npm install` or `cargo build`
after a bump rewrites them otherwise, and a checkout that turns dirty halfway
through a release makes the next run of either script refuse to start.

Both release scripts run `--check --expect <tag>` **before** building:
notarizing takes minutes and cannot be taken back, and an installer that reports
the wrong version misleads everyone who already downloaded it.

---

## One-time setup

### macOS signing

You need a **Developer ID Application** certificate in your login keychain. An
"Apple Development" certificate builds fine and then fails notarization.

```bash
security find-identity -v -p codesigning     # confirm what you have
cp scripts/apple-credentials.env.example scripts/apple-credentials.env
# fill it in — the file is git-ignored
```

`APPLE_PASSWORD` is an **app-specific password** (appleid.apple.com → Sign-In and
Security → App-Specific Passwords), not your Apple ID password.

**The certificate expires on 2027-02-01.** Releases signed before then keep
opening — each carries a secure timestamp and a stapled ticket — but nothing can
be signed with it afterwards. Make a new one before then, and delete the old one
from the keychain once the new one is in: two certificates with the same name
make `codesign` refuse the name as ambiguous.

```bash
security find-certificate -c "Developer ID Application" -p | openssl x509 -noout -enddate
```

More on the certificate itself: [APPLE_SIGNING.md](APPLE_SIGNING.md).

### The updater signing key

The application updates itself, and only installs an archive signed by the key
whose public half is compiled into it (`plugins.updater.pubkey` in
`src-tauri/tauri.conf.json`, key id `43FDB9942AC8CABC`). The private half lives
at **`~/.tauri/mother-claude-updater.key`**, outside the repository, with an
empty password — the file's permissions are what protect it:

```bash
chmod 600 ~/.tauri/mother-claude-updater.key
```

**This key cannot be regenerated.** Every installed copy carries the matching
public key; a new keypair would mean none of them could accept another update.
Back it up somewhere you would still have after losing this machine.

Both machines need the same key. Copy it to the Windows desktop at
`%USERPROFILE%\.tauri\mother-claude-updater.key`. Each release script loads it,
or takes `TAURI_SIGNING_PRIVATE_KEY` from the environment.

### Windows signing

Windows installers are **unsigned** (no Authenticode). SmartScreen warns on
first run ("More info" → "Run anyway"), and the release notes say so. Fixing it
is a purchase, not a code change — the options are in
[APPLE_SIGNING.md](APPLE_SIGNING.md#windows-authenticode-optional).

### GitHub auth

The upload scripts take a token from, in order: `GH_TOKEN`; `gh auth token`;
whatever `git credential` holds. `gh` comes first because a credential that can
push code is not necessarily one that can write releases. Check a machine in
about a second, before committing to a build:

```bash
RELEASE_TAG=v0.4.0 bash scripts/release/release-upload.sh preflight
```

```powershell
pwsh scripts\release\release-upload.ps1 -Tag v0.4.0 -Preflight
```

Both build scripts run this themselves before building. `GH_REPO=owner/name`
points every script at another repository, for a rehearsal.

---

## Cutting a release

Both machines build **the same commit**, the one the tag points at; both scripts
refuse to build anything else. Everything below assumes `v0.4.0`.

### The two legs run in parallel

Step 1 happens once. Then **steps 2 and 3 run at the same time on their own
machines**; they upload to the same draft, and whichever finishes second merges
its entries into the `latest.json` the first one left. The macOS leg is the long
one (20–30 minutes: the full gate, the build, and Apple notarizing twice).

### What a finished release contains

Eight assets: three from the Mac, four from Windows, and the shared manifest.

| Asset | From | What it is |
|---|---|---|
| `Mother.Claude_<v>_aarch64.dmg` | macOS | what a person downloads |
| `Mother.Claude.app.tar.gz` | macOS | what the updater downloads |
| `Mother.Claude.app.tar.gz.sig` | macOS | its detached signature |
| `Mother.Claude_<v>_x64-setup.exe` | Windows | what a person downloads, and the updater for NSIS installs |
| `Mother.Claude_<v>_x64-setup.exe.sig` | Windows | its detached signature |
| `Mother.Claude_<v>_x64_en-US.msi` | Windows | for managed installs, and the updater for MSI installs |
| `Mother.Claude_<v>_x64_en-US.msi.sig` | Windows | its detached signature |
| `latest.json` | both | the update manifest, merged |

(The scripts turn the space in "Mother Claude" into a dot; GitHub would mangle it
otherwise.) The `.sig` files are not read by the updater — its copies are inside
`latest.json` — they let a person check a download against the public key
without trusting the manifest.

### 0. Before anything

```bash
git switch main && git pull
git status --porcelain          # must be empty; both scripts refuse a dirty tree
```

### 1. Bump, verify, commit, tag

```bash
node scripts/release/set-version.mjs 0.4.0
# the same gate the macOS leg runs:
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
npm run lint && npm test && npm run test:release
git commit -am "chore(release): v0.4.0"
git tag v0.4.0
git push origin main v0.4.0                   # harmless: no workflow runs on a tag
```

**Tag before building.** Both scripts check the version spots against the tag
and refuse to build a commit the tag does not point at.

### 2. macOS leg (on the Mac)

```bash
git checkout v0.4.0
source scripts/apple-credentials.env
RELEASE_TAG=v0.4.0 bash scripts/release/release-macos.sh
```

In order, and everything that can invalidate the release before anything
irreversible:

1. preflight — the version spots against the tag, the signing identity, a clean
   tree at the tag's commit, write access to the repository;
2. `npm ci`, then the full gate: `cargo fmt --check`, `cargo clippy
   --all-targets -D warnings`, `cargo test`, `npm run lint`, `npm test`,
   `npm run test:release`;
3. `tauri build --target aarch64-apple-darwin` — bundles the sidecar and the
   Angular app, signs with the hardened runtime, notarizes the `.app`, and signs
   the update archive;
4. checks the executable is arm64 and that the bundle carries the Agent SDK's
   native `claude` binary (the bridge cannot start a conversation without it);
5. notarizes and staples the **`.dmg`** itself — `tauri build` does not, and an
   unstapled dmg is rejected by Gatekeeper on a machine that has never seen it —
   and fails unless `stapler validate` and `spctl` both accept the result;
6. uploads the dmg, then the update archive, its `.sig` and the merged
   `latest.json` (`darwin-aarch64`).

If it stops before the last step, nothing was uploaded.

```bash
SKIP_UPLOAD=1 RELEASE_TAG=v0.4.0 bash scripts/release/release-macos.sh   # stop before GitHub
SKIP_BUILD=1  RELEASE_TAG=v0.4.0 bash scripts/release/release-macos.sh   # re-notarize/upload the built bundle
ALLOW_DIRTY=1 ...                                                        # rehearsals only
```

**Apple silicon only — why not universal, as Gitweaver is.** The bundle carries
`sidecar/node_modules`, and the Agent SDK in it ships Claude Code as a native
binary that npm picks for the machine that installed it
(`@anthropic-ai/claude-agent-sdk-darwin-arm64`). A universal or Intel app built
on this Mac would carry the arm64 binary, and every bridge conversation on an
Intel Mac would fail. Shipping Intel means deliberately installing
`claude-agent-sdk-darwin-x64` into the bundle too (a second native binary,
226 MB uncompressed, in every download) or pointing the SDK at the installed
`claude`; until then the download page says Apple silicon.

### 3. Windows leg (on the Windows desktop)

Runs at the same time as step 2.

#### First time on that machine only

| Needed | Check |
|---|---|
| Git | `git --version` |
| PowerShell 7 | `pwsh --version` (the scripts declare `#requires -Version 7.0`) |
| Node 24+ and npm | `node --version` |
| Rust, **MSVC** host | `rustup show` → default host `x86_64-pc-windows-msvc` |
| VS 2022 Build Tools, "Desktop development with C++" | `tauri build` fails at link time without it |
| WebView2 runtime | preinstalled on Windows 11 |
| GitHub CLI, logged in | `gh auth status` |

```powershell
git clone https://github.com/codertapsu/mother-claude.git   # a fresh clone, see below
cd mother-claude
rustup target add x86_64-pc-windows-msvc

# The updater signing key, copied from the Mac (~/.tauri/mother-claude-updater.key)
# by whatever means you trust. It CANNOT be regenerated.
New-Item -ItemType Directory -Force "$env:USERPROFILE\.tauri" | Out-Null
# ...copy mother-claude-updater.key into it, then:
Test-Path "$env:USERPROFILE\.tauri\mother-claude-updater.key"   # must be True

gh auth login
pwsh scripts\release\release-upload.ps1 -Tag v0.4.0 -Preflight
```

**Clone fresh rather than reusing an old checkout.** `.gitattributes` checks
every file out with LF, but only in a clone made after it was added (v0.4.0);
an older clone keeps the CRLF files Git for Windows wrote. The leg refuses such
a checkout before anything else, because the build would embed the plugin's
`#!/bin/sh` hook scripts with CRLF, and sh cannot run them.

#### Every time

```powershell
git fetch --tags origin
git checkout v0.4.0                # the same commit the Mac builds
$env:RELEASE_TAG = 'v0.4.0'
pwsh scripts\release\release-windows.ps1
```

That one command is the whole leg: it preflights the line endings, the version,
the tree and the token; runs `npm ci`, `npm run lint` and `npm run test:release`; builds the NSIS
`-setup.exe` and the `.msi` with `tauri build --target x86_64-pc-windows-msvc`;
checks the sidecar's Windows SDK binary is there and reads the MSI's
`ProductVersion` back; then uploads both installers, their `.sig` files and
their manifest entries (`windows-x86_64`, `windows-x86_64-nsis`,
`windows-x86_64-msi`).

**Its gate is smaller than the Mac's, on purpose.** The Rust test suite assumes
Unix — 48 `cfg(unix)` items, test files that use Unix-only APIs — and has never
been run on Windows; the Mac runs the full gate on the same commit. The Windows
leg runs what is known to hold there, and the build.

```powershell
pwsh scripts\release\release-windows.ps1 -SkipUpload   # stop before GitHub
pwsh scripts\release\release-windows.ps1 -SkipBuild    # re-upload the built installers
pwsh scripts\release\release-windows.ps1 -AllowDirty   # rehearsals only
```

### 4. Check the artifact before anybody else does

The scripts prove the release is signed, notarized and verifiable. They do not
prove it _runs_.

```bash
APP="src-tauri/target/aarch64-apple-darwin/release/bundle/macos/Mother Claude.app"
open "$APP"                                                    # it launches and lists sessions
vtool -show-build "$APP/Contents/MacOS/mother-claude" | grep minos
plutil -extract LSMinimumSystemVersion raw "$APP/Contents/Info.plist"
gh release view v0.4.0 --repo codertapsu/mother-claude \
  --json assets -q '.assets[] | "\(.name) \(.size) \(.state)"'
```

- **Launch it** — from the `.dmg`, on another Mac if one is available.
- **The minimum macOS** the download page promises is the binary's `minos`
  (11.0 for an arm64 build), not `LSMinimumSystemVersion` (10.15 in
  `tauri.conf.json`). Read it off the artifact.
- **Every asset's `state` is `uploaded`**, and its size matches the local file.
- On Windows: install the `-setup.exe` and launch it.

### 5. The update path

Each leg signs its update artifact and merges its entries into `latest.json`,
the one file every installed copy reads
(`releases/latest/download/latest.json`). The updater looks up
`{os}-{arch}-{installer}` first, then `{os}-{arch}`:

| Key | Points at | Used by |
|---|---|---|
| `darwin-aarch64` | `Mother.Claude.app.tar.gz` | every Mac install |
| `windows-x86_64-nsis` | `…_x64-setup.exe` | installs made by the `-setup.exe` |
| `windows-x86_64-msi` | `…_x64_en-US.msi` | installs made by the `.msi` |
| `windows-x86_64` | `…_x64-setup.exe` | an install that cannot say which it was |

Each install is updated through the installer that made it. v0.3.0's manifest
(written by tauri-action) carried the same three Windows keys; its plain
`windows-x86_64` pointed at the `.msi`, which no installed copy reads while the
two specific keys are there.

`verify` (step 6) checks each entry the way the updater does: it downloads
`latest.json` and every asset it names through the API, and verifies each
signature against its asset under the public key in `tauri.conf.json`
(`scripts/release/check-manifest.mjs`). It also checks that `version` is the
tag, that `pub_date` is RFC 3339 (the updater rejects the whole file
otherwise), that every `url` names an asset of this tag, that each uploaded
`.sig` is the manifest's signature, and — because tauri-plugin-updater 2.13
refuses it — that a signature recording the version it was made for records
this one. A finished release reads:

```text
latest.json for v0.4.0: version 0.4.0, pub_date 2026-10-13T06:00:00Z
  ok      darwin-aarch64       verifies under key 43FDB9942AC8CABC
  ok      windows-x86_64       verifies under key 43FDB9942AC8CABC
  ok      windows-x86_64-msi   verifies under key 43FDB9942AC8CABC
  ok      windows-x86_64-nsis  verifies under key 43FDB9942AC8CABC
  every platform verifies.
```

**Why names are not enough.** Rebuilding a leg uploads its archive, then its
`.sig`, then `latest.json`. Interrupted after the first, the release holds the
new archive beside the old signature: every name is present, and once published
every installed copy would download the archive, fail the signature check, and
say nothing. If a platform reads `INVALID`, re-run that leg's uploads from the
build still on disk (`SKIP_BUILD=1` / `-SkipBuild`) and verify again.

A draft is invisible to the updater, so nothing ships until step 6 — and
equally, **publishing is what pushes the update**. Anything published as the
newest release of this repository becomes the update feed for every installed
copy, so publish only app releases here.

### 6. Review and publish

```bash
RELEASE_TAG=v0.4.0 bash scripts/release/release-upload.sh verify
```

```powershell
pwsh scripts\release\release-upload.ps1 -Tag v0.4.0 -Verify
```

It exits non-zero, saying why, until all **eight** assets are there and every
platform's update verifies. Then open the draft at
<https://github.com/codertapsu/mother-claude/releases>, write the notes (what's
new; requires the Claude Code CLI; the `.dmg` is signed and notarized; the
Windows installers are unsigned → More info → Run anyway; upgrading is
Settings → Software updates), and publish.

---

## Redoing a release that has not been published

A draft has not shipped, so it can be replaced outright:

```bash
# 1. Fix, commit, push.
# 2. Move the tag. `--force-with-lease` does not work on tags; delete and re-push.
git tag -f v0.4.0
git push origin :refs/tags/v0.4.0
git push origin v0.4.0
# 3. Delete the draft (the next upload recreates it), or keep it: both legs
#    replace every asset by name, and `verify` proves each pair matches.
gh release delete v0.4.0 --repo codertapsu/mother-claude --yes
# 4. Build BOTH legs again — the two halves must come from one commit.
```

`latest.json` looks after itself: the merge drops every entry left from another
version, and refuses a signature made for one.

**When the tag may move without a rebuild.** Only when nothing compiled or
bundled changed — check it, do not assess it:

```bash
git diff --name-only <built-commit> <new-tag> |
  grep -E '^(src/|src-tauri/|sidecar/|package(-lock)?\.json|angular\.json|tsconfig)'
```

Any output at all means rebuild. Once a release is **published**, none of this
applies: cut a new version instead.

---

## Uploading something by hand

Both legs use the same helper, which creates the draft if it does not exist and
**replaces** a same-named asset rather than duplicating it:

```bash
RELEASE_TAG=v0.4.0 bash scripts/release/release-upload.sh upload path/to/file
```

```powershell
pwsh scripts\release\release-upload.ps1 -Tag v0.4.0 -Upload path\to\file
```

A missing file is an **error**, not a warning.

---

## Troubleshooting

**"the version spots disagree"** — `node scripts/release/set-version.mjs --check`
shows which. Bump them all with the script; never edit one by hand.

**"'Developer ID Application: …' is not in the login keychain"** — the script
prints the identities you do have. `APPLE_SIGNING_IDENTITY` must match one
exactly, team ID included.

**Notarization fails** — check `APPLE_TEAM_ID`, `APPLE_ID`, and that
`APPLE_PASSWORD` is app-specific. Get the detail with
`xcrun notarytool log <submission-id> --apple-id … --team-id …`.

**Notarization fails with HTTP 403, "A required agreement is missing or has
expired"** — Apple updated its terms; the account holder must accept them at
<https://developer.apple.com/account> and in App Store Connect → Agreements (it
can take twenty minutes to clear). Probe before re-running a long leg:

```bash
source scripts/apple-credentials.env
xcrun notarytool history --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" --password "$APPLE_PASSWORD"
```

**"the .dmg is not stapled"** — usually transient; re-run with `SKIP_BUILD=1`. Do
not ship it unstapled.

**`verify` reports a platform `INVALID`** — its archive and the signature in
`latest.json` are not from the same build, or the signature records another
version. Re-run that leg with `SKIP_BUILD=1` / `-SkipBuild`; if it persists,
rebuild the leg.

**"the built MSI reports ProductVersion X but the tag says Y"** — a stale bundle
got reused. Drop `-SkipBuild`.

**"file(s) in this checkout have Windows (CRLF) line endings"** — a clone from
before `.gitattributes`. Clone again, or re-check it out in place (this discards
uncommitted changes): `git rm -r -q --cached . ; git reset --hard`.

**PowerShell refuses to run the script** ("running scripts is disabled on this
system") — the execution policy is `Restricted`. Run it as
`pwsh -ExecutionPolicy Bypass -File scripts\release\release-windows.ps1`, or
once: `Set-ExecutionPolicy -Scope CurrentUser RemoteSigned`.

**`EALLOWSCRIPTS` / "--allow-scripts is not allowed in project-scoped
installs"** — npm 12 refusing the `allow-scripts` from `~/.npmrc` when it
reaches a nested npm through the environment. `npm run sidecar:*` goes through
`scripts/sidecar-npm.mjs`, which drops it; a new script that runs npm for
`sidecar/` must do the same.

**`error calling dlltool 'dlltool.exe': program not found`** on Windows — the
Rust toolchain is the GNU build, not MSVC:

```powershell
rustup set default-host x86_64-pc-windows-msvc
rustup toolchain install stable
```

**A frontend fix is missing from the packaged app** — Tauri can re-bundle a
stale embedded frontend when only the frontend changed. Run
`cargo clean --manifest-path src-tauri/Cargo.toml -p mother-claude` and build
again.

**Uploads fail with 403 or 404** — the token cannot write to the repository.
Run `preflight`; prefer `gh auth login`.
