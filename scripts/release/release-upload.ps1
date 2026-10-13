#requires -Version 7.0
<#
.SYNOPSIS
  Ensure a DRAFT GitHub release exists for a tag and upload locally-built
  artifacts to it. Windows twin of scripts/release/release-upload.sh.

.DESCRIPTION
  Mother Claude builds its installers on the maintainer's own machines, not in
  CI (docs/RELEASING.md). Both machines upload to the SAME draft for a tag, so
  the order does not matter and neither leg has to know about the other. The
  release stays a draft until a human reviews and publishes it.

  Auth: $env:GH_TOKEN if set, else `gh auth token`, else the token
  `git credential` holds. In that order, and the order matters — see the block
  where the token is read.

  Repo: $env:GH_REPO if set, else $ReleaseRepo below — never derived from
  `origin`, so a fork or a mirror remote cannot quietly receive a release. Tag:
  $env:RELEASE_TAG or -Tag.

.EXAMPLE
  $env:RELEASE_TAG = "v0.4.0"
  pwsh scripts/release/release-upload.ps1 -Verify
#>
[CmdletBinding()]
param(
  [string]$Tag = $env:RELEASE_TAG,
  [switch]$Ensure,
  # Check the token can write to the release repo. Touches nothing.
  [switch]$Preflight,
  # Check the draft has all eight assets, and that every platform's update
  # verifies. Touches nothing.
  [switch]$Verify,
  # Publishing the updater manifest. Three named parameters rather than one
  # array: an array does not survive being handed to a child `pwsh` process,
  # where it is flattened into separate arguments that then bind to whatever
  # happens to be positional.
  [string]$ManifestArchive,
  [string]$ManifestSignature,
  [string[]]$ManifestPlatform,
  # The ONLY positional parameter, and explicitly so. Without a Position here,
  # PowerShell assigns implicit positions to every parameter in declaration
  # order, and the first argument that does not bind by name becomes -Tag.
  # That is not hypothetical: `-Upload <setup> <msi>` bound the msi path to
  # -Tag and created a draft release named after it.
  [Parameter(Position = 0, ValueFromRemainingArguments = $true)][string[]]$Upload
)

$ErrorActionPreference = 'Stop'
# Exit codes are checked by hand after every native call that matters. A
# profile that turns this on would make `git rev-parse --verify` on a missing
# tag throw a raw error instead of the message below.
$PSNativeCommandUseErrorActionPreference = $false
Set-Location (Resolve-Path (Join-Path $PSScriptRoot '..\..'))

if (-not $Tag) { throw 'Set RELEASE_TAG (e.g. $env:RELEASE_TAG = "v0.2.0") or pass -Tag.' }

# A tag that is not a tag must stop here, not be created. `Confirm-Release` will
# make a release for any string it is given, so a mis-bound argument becomes a
# junk draft in a public repository rather than an error — which is exactly what
# happened, twice, before the parameter binding above was made explicit.
if ($Tag -notmatch '^v[0-9]') {
  throw "RELEASE_TAG '$Tag' does not look like a version tag (expected v0.2.0 or similar)."
}

# Where published installers live: the public source repository itself, whose
# releases/latest/download/latest.json every installed copy polls for updates
# (plugins.updater.endpoints in src-tauri/tauri.conf.json). Spelled out rather
# than read from `origin`, which a fork or a mirror remote can point elsewhere.
$ReleaseRepo = 'codertapsu/mother-claude'

$Repo = if ($env:GH_REPO) { $env:GH_REPO } else { $ReleaseRepo }
if (-not $Repo -or $Repo -notmatch '/') {
  throw "Could not work out the GitHub repo (got '$Repo'). Set `$env:GH_REPO = 'owner/name'."
}

# Token, in order of preference.
#
# `gh auth token` before `git credential`, because `git credential` holds
# whatever token was used to clone — which may be a fine-grained PAT that can
# read and push code but has no `contents: write` on releases. That is how all
# of Gitweaver v0.7.0's uploads failed while its script reported success.
#
# `git credential` stays as the last resort so a machine with no `gh` CLI can
# still publish.
$script:TokenSource = ''
function Get-GhToken {
  if ($env:GH_TOKEN) { $script:TokenSource = 'GH_TOKEN'; return $env:GH_TOKEN }

  # Guarded twice. `$ErrorActionPreference = 'Stop'` is already in force, so on
  # a machine with no `gh` the bare call would throw CommandNotFoundException
  # and take the script down — defeating the `git credential` fallback below,
  # which exists precisely for that machine.
  if (Get-Command gh -ErrorAction SilentlyContinue) {
    try {
      $fromGh = (gh auth token 2>$null)
      if ($LASTEXITCODE -eq 0 -and $fromGh) {
        $script:TokenSource = 'gh auth token'
        return $fromGh.Trim()
      }
    }
    catch {
      # `gh` is present but not logged in, or broken. Fall through.
    }
  }

  $out = "protocol=https`nhost=github.com`n`n" | git credential fill 2>$null
  $line = ($out | Select-String '^password=' | Select-Object -First 1)
  if ($line) {
    $script:TokenSource = 'git credential'
    return ($line.Line -replace '^password=', '')
  }
  return $null
}

$Token = Get-GhToken
if (-not $Token) { throw "No GitHub token. Run 'gh auth login', or set `$env:GH_TOKEN." }
$Headers = @{ Authorization = "Bearer $Token"; Accept = 'application/vnd.github+json' }

function Confirm-Release {
  $rels = Invoke-RestMethod -Headers $Headers "https://api.github.com/repos/$Repo/releases?per_page=100"
  $existing = $rels | Where-Object { $_.tag_name -eq $Tag } | Select-Object -First 1
  if ($existing) { return $existing.id }

  $body = @{ tag_name = $Tag; name = "Mother Claude $Tag"; draft = $true; prerelease = $false } | ConvertTo-Json
  $created = Invoke-RestMethod -Method Post -Headers $Headers -ContentType 'application/json' -Body $body `
    "https://api.github.com/repos/$Repo/releases"
  Write-Host "created draft release $Tag (id $($created.id))"
  return $created.id
}

function Send-Asset($ReleaseId, $File) {
  # GitHub rejects raw spaces in the asset-name query. Normalizing to '.' keeps
  # the URL valid and the macOS/Windows asset names consistent.
  $name = (Split-Path $File -Leaf) -replace ' ', '.'

  # Replace rather than duplicate: re-running a release after a fix must not
  # leave the previous, broken artifact next to the new one.
  $assets = Invoke-RestMethod -Headers $Headers "https://api.github.com/repos/$Repo/releases/$ReleaseId/assets?per_page=100"
  $existing = $assets | Where-Object { $_.name -eq $name } | Select-Object -First 1
  if ($existing) {
    Invoke-RestMethod -Method Delete -Headers $Headers `
      "https://api.github.com/repos/$Repo/releases/assets/$($existing.id)" | Out-Null
  }

  $enc = [uri]::EscapeDataString($name)
  Invoke-RestMethod -Method Post -Headers $Headers -ContentType 'application/octet-stream' -InFile $File `
    "https://uploads.github.com/repos/$Repo/releases/$ReleaseId/assets?name=$enc" | Out-Null

  $mb = [math]::Round((Get-Item $File).Length / 1MB, 1)
  Write-Host "  uploaded $name ($mb MB)"
}

# Every asset a complete release has. Eight, and the count is the point: the
# macOS leg makes three (.dmg, update archive, its .sig), this leg four (both
# installers and their .sig), and the manifest is shared — so "four" means one
# machine has not finished and "seven" means something failed quietly.
$ExpectedAssets = @('.dmg', '.app.tar.gz', '.app.tar.gz.sig', '-setup.exe', '-setup.exe.sig', '.msi', '.msi.sig', 'latest.json')

# Every platform latest.json must carry. The macOS leg writes darwin-aarch64;
# this leg one entry per installer, because the updater looks up
# `windows-x86_64-<msi|nsis>` before `windows-x86_64`, and an install made from
# the .msi must be updated by the .msi. -Verify checks each one's signature,
# not its name.
$ExpectedPlatforms = @('darwin-aarch64', 'windows-x86_64', 'windows-x86_64-msi', 'windows-x86_64-nsis')

if ($Preflight) {
  # Before building, not after. The alternative is discovering a read-only
  # token at the end of a twenty-minute build, which is how Gitweaver found it.
  Write-Host "auth: $script:TokenSource -> $Repo"
  try {
    $info = Invoke-RestMethod -Headers $Headers "https://api.github.com/repos/$Repo"
  }
  catch {
    throw "Cannot reach $Repo with this token: $($_.Exception.Message)"
  }
  $p = $info.permissions
  if (-not ($p.push -or $p.admin -or $p.maintain)) {
    throw @"
This token cannot write to $Repo.

  A token that can read the repository but not write to it returns 403 on
  every upload — and a release is only as good as its uploads.

  Fix, in order of preference:
    gh auth login                       # then re-run; this script prefers it
    `$env:GH_TOKEN = '<token with contents: write on $Repo>'
"@
  }
  Write-Host "auth: write access to $Repo confirmed"
  return
}

if ($Verify) {
  # Looked up, never created. `Confirm-Release` would make a draft for a tag
  # that has none — so a typo in the tag would leave an empty release in a
  # public repository, from a command whose whole job is to look.
  $rels = Invoke-RestMethod -Headers $Headers "https://api.github.com/repos/$Repo/releases?per_page=100"
  $existing = $rels | Where-Object { $_.tag_name -eq $Tag } | Select-Object -First 1
  if (-not $existing) {
    throw "No release for $Tag on $Repo — neither leg has uploaded anything yet."
  }
  $rid = $existing.id
  $names = @((Invoke-RestMethod -Headers $Headers `
        "https://api.github.com/repos/$Repo/releases/$rid/assets?per_page=100").name)

  Write-Host "release $Tag (id $rid)"
  $missing = 0
  foreach ($suffix in $ExpectedAssets) {
    if ($names | Where-Object { $_.EndsWith($suffix, 'OrdinalIgnoreCase') }) {
      Write-Host "  ok      *$suffix"
    }
    else {
      Write-Host "  MISSING *$suffix" -ForegroundColor Red
      $missing += 1
    }
  }
  Write-Host "  $($names.Count) asset(s) present; $($ExpectedAssets.Count) expected"

  # Then what the names cannot say: whether the update would install. Every
  # name can be right while latest.json carries the signature of an archive
  # that has since been replaced — an interrupted manifest step leaves exactly
  # that, and this used to call it complete. check-manifest.mjs downloads the
  # manifest and what it names, and checks each signature the way the updater
  # does. It runs even when assets are missing, so a half-finished release
  # shows which half verifies.
  #
  # The token goes through the environment, not the command line, where any
  # process listing would show it.
  Write-Host ''
  $savedToken = $env:GH_TOKEN
  $env:GH_TOKEN = $Token
  try {
    node scripts/release/check-manifest.mjs --tag $Tag --repo $Repo --release $rid `
      --require ($ExpectedPlatforms -join ',')
    $manifestOk = ($LASTEXITCODE -eq 0)
  }
  finally {
    $env:GH_TOKEN = $savedToken
  }

  if ($missing -gt 0 -or -not $manifestOk) {
    $why = @()
    if ($missing -gt 0) { $why += "$missing expected asset(s) missing — the other leg has not finished, or one failed." }
    if (-not $manifestOk) { $why += 'latest.json does not verify for every platform (see above) — published now, some or all users could not install the update.' }
    throw (($why -join ' ') + ' Do not publish yet.')
  }
  Write-Host '  complete: both legs have uploaded, and every update verifies.'
  return
}

$rid = Confirm-Release
Write-Host "release $Tag -> id $rid"
if ($Ensure -and -not $Upload -and -not $ManifestArchive) { Write-Output $rid; return }

if ($ManifestArchive) {
  $archive = $ManifestArchive
  $sigFile = $ManifestSignature
  $platforms = $ManifestPlatform
  if (-not $sigFile) { throw 'Pass -ManifestSignature with -ManifestArchive.' }
  if (-not $platforms) { throw 'Pass at least one -ManifestPlatform with -ManifestArchive.' }
  if (-not (Test-Path $archive)) { throw "No such archive: $archive" }
  if (-not (Test-Path $sigFile)) { throw "No such signature: $sigFile" }

  # Whatever the other machine already published. Merging into it rather than
  # over it is the whole reason this is a separate step: the two halves of a
  # release are built on different machines and share one latest.json.
  #
  # Downloaded as bytes, never through Invoke-RestMethod: that parses JSON
  # itself, and for a file it cannot parse (a byte-order mark, a truncated
  # upload) it returns the raw text, which ConvertTo-Json then made into a JSON
  # *string*. The merge read that as "no manifest yet" and wrote one holding
  # only this platform — every Mac entry gone, and the leg exiting 0.
  $existing = ''
  $assets = Invoke-RestMethod -Headers $Headers "https://api.github.com/repos/$Repo/releases/$rid/assets?per_page=100"
  $current = $assets | Where-Object { $_.name -eq 'latest.json' } | Select-Object -First 1
  if ($current) {
    $download = Join-Path ([System.IO.Path]::GetTempPath()) 'mother-claude-existing-latest.json'
    Invoke-WebRequest -Headers (@{ Authorization = "Bearer $Token"; Accept = 'application/octet-stream' }) `
      -OutFile $download "https://api.github.com/repos/$Repo/releases/assets/$($current.id)"
    $existing = [System.IO.File]::ReadAllText($download)
    Remove-Item $download -ErrorAction SilentlyContinue
  }

  $assetName = (Split-Path $archive -Leaf) -replace ' ', '.'
  $nodeArgs = @(
    'scripts/release/update-manifest.mjs',
    '--version', ($Tag -replace '^v', ''),
    '--tag', $Tag,
    # InvariantCulture, because ':' in a .NET format string means "the culture's
    # time separator" — '.' on a Finnish machine — and the updater rejects the
    # whole manifest over a date that is not RFC 3339.
    '--now', [DateTime]::UtcNow.ToString("yyyy-MM-dd'T'HH:mm:ss'Z'", [cultureinfo]::InvariantCulture),
    '--url', "https://github.com/$Repo/releases/download/$Tag/$assetName",
    '--signature', (Get-Content -Raw $sigFile),
    '--archive', $archive
  )
  foreach ($p in $platforms) { $nodeArgs += @('--platform', $p) }

  $merged = Join-Path ([System.IO.Path]::GetTempPath()) 'latest.json'
  $existing | node @nodeArgs | Set-Content -NoNewline -Path $merged
  if ($LASTEXITCODE -ne 0) { throw 'Merging the update manifest failed (see above).' }

  Send-Asset $rid $archive
  # The detached signature, as an asset of its own. The updater does not need
  # it — its copy is inside latest.json — but it is what lets somebody who
  # downloaded the installer check it against the public key without trusting
  # the manifest, or us.
  Send-Asset $rid $sigFile
  Send-Asset $rid $merged
  Remove-Item $merged -ErrorAction SilentlyContinue
  Write-Host "  manifest lists: $($platforms -join ' ')"
  Write-Host "done. Review and publish the draft: https://github.com/$Repo/releases"
  return
}

$files = @()
$anyMissing = $false
foreach ($pattern in $Upload) {
  $matched = @(Get-ChildItem -Path $pattern -File -ErrorAction SilentlyContinue)
  if (-not $matched) {
    Write-Host "  MISSING: $pattern" -ForegroundColor Red
    $anyMissing = $true
  }
  $files += $matched
}

# A missing artifact is a failed release, not a warning: without this a
# "successful" run can publish a draft with no installer in it.
if ($anyMissing) { throw 'One or more artifacts were missing (see above).' }
if (-not $files) { throw 'No files matched to upload.' }

foreach ($f in $files) { Send-Asset $rid $f.FullName }
Write-Host "done. Review and publish the draft: https://github.com/$Repo/releases"
