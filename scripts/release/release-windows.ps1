#requires -Version 7.0
<#
.SYNOPSIS
  One command to cut the Windows half of a Mother Claude release on this
  desktop: preflight, build, verify, upload.

.DESCRIPTION
  Artifacts are built locally by design, not in CI (docs/RELEASING.md). This leg
  produces the NSIS `-setup.exe` and the `.msi`, signs both for the updater, and
  uploads them to the same draft release the macOS leg uses. Order between the
  two machines does not matter.

  Windows installers are currently UNSIGNED (no Authenticode). SmartScreen will
  warn on first run ("More info" -> "Run anyway"); the release notes say so.

  THE GATE HERE IS SMALLER THAN THE MAC'S, ON PURPOSE. The Rust test suite and
  clippy's test targets assume Unix (48 `cfg(unix)` items, test files using
  Unix-only APIs) and have never been run on Windows; the macOS leg runs the
  full gate on the same commit. This leg runs what is known to hold here: a
  clean `npm ci`, ESLint, the release tooling's own tests, and the build.

  x86_64 only. The bundle carries sidecar/node_modules, and the Agent SDK in it
  ships Claude Code as a native binary npm picks for the machine that installed
  it (@anthropic-ai/claude-agent-sdk-win32-x64 here) — an installer for another
  architecture built on this machine would carry the wrong one.

.PARAMETER Tag
  Release tag, e.g. v0.4.0. Defaults to $env:RELEASE_TAG.

.PARAMETER SkipBuild
  Reuse the installers already under src-tauri/target/x86_64-pc-windows-msvc/release/bundle.

.PARAMETER SkipUpload
  Build and verify, then stop before touching GitHub.

.PARAMETER AllowDirty
  Permit building uncommitted work, or a commit other than the tag's
  (rehearsals only; not reproducible).

.EXAMPLE
  $env:RELEASE_TAG = "v0.4.0"
  pwsh scripts/release/release-windows.ps1
#>
[CmdletBinding()]
param(
  [string]$Tag = $env:RELEASE_TAG,
  [switch]$SkipBuild,
  [switch]$SkipUpload,
  [switch]$AllowDirty
)

$ErrorActionPreference = 'Stop'
# Exit codes are checked by hand after every native call that matters. A
# profile that turns this on would make `git rev-parse --verify` on a missing
# tag throw a raw error instead of the message below.
$PSNativeCommandUseErrorActionPreference = $false
Set-Location (Resolve-Path (Join-Path $PSScriptRoot '..\..'))

if (-not $Tag) { throw 'Set RELEASE_TAG (e.g. $env:RELEASE_TAG = "v0.4.0") or pass -Tag.' }
if (-not $IsWindows) { throw 'This is the Windows leg; run scripts/release/release-macos.sh on the Mac.' }

$Target = 'x86_64-pc-windows-msvc'
$SdkPackage = 'claude-agent-sdk-win32-x64'

# The updater signing key. `bundle.createUpdaterArtifacts` is on, so the build
# signs both installers and stops with "A public key has been found, but no
# private key" without this. Same key as the Mac uses: one keypair per
# application, because the public half is compiled into every installed copy.
if (-not $env:TAURI_SIGNING_PRIVATE_KEY) {
  $updaterKey = Join-Path $env:USERPROFILE '.tauri\mother-claude-updater.key'
  if (-not (Test-Path $updaterKey)) {
    throw @"
No updater signing key at $updaterKey, and TAURI_SIGNING_PRIVATE_KEY is unset.
Copy it from the Mac (~/.tauri/mother-claude-updater.key) — it cannot be
regenerated, because the public half is already compiled into every installed
copy. See docs/RELEASING.md, 'The updater signing key'.
"@
  }
  $env:TAURI_SIGNING_PRIVATE_KEY = Get-Content -Raw $updaterKey
}
if ($null -eq $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD) { $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = '' }

# ---------------------------------------------------------------------------
# 1. Preflight. Everything cheap that can invalidate the release runs BEFORE the
#    build, so a wrong version number costs seconds rather than a full rebuild.
# ---------------------------------------------------------------------------
Write-Host '==> preflight: the checkout has LF line endings'
# .gitattributes checks everything out with LF, but only in a clone made after
# it was added. An older clone keeps CRLF files, and the build embeds the
# plugin's `#!/bin/sh` hook scripts byte for byte — an installer that would
# pass every other check here and ship hooks sh cannot run.
$crlf = @(git ls-files --eol | Where-Object { $_ -match '\sw/crlf\s' })
if ($LASTEXITCODE -ne 0) { throw 'git ls-files failed — is this a git checkout?' }
if ($crlf.Count -gt 0) {
  $crlf | Select-Object -First 5 | ForEach-Object { Write-Host "      $_" }
  throw @"
$($crlf.Count) file(s) in this checkout have Windows (CRLF) line endings. Clone the
repository again (a clone now checks out LF), or re-check it out in place —
which discards uncommitted changes:
  git rm -r -q --cached . ; git reset --hard
"@
}

Write-Host "==> preflight: the version spots agree, and match $Tag"
node scripts/release/set-version.mjs --check --expect $Tag
if ($LASTEXITCODE -ne 0) { throw 'Version check failed — bump before building (see above).' }

Write-Host "==> preflight: the working tree is clean, and is $Tag"
# A release built from uncommitted work, or from a commit other than the tag's,
# cannot be reproduced from its tag, and nobody can tell later which of the two
# the users actually got. Both legs must build the tag's commit.
$dirty = git status --porcelain
# Checked, because a `git` that failed prints nothing — which reads as clean.
if ($LASTEXITCODE -ne 0) { throw 'git status failed — is this a git checkout?' }
$tagCommit = git rev-parse -q --verify "refs/tags/$Tag^{commit}" 2>$null
$headCommit = git rev-parse HEAD
if ($dirty -or $tagCommit -ne $headCommit) {
  if ($AllowDirty) {
    Write-Warning "-AllowDirty was passed — this build is not reproducible from $Tag"
  }
  elseif (-not $tagCommit) {
    throw "There is no tag $Tag here. Fetch it first: git fetch --tags; git checkout $Tag"
  }
  elseif ($tagCommit -ne $headCommit) {
    throw "HEAD is not $Tag. Build the tag, not whatever a branch has moved to: git checkout $Tag"
  }
  else {
    $dirty | ForEach-Object { Write-Host "      $_" }
    throw 'Uncommitted changes. Commit them, or pass -AllowDirty if you mean it.'
  }
}

# Before the build, not after it. A token that cannot write to the release
# repository is a two-second check and a twenty-minute wasted build.
if (-not $SkipUpload) {
  Write-Host '==> preflight: this machine can publish to the release repository'
  & (Join-Path $PSScriptRoot 'release-upload.ps1') -Tag $Tag -Preflight
}

# Tauri writes to target/<triple>/release/bundle once a target is named. Naming
# it always keeps this build's installers apart from anything a plain
# `tauri build` left behind.
$bundle = "src-tauri/target/$Target/release/bundle"

if ($SkipBuild) {
  Write-Host '==> -SkipBuild — reusing the existing bundle (it was gated when it was built)'
}
else {
  Write-Host "==> preflight: the Rust target $Target is installed"
  $installed = (rustup target list --installed) -split "`n" | ForEach-Object { $_.Trim() }
  if ($installed -notcontains $Target) {
    throw "The Rust target $Target is not installed. Fix: rustup target add $Target"
  }

  # The dependencies the lockfiles name, exactly — not whatever node_modules
  # happens to hold from the last time this machine built anything.
  Write-Host '==> install: npm ci'
  npm ci
  if ($LASTEXITCODE -ne 0) { throw 'npm ci failed.' }

  Write-Host '==> the quality gate (the full one ran on the Mac; see the header)'
  npm run lint
  if ($LASTEXITCODE -ne 0) { throw 'npm run lint failed.' }
  npm run test:release
  if ($LASTEXITCODE -ne 0) { throw 'The release tooling tests failed.' }

  # Delete the previous installers first. A failed build leaves the last good
  # one in place, and uploading that under a new tag ships a stale binary under
  # a version number it does not contain.
  Remove-Item -Recurse -Force "$bundle/nsis", "$bundle/msi" -ErrorAction SilentlyContinue

  Write-Host "==> build (tauri build --target $Target)"
  # beforeBuildCommand bundles the sidecar (npm run sidecar:bundle) and builds
  # the Angular app; both end up inside the installers. `npx`, not `npm run
  # tauri:build -- …`: a bare `--` is PowerShell's own end-of-parameters token,
  # and not every PowerShell hands it on to a native command.
  npx tauri build --target $Target
  if ($LASTEXITCODE -ne 0) { throw 'tauri build failed — not packaging a stale artifact.' }
}

$setup = Get-ChildItem "$bundle/nsis" -Filter '*-setup.exe' -ErrorAction SilentlyContinue | Select-Object -First 1
$msi = Get-ChildItem "$bundle/msi" -Filter '*.msi' -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $setup) { throw "No -setup.exe under $bundle/nsis — the build did not produce one." }
if (-not $msi) { throw "No .msi under $bundle/msi — the build did not produce one." }
Write-Host "==> artifacts: $($setup.Name), $($msi.Name)"

# The native piece the bridge needs. The installers carry sidecar/node_modules
# as it stood after the build; without this package in it, every bridge
# conversation on Windows fails to start, and nothing else would notice.
if (-not (Test-Path "sidecar/node_modules/@anthropic-ai/$SdkPackage")) {
  throw "sidecar/node_modules has no @anthropic-ai/$SdkPackage — the installers would ship a bridge that cannot start."
}

# ---------------------------------------------------------------------------
# 2. Verify the MSI reports the version we think we built.
#    tauri writes it from tauri.conf.json, which preflight already checked — but
#    a stale bundle reused via -SkipBuild would slip past that, and an installer
#    that reports the wrong version misleads every user who inspects it.
# ---------------------------------------------------------------------------
$expected = $Tag -replace '^v', ''
try {
  $installer = New-Object -ComObject WindowsInstaller.Installer
  $db = $installer.GetType().InvokeMember('OpenDatabase', 'InvokeMethod', $null, $installer, @($msi.FullName, 0))
  $view = $db.GetType().InvokeMember('OpenView', 'InvokeMethod', $null, $db,
    @("SELECT Value FROM Property WHERE Property='ProductVersion'"))
  $view.GetType().InvokeMember('Execute', 'InvokeMethod', $null, $view, $null) | Out-Null
  $record = $view.GetType().InvokeMember('Fetch', 'InvokeMethod', $null, $view, $null)
  $msiVersion = if ($record) { $record.GetType().InvokeMember('StringData', 'GetProperty', $null, $record, @(1)) } else { $null }
}
catch {
  # Warn, never abort: a COM read that is unavailable must not block an
  # otherwise-good release. Preflight already validated tauri.conf.json.
  Write-Warning "could not read the MSI ProductVersion ($($_.Exception.Message)); preflight already checked tauri.conf.json."
  $msiVersion = $null
}

if ($msiVersion -and $msiVersion -ne $expected) {
  throw "The built MSI reports ProductVersion '$msiVersion' but the tag says '$expected' — that is a stale build."
}
elseif ($msiVersion) { Write-Host "    MSI ProductVersion $msiVersion OK" }

if ($SkipUpload) {
  Write-Host '==> -SkipUpload — stopping before upload.'
  Write-Host "    Built: $($setup.FullName)"
  Write-Host "           $($msi.FullName)"
  return
}

# ---------------------------------------------------------------------------
# 3. Upload to the tag's draft release, with the update manifest. The macOS leg
#    uploads to the same one.
# ---------------------------------------------------------------------------
# Invoked in-process with `&`, not as a child `pwsh`. Spawning one turns every
# argument back into a string on a command line, where an array is flattened
# and the pieces bind to whatever is positional — which is how a .msi path once
# became a Gitweaver release tag. In-process, parameters bind by name and keep
# their type.
$upload = Join-Path $PSScriptRoot 'release-upload.ps1'

foreach ($file in @($setup, $msi)) {
  if (-not (Test-Path "$($file.FullName).sig")) {
    throw "No signature at $($file.FullName).sig — check that bundle.createUpdaterArtifacts is true in src-tauri/tauri.conf.json."
  }
}

# One manifest entry per installer, each pointing at that installer: the updater
# looks up `windows-x86_64-<msi|nsis>` before `windows-x86_64`, so a copy
# installed from the .msi is updated by the .msi and one installed by the
# -setup.exe by the -setup.exe — as v0.3.0's manifest did. The plain
# `windows-x86_64` key is for an install that cannot say which it was: the
# -setup.exe, what a person downloads. Each step uploads its installer, the
# installer's .sig, and the merged latest.json.
Write-Host "==> upload the NSIS installer and its manifest entries to the draft release $Tag"
& $upload -Tag $Tag `
  -ManifestArchive $setup.FullName -ManifestSignature "$($setup.FullName).sig" `
  -ManifestPlatform 'windows-x86_64', 'windows-x86_64-nsis'

Write-Host "==> upload the MSI installer and its manifest entry to the draft release $Tag"
& $upload -Tag $Tag `
  -ManifestArchive $msi.FullName -ManifestSignature "$($msi.FullName).sig" `
  -ManifestPlatform 'windows-x86_64-msi'

Write-Host ''
Write-Host "Windows leg done for $Tag — its 4 of the 8 assets, and its 3 entries in latest.json."
Write-Host ''
Write-Host 'The macOS leg runs independently and may already have finished. Check:'
Write-Host "  pwsh scripts\release\release-upload.ps1 -Tag $Tag -Verify"
Write-Host ''
Write-Host 'When that reports all 8, write the notes and publish:'
Write-Host '  https://github.com/codertapsu/mother-claude/releases'
