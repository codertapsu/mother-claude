#!/usr/bin/env bash
#
# make-latest-json.sh — finish the auto-updater manifest for a release whose
# macOS half is built LOCALLY and whose Windows half is built in CI.
#
# The Windows CI build (tauri-action, includeUpdaterJson:true) already uploaded a
# latest.json carrying the windows entries (windows-x86_64 / -nsis / -msi, so both
# Windows install types self-update). This downloads THAT manifest, ADDS the local
# macOS darwin-aarch64 entry — signature = the .app.tar.gz.sig file contents, url =
# the .app.tar.gz release asset — and re-uploads the merged manifest.
#
# The macOS updater downloads the .app.tar.gz (NOT the .dmg). See docs/AUTOUPDATE.md.
#
# Prereqs:
#   - macOS updater artifacts exist locally: build (arm64) with
#     createUpdaterArtifacts=true AND TAURI_SIGNING_PRIVATE_KEY set, so a
#     *aarch64.app.tar.gz + .sig exist.
#   - The Windows CI build has already uploaded its latest.json to this tag.
#
# Usage: RELEASE_TAG=v0.1.0 bash scripts/release/make-latest-json.sh
set -euo pipefail

TAG="${RELEASE_TAG:-v0.1.0}"
REPO="${GH_REPO:-codertapsu/mother-claude}"
VERSION="${TAG#v}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DL="https://github.com/$REPO/releases/download/$TAG"

TOKEN="${GH_TOKEN:-$(printf 'protocol=https\nhost=github.com\n\n' | git credential fill 2>/dev/null | sed -n 's/^password=//p')}"
[ -n "$TOKEN" ] || { echo "error: no GitHub token (set GH_TOKEN or log in so 'git credential' has one)" >&2; exit 1; }
api() { curl -fsSL -H "Authorization: Bearer $TOKEN" -H "Accept: application/vnd.github+json" "$@"; }

# Download a release asset by name to stdout. Draft assets are NOT served at
# github.com/releases/download/<tag>/<name> (404 until published), so resolve the
# asset id from the release JSON ($1) and GET it with Accept: octet-stream.
download_asset() {
  local id
  id="$(printf '%s' "$1" | N="$2" python3 -c "import sys,json,os;print(next((str(a['id']) for a in json.load(sys.stdin).get('assets',[]) if a['name']==os.environ['N']), ''))")"
  [ -n "$id" ] || return 1
  curl -fsSL -H "Authorization: Bearer $TOKEN" -H "Accept: application/octet-stream" \
    "https://api.github.com/repos/$REPO/releases/assets/$id"
}

# --- 1. Local macOS updater artifacts ----------------------------------------
# Tauri names the macOS updater tarball "<productName>.app.tar.gz" (no version or
# arch in the name; only the .dmg carries those). The local arm64 build writes
# exactly one here under target/release/bundle/macos.
mac_targz="$(find "$ROOT/src-tauri/target/release/bundle/macos" -name '*.app.tar.gz' -print -quit 2>/dev/null || true)"
[ -n "$mac_targz" ] || { echo "error: no .app.tar.gz under src-tauri/target/release/bundle/macos — build with createUpdaterArtifacts=true and TAURI_SIGNING_PRIVATE_KEY set first" >&2; exit 1; }
[ -f "$mac_targz.sig" ] || { echo "error: missing $mac_targz.sig — set TAURI_SIGNING_PRIVATE_KEY before 'tauri build'" >&2; exit 1; }
mac_sig="$(cat "$mac_targz.sig")"
mac_asset="$(basename "$mac_targz" | tr ' ' '.')"   # name as release-upload.sh uploads it

echo "==> Uploading macOS updater artifact ($mac_asset)…"
bash "$ROOT/scripts/release/release-upload.sh" upload "$mac_targz" >/dev/null

# --- 2. Fetch the CI-produced latest.json (carries the windows entries) -----
rel="$(api "https://api.github.com/repos/$REPO/releases/tags/$TAG")"
has_lj="$(printf '%s' "$rel" | python3 -c "import sys,json;print('yes' if any(a['name']=='latest.json' for a in json.load(sys.stdin).get('assets',[])) else '')")"
[ -n "$has_lj" ] || { echo "error: no latest.json on release $TAG yet — let the Windows CI build finish (it uploads it), then re-run." >&2; exit 1; }
ci_manifest="$(download_asset "$rel" "latest.json")"

# --- 3. Merge: keep the windows entries, add darwin-aarch64 -----------------
out="$ROOT/latest.json"
PUB_DATE="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
CI_MANIFEST="$ci_manifest" MAC_SIG="$mac_sig" MAC_URL="$DL/$mac_asset" \
VERSION="$VERSION" PUB_DATE="$PUB_DATE" OUT="$out" python3 - <<'PY'
import json, os, sys
m = json.loads(os.environ["CI_MANIFEST"])
if "platforms" not in m or not any(k.startswith("windows-") for k in m["platforms"]):
    print("error: CI latest.json has no windows-* platform entries", file=sys.stderr); sys.exit(1)
m["version"] = os.environ["VERSION"]
m.setdefault("notes", "Update to the latest version. See the GitHub release page for details.")
m["pub_date"] = os.environ["PUB_DATE"]
m["platforms"]["darwin-aarch64"] = {"signature": os.environ["MAC_SIG"], "url": os.environ["MAC_URL"]}
with open(os.environ["OUT"], "w") as f:
    f.write(json.dumps(m, indent=2) + "\n")
PY

echo "==> Uploading merged latest.json…"
bash "$ROOT/scripts/release/release-upload.sh" upload "$out" >/dev/null

# --- 4. Verify (draft-safe: manifest shape + referenced assets exist) -------
echo "==> Verifying…"
rel="$(api "https://api.github.com/repos/$REPO/releases/tags/$TAG")"
NAMES="$(printf '%s' "$rel" | python3 -c "import sys,json;print('\n'.join(a['name'] for a in json.load(sys.stdin).get('assets',[])))")"
OUT="$out" MAC_ASSET="$mac_asset" NAMES="$NAMES" EXPECT_VERSION="$VERSION" python3 - <<'PY'
import json, os, sys
m = json.load(open(os.environ["OUT"]))
names = set(os.environ["NAMES"].splitlines())
ok = True
if m["version"] != os.environ["EXPECT_VERSION"]:
    print(f"  FAIL version {m['version']!r} != tag version {os.environ['EXPECT_VERSION']!r}"); ok = False
plats = m.get("platforms", {})
if "darwin-aarch64" not in plats or not plats["darwin-aarch64"]["signature"].strip():
    print("  FAIL darwin-aarch64 entry missing/empty"); ok = False
elif os.environ["MAC_ASSET"] not in names:
    print(f"  FAIL darwin-aarch64 asset {os.environ['MAC_ASSET']!r} not on release"); ok = False
else:
    print(f"  OK   darwin-aarch64 -> {os.environ['MAC_ASSET']}")
win = [k for k in plats if k.startswith("windows-")]
if not win:
    print("  FAIL no windows-* entries"); ok = False
for k in win:
    p = plats[k]
    asset = p["url"].rsplit("/", 1)[-1]
    bad = (not p["signature"].strip()) or (asset not in names)
    if bad: ok = False
    print(f"  {'FAIL' if bad else 'OK  '} {k} -> {asset}")
if "latest.json" not in names:
    print("  FAIL latest.json not uploaded"); ok = False
print("version:", m["version"], "platforms:", sorted(plats))
sys.exit(0 if ok else 1)
PY
rm -f "$out"
echo "done. latest.json is on $TAG; once published it serves at:"
echo "  https://github.com/$REPO/releases/latest/download/latest.json"
