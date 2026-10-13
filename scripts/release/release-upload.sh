#!/usr/bin/env bash
#
# release-upload.sh — ensure a DRAFT GitHub release exists for a tag and upload
# locally-built artifacts to it. macOS / Linux; the Windows twin is
# release-upload.ps1.
#
# Mother Claude builds its installers on the maintainer's own machines, not in
# CI (docs/RELEASING.md). Both machines upload to the SAME draft for a tag, so
# the order does not matter and neither leg has to know about the other. The
# release stays a draft until a human reviews and publishes it — an artifact
# that was never opened should not be downloadable.
#
# Usage:
#   bash scripts/release/release-upload.sh preflight                # can this token write?
#   bash scripts/release/release-upload.sh ensure                   # print the release id
#   bash scripts/release/release-upload.sh upload <file> [file...]  # ensure + upload
#   bash scripts/release/release-upload.sh verify                   # all 8 there, all verifying?
#
# Auth: GH_TOKEN if exported, else `gh auth token`, else the token
# `git credential` holds. In that order, and the order matters — see the block
# where TOKEN is set.
#
# Repo: GH_REPO if exported, else RELEASE_REPO below — never derived from
# `origin`, so a fork or a mirror remote cannot quietly receive a release. Tag:
# RELEASE_TAG (required).
set -euo pipefail

# WHY `ensure_release` SETS A GLOBAL RATHER THAN PRINTING AN ID
#
# It used to print, and every caller read it with a command substitution.
# A command substitution runs in a subshell that does not inherit `-e`, so a
# 403 from the API did not stop anything: the substitution yielded an empty
# string, the script uploaded to release id "" and exited 0.
#
# That is what happened cutting Gitweaver v0.7.0, whose release tooling this is
# — every upload returned 403 and the run reported success. A release script
# that cannot fail is worse than none, because it is believed.
#
# `shopt -s inherit_errexit` fixes exactly this and needs bash 4.4. macOS ships
# 3.2, which is the shell this runs under on the machine that cuts every
# release, and `shopt` on an unknown option is itself an error. So the fix is
# structural instead: no subshell, and the failure propagates on its own.

cd "$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

TAG="${RELEASE_TAG:?set RELEASE_TAG (e.g. RELEASE_TAG=v0.2.0)}"

# A tag that is not a tag stops here rather than being created. `ensure_release`
# will make a release for any string it is handed, so a mis-set variable becomes
# a junk draft in a public repository instead of an error. The Windows twin
# learned this the expensive way.
case "$TAG" in
  v[0-9]*) ;;
  *) echo "error: RELEASE_TAG '$TAG' does not look like a version tag (expected v0.2.0 or similar)." >&2; exit 1 ;;
esac

# Where published installers live: the public source repository itself, whose
# releases/latest/download/latest.json every installed copy polls for updates
# (plugins.updater.endpoints in src-tauri/tauri.conf.json). Spelled out rather
# than read from `origin`, which a fork or a mirror remote can point elsewhere.
RELEASE_REPO='codertapsu/mother-claude'

REPO="${GH_REPO:-$RELEASE_REPO}"
case "$REPO" in
  */*) ;;
  *) echo "error: could not work out the GitHub repo (got '${REPO:-<empty>}'). Set GH_REPO=owner/name." >&2; exit 1 ;;
esac

# Token, in order of preference.
#
# `gh auth token` first, because `git credential` holds whatever token was used
# to clone — which may be a fine-grained PAT that can read and push code but has
# no `contents: write` on releases. That is how all of Gitweaver v0.7.0's
# uploads returned 403.
#
# `git credential` stays as the fallback so a machine without the `gh` CLI can
# still release.
if [ -n "${GH_TOKEN:-}" ]; then
  TOKEN="$GH_TOKEN"
  TOKEN_SOURCE='GH_TOKEN'
elif TOKEN="$(gh auth token 2>/dev/null)" && [ -n "$TOKEN" ]; then
  TOKEN_SOURCE='gh auth token'
else
  TOKEN="$(printf 'protocol=https\nhost=github.com\n\n' | git credential fill 2>/dev/null | sed -n 's/^password=//p')"
  TOKEN_SOURCE='git credential'
fi
[ -n "$TOKEN" ] || {
  echo "error: no GitHub token. Run 'gh auth login', or export GH_TOKEN." >&2
  exit 1
}

RELEASE_NAME="Mother Claude ${TAG}"
export TAG REPO RELEASE_NAME

api() { # api METHOD PATH [curl args...]
  local method="$1" path="$2"
  shift 2
  curl -fsSL -X "$method" \
    -H "Authorization: Bearer $TOKEN" \
    -H "Accept: application/vnd.github+json" \
    -H "Content-Type: application/json" \
    "https://api.github.com${path}" "$@"
}

# Sets RELEASE_ID. Does not print it — see the note at the top of the file.
RELEASE_ID=''
ensure_release() {
  local id body list
  # The response is captured before it is parsed. Piping `api` straight into
  # node means a 401 hands node an empty stdin, and what the maintainer sees is
  # a `SyntaxError: Unexpected end of JSON input` stack trace rather than
  # "your token is wrong".
  list="$(api GET "/repos/$REPO/releases?per_page=100")" || {
    echo "error: could not list the releases on $REPO — the API call failed." >&2
    echo "       Almost always the token. Check it with:" >&2
    echo "         RELEASE_TAG=$TAG bash scripts/release/release-upload.sh preflight" >&2
    return 1
  }
  id="$(printf '%s' "$list" | node -e "
const all = JSON.parse(require('fs').readFileSync(0, 'utf8'));
process.stdout.write(String(all.find((r) => r.tag_name === process.env.TAG)?.id ?? ''));")"

  if [ -z "$id" ]; then
    body="$(node -e "process.stdout.write(JSON.stringify({
      tag_name: process.env.TAG,
      name: process.env.RELEASE_NAME,
      draft: true,
      prerelease: false,
    }));")"
    id="$(api POST "/repos/$REPO/releases" -d "$body" | node -e "
process.stdout.write(String(JSON.parse(require('fs').readFileSync(0, 'utf8')).id));")"
    echo "created draft release $TAG (id $id)" >&2
  fi

  # Belt and braces for the shell that cannot do `inherit_errexit`: an id that
  # is not a number means something upstream failed quietly, and uploading to
  # it would 404 for every asset while looking like progress.
  case "$id" in
    '' | *[!0-9]*)
      echo "error: no usable release id for $TAG (got '${id}')." >&2
      echo "       The API call above failed — check the token's access to $REPO." >&2
      return 1
      ;;
  esac

  RELEASE_ID="$id"
}

# Prints the id of the asset called NAME on the release, or nothing. The listing
# is captured before it is parsed, for the reason `ensure_release` gives.
asset_id() { # asset_id RELEASE_ID NAME
  local list
  list="$(api GET "/repos/$REPO/releases/$1/assets?per_page=100")" || {
    echo "error: could not list the assets of release $1 on $REPO." >&2
    return 1
  }
  printf '%s' "$list" | NAME="$2" node -e "
const all = JSON.parse(require('fs').readFileSync(0, 'utf8'));
process.stdout.write(String(all.find((a) => a.name === process.env.NAME)?.id ?? ''));"
}

upload_one() { # upload_one RELEASE_ID FILE
  local rid="$1" file="$2" name aid enc
  # GitHub rejects raw spaces in the asset-name query. Normalizing to '.' keeps
  # the URL valid and makes the macOS and Windows asset names consistent.
  name="$(basename "$file" | tr ' ' '.')"

  # Replace rather than duplicate: re-running a release after fixing something
  # must not leave the previous, broken artifact sitting next to the new one.
  aid="$(asset_id "$rid" "$name")"
  [ -z "$aid" ] || api DELETE "/repos/$REPO/releases/assets/$aid" >/dev/null

  enc="$(NAME="$name" node -e "process.stdout.write(encodeURIComponent(process.env.NAME));")"
  curl -fsSL -X POST \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/octet-stream" \
    --data-binary @"$file" \
    "https://uploads.github.com/repos/$REPO/releases/$rid/assets?name=$enc" >/dev/null

  echo "  uploaded $name ($(du -h "$file" | cut -f1 | tr -d ' '))"
}

# Fetches the manifest already attached to the draft, or nothing when this is
# the first machine to finish. Its output is piped into update-manifest.mjs,
# which is what keeps the other platform's entry alive.
fetch_manifest() { # fetch_manifest RELEASE_ID
  local rid="$1" aid
  aid="$(asset_id "$rid" latest.json)"
  [ -n "$aid" ] || return 0
  curl -fsSL -H "Authorization: Bearer $TOKEN" -H "Accept: application/octet-stream" \
    "https://api.github.com/repos/$REPO/releases/assets/$aid"
}

# Every asset a complete release has, and which machine makes it.
#
# Eight, and the count is the point: the macOS leg makes three (.dmg, update
# archive, its .sig), the Windows leg four (both installers and their .sig), and
# the manifest is shared — so "four" means one machine has not finished and
# "seven" means something failed quietly. Checked by `verify` before anybody
# publishes.
EXPECTED_ASSETS='.dmg .app.tar.gz .app.tar.gz.sig -setup.exe -setup.exe.sig .msi .msi.sig latest.json'
EXPECTED_COUNT=8

# Every platform latest.json must carry. The macOS leg writes darwin-aarch64;
# the Windows leg writes one entry per installer, because the updater looks up
# `windows-x86_64-<msi|nsis>` before `windows-x86_64`, and an install made from
# the .msi must be updated by the .msi. `verify` checks each one's signature,
# not its name.
EXPECTED_PLATFORMS='darwin-aarch64,windows-x86_64,windows-x86_64-msi,windows-x86_64-nsis'

# Proves the token can write to the release repo, in about a second.
#
# Run before building rather than after, because the alternative is finding out
# that a token is read-only at the end of a twenty-minute build — which is how
# Gitweaver found it. Reads the repository and asks for the permissions the
# token actually has, rather than trying a write and leaving debris behind.
preflight() {
  local perms
  echo "auth: $TOKEN_SOURCE -> $REPO"
  perms="$(api GET "/repos/$REPO" 2>/dev/null | node -e "
const repo = JSON.parse(require('fs').readFileSync(0, 'utf8'));
const p = repo.permissions ?? {};
process.stdout.write(p.push || p.admin || p.maintain ? 'write' : 'read');" 2>/dev/null || true)"

  if [ "$perms" != "write" ]; then
    cat >&2 <<EOF
error: this token cannot write to $REPO (it has: ${perms:-no access}).

  A token that can read the repository but not write to it returns 403 on
  every upload — and a release is only as good as its uploads.

  Fix, in order of preference:
    gh auth login                      # then re-run; this script prefers it
    export GH_TOKEN=<a token with 'contents: write' on $REPO>
EOF
    exit 1
  fi
  echo "auth: write access to $REPO confirmed"
}

case "${1:-}" in
  preflight)
    preflight
    ;;
  verify)
    # Looked up, never created. `ensure_release` would make a draft for a tag
    # that has none — so a typo in RELEASE_TAG would leave an empty release in
    # a public repository, from a command whose whole job is to look.
    list="$(api GET "/repos/$REPO/releases?per_page=100")" || {
      echo "error: could not list the releases on $REPO — check the token with 'preflight'." >&2
      exit 1
    }
    rid="$(printf '%s' "$list" | node -e "
const all = JSON.parse(require('fs').readFileSync(0, 'utf8'));
process.stdout.write(String(all.find((r) => r.tag_name === process.env.TAG)?.id ?? ''));")"
    if [ -z "$rid" ]; then
      echo "error: no release for $TAG on $REPO — neither leg has uploaded anything yet." >&2
      exit 1
    fi
    assets="$(api GET "/repos/$REPO/releases/$rid/assets?per_page=100")" || {
      echo "error: could not list the assets of release $rid on $REPO." >&2
      exit 1
    }
    names="$(printf '%s' "$assets" | node -e "
const all = JSON.parse(require('fs').readFileSync(0, 'utf8'));
process.stdout.write(all.map((a) => a.name).join('\n'));")"

    echo "release $TAG (id $rid)"
    missing=0
    for suffix in $EXPECTED_ASSETS; do
      # A glob `case`, not `grep`: the suffixes contain dots, and in a regex a
      # dot matches anything — `.msi` would be satisfied by `x-setup.exe` if
      # the name happened to line up. Here the suffix is literal.
      found=0
      while IFS= read -r name; do
        case "$name" in *"$suffix") found=1; break ;; esac
      done <<EOF
$names
EOF
      if [ "$found" -eq 1 ]; then
        printf '  ok      *%s\n' "$suffix"
      else
        printf '  MISSING *%s\n' "$suffix"
        missing=$((missing + 1))
      fi
    done

    count="$(printf '%s\n' "$names" | grep -c . || true)"
    echo "  $count asset(s) present; $EXPECTED_COUNT expected"

    # Then what the names cannot say: whether the update would install. Every
    # name can be right while latest.json carries the signature of an archive
    # that has since been replaced — an interrupted manifest step leaves
    # exactly that, and this command used to call it complete. This downloads
    # the manifest and what it names, and checks each signature the way the
    # updater does. It runs even when assets are missing, so a half-finished
    # release shows which half verifies.
    echo
    manifest_ok=1
    GH_TOKEN="$TOKEN" node scripts/release/check-manifest.mjs \
      --tag "$TAG" --repo "$REPO" --release "$rid" --require "$EXPECTED_PLATFORMS" || manifest_ok=0

    if [ "$missing" -gt 0 ] || [ "$manifest_ok" -eq 0 ]; then
      echo >&2
      [ "$missing" -eq 0 ] || echo "error: $missing expected asset(s) missing — the other leg has not finished, or one of them failed." >&2
      [ "$manifest_ok" -eq 1 ] || echo "error: latest.json does not verify for every platform (see above) — published now, some or all users could not install the update." >&2
      echo "       Do not publish yet." >&2
      exit 1
    fi
    echo "  complete: both legs have uploaded, and every update verifies."
    ;;
  ensure)
    ensure_release
    printf '%s\n' "$RELEASE_ID"
    ;;
  manifest)
    # manifest <archive> <sig-file> <platform> [<platform>...]
    shift
    archive="${1:?usage: release-upload.sh manifest <archive> <sig-file> <platform>...}"
    sigfile="${2:?usage: release-upload.sh manifest <archive> <sig-file> <platform>...}"
    shift 2
    [ "$#" -gt 0 ] || { echo "error: name at least one platform" >&2; exit 1; }
    [ -f "$archive" ] || { echo "error: no such archive: $archive" >&2; exit 1; }
    [ -f "$sigfile" ] || { echo "error: no such signature: $sigfile" >&2; exit 1; }

    ensure_release
    rid="$RELEASE_ID"
    asset="$(basename "$archive" | tr ' ' '.')"

    platform_args=()
    for p in "$@"; do platform_args+=(--platform "$p"); done

    merged="$(mktemp -t mother-claude-latest)"
    # Merge into whatever the other machine already published, never over it.
    fetch_manifest "$rid" | node scripts/release/update-manifest.mjs \
      --version "${TAG#v}" \
      --tag "$TAG" \
      --now "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
      --url "https://github.com/$REPO/releases/download/$TAG/$asset" \
      --signature "$(cat "$sigfile")" \
      --archive "$archive" \
      "${platform_args[@]}" > "$merged"

    manifest_dir="$(dirname "$merged")"
    mv "$merged" "$manifest_dir/latest.json"
    upload_one "$rid" "$archive"
    # The detached signature, as an asset of its own.
    #
    # The updater does not need it — its copy is inside latest.json, which is
    # what the client reads. This is for a person: it is what lets somebody who
    # downloaded the archive check it against the public key without trusting
    # the manifest, or us. Costs a few hundred bytes.
    upload_one "$rid" "$sigfile"
    upload_one "$rid" "$manifest_dir/latest.json"
    rm -f "$manifest_dir/latest.json"
    echo "  manifest lists: $(printf '%s ' "$@")"
    ;;
  upload)
    shift
    [ "$#" -gt 0 ] || {
      echo "usage: release-upload.sh upload <file> [file...]" >&2
      exit 1
    }
    ensure_release
    rid="$RELEASE_ID"
    echo "release $TAG -> id $rid"
    missing=0
    for f in "$@"; do
      if [ -f "$f" ]; then
        upload_one "$rid" "$f"
      else
        echo "  MISSING: $f" >&2
        missing=1
      fi
    done
    # A missing artifact is a failed release, not a warning. Exiting 0 here once
    # let a "successful" run publish a draft with no installer in it.
    [ "$missing" -eq 0 ] || {
      echo "error: one or more artifacts were missing (see above)." >&2
      exit 1
    }
    echo "done. Review and publish the draft: https://github.com/$REPO/releases"
    ;;
  *)
    cat >&2 <<'EOF'
usage: release-upload.sh <command>

  preflight                      check the token can write to the release repo
  ensure                         print the draft release id, creating it if needed
  upload <file> [file...]        ensure, then upload
  manifest <archive> <sig> <platform>...
                                 merge this platform into latest.json and upload
                                 the archive, its signature, and the manifest
  verify                         check the draft has all 8 assets, and that every
                                 platform's update verifies, before publishing

  RELEASE_TAG is required for all of them.
EOF
    exit 1
    ;;
esac
