#!/usr/bin/env bash
#
# release-upload.sh — local-first release helper (macOS / Linux).
#
# Ensures the DRAFT GitHub release for the tag exists and uploads locally-built
# installer artifacts to it, WITHOUT running CI — so cutting a release doesn't
# burn (10x-billed) macOS Actions minutes. Pairs with a local build:
#   npm ci
#   npm run tauri:build
#   bash scripts/release/release-upload.sh upload \
#     src-tauri/target/release/bundle/dmg/*.dmg
#
# Auth: the GitHub OAuth token from `git credential` (no `gh` CLI needed), or
# GH_TOKEN if exported. Repo/tag default to this project; override with
# GH_REPO / RELEASE_TAG.
#
# Usage:
#   bash scripts/release/release-upload.sh ensure                  # print RELEASE_ID
#   bash scripts/release/release-upload.sh upload <file> [file...] # ensure + upload (replacing)
set -euo pipefail

TAG="${RELEASE_TAG:-v0.1.0}"
REPO="${GH_REPO:-codertapsu/mother-claude}"
RELEASE_NAME="Mother Claude ${TAG}"

TOKEN="${GH_TOKEN:-$(printf 'protocol=https\nhost=github.com\n\n' | git credential fill 2>/dev/null | sed -n 's/^password=//p')}"
[ -n "$TOKEN" ] || { echo "error: no GitHub token (set GH_TOKEN or log in so 'git credential' has one)" >&2; exit 1; }

# api METHOD PATH [extra curl args...]
api() {
  local method="$1" path="$2"; shift 2
  curl -fsSL -X "$method" \
    -H "Authorization: Bearer $TOKEN" \
    -H "Accept: application/vnd.github+json" \
    -H "Content-Type: application/json" \
    "https://api.github.com${path}" "$@"
}

ensure_release() {
  local id
  id="$(api GET "/repos/$REPO/releases?per_page=100" | python3 -c "import sys,json,os
print(next((str(r['id']) for r in json.load(sys.stdin) if r['tag_name']==os.environ['TAG']), ''))")"
  if [ -z "$id" ]; then
    local body
    body="$(python3 -c "import json,os;print(json.dumps({'tag_name':os.environ['TAG'],'name':os.environ['RELEASE_NAME'],'draft':True,'prerelease':False}))")"
    id="$(api POST "/repos/$REPO/releases" -d "$body" | python3 -c "import sys,json;print(json.load(sys.stdin)['id'])")"
    echo "created draft release $TAG (id $id)" >&2
  fi
  printf '%s' "$id"
}

upload_one() {  # upload_one RELEASE_ID FILE
  local rid="$1" file="$2" name aid enc
  # GitHub rejects raw spaces in the asset-name query and would otherwise mangle
  # them; normalize to a clean, stable asset name (spaces → '.', matching how
  # tauri-action/GitHub name the Windows assets) so the URL is valid, re-uploads
  # replace the same asset, and the macOS/Windows names are consistent.
  name="$(basename "$file" | tr ' ' '.')"
  aid="$(api GET "/repos/$REPO/releases/$rid/assets?per_page=100" | NAME="$name" python3 -c "import sys,json,os
print(next((str(a['id']) for a in json.load(sys.stdin) if a['name']==os.environ['NAME']), ''))")"
  if [ -n "$aid" ]; then api DELETE "/repos/$REPO/releases/assets/$aid" >/dev/null; fi
  # URL-encode the name for the query param (covers any remaining unsafe chars).
  enc="$(printf '%s' "$name" | python3 -c "import sys,urllib.parse;print(urllib.parse.quote(sys.stdin.read(), safe=''))")"
  curl -fsSL -X POST \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/octet-stream" \
    --data-binary @"$file" \
    "https://uploads.github.com/repos/$REPO/releases/$rid/assets?name=$enc" >/dev/null
  echo "  uploaded $name ($(du -h "$file" | cut -f1))"
}

export TAG REPO RELEASE_NAME
cmd="${1:-}"; shift || true
case "$cmd" in
  ensure)
    ensure_release; echo >&2
    ;;
  upload)
    [ "$#" -gt 0 ] || { echo "usage: release-upload.sh upload <file> [file...]" >&2; exit 1; }
    rid="$(ensure_release)"
    echo "release $TAG -> id $rid"
    for f in "$@"; do
      if [ -f "$f" ]; then upload_one "$rid" "$f"; else echo "  skip (missing): $f" >&2; fi
    done
    echo "done. Review/publish the draft: https://github.com/$REPO/releases"
    ;;
  *)
    echo "usage: release-upload.sh {ensure | upload <file> [file...]}" >&2; exit 1
    ;;
esac
