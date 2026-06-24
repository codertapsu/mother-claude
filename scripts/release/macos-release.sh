#!/usr/bin/env bash
#
# macos-release.sh — build + sign + notarize the macOS app LOCALLY and upload the
# .dmg to the draft GitHub release. Keeps macOS off CI (no 10x-billed Actions
# minutes); pair it with the Windows build in CI (RELEASE_CI_WINDOWS=true).
#
# `tauri build` signs (hardened runtime) + notarizes + staples the .APP, but it
# does NOT notarize/staple the .DMG wrapper — an unstapled dmg is rejected by
# Gatekeeper on download ("Apple cannot check it"). So this script adds an
# explicit notarize + staple pass on the dmg. See docs/APPLE_SIGNING.md.
#
# Prereqs:
#   - Your "Developer ID Application" cert is in the login keychain
#     (check: security find-identity -v -p codesigning).
#   - Export the signing + notarization env first:
#       export APPLE_SIGNING_IDENTITY="Developer ID Application: Khanh Hoang (4XVYLW8RXS)"
#       export APPLE_ID="hoangduykhanh.dn@gmail.com"
#       export APPLE_PASSWORD="<app-specific password>"   # appleid.apple.com → App-Specific Passwords
#       export APPLE_TEAM_ID="4XVYLW8RXS"
#
# Usage:
#   RELEASE_TAG=v0.1.0 bash scripts/release/macos-release.sh                 # build → notarize → upload
#   RELEASE_TAG=v0.1.0 SKIP_BUILD=1  bash scripts/release/macos-release.sh   # reuse an existing dmg
#   RELEASE_TAG=v0.1.0 SKIP_UPLOAD=1 bash scripts/release/macos-release.sh   # stop before upload
set -euo pipefail

: "${APPLE_SIGNING_IDENTITY:?export APPLE_SIGNING_IDENTITY first (see header)}"
: "${APPLE_ID:?export APPLE_ID first}"
: "${APPLE_PASSWORD:?export APPLE_PASSWORD (app-specific) first}"
: "${APPLE_TEAM_ID:?export APPLE_TEAM_ID first}"

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

if [ "${SKIP_BUILD:-}" = "1" ]; then
  echo "==> SKIP_BUILD=1 — reusing the existing build."
else
  echo "==> Building + signing + notarizing the app (npm run tauri:build)…"
  echo "    identity: $APPLE_SIGNING_IDENTITY"
  npm run tauri:build
fi

dmg="$(find src-tauri/target/release/bundle/dmg -name '*.dmg' -print -quit 2>/dev/null || true)"
[ -n "$dmg" ] || { echo "error: no .dmg under src-tauri/target/release/bundle/dmg" >&2; exit 1; }
app="$(find src-tauri/target/release/bundle/macos -name '*.app' -print -quit 2>/dev/null || true)"
echo "==> Artifact: $dmg"

# Notarize + staple the DMG itself (the app inside is already notarized+stapled
# by tauri build). notarytool --wait blocks until Apple finishes (~1-5 min).
echo "==> Notarizing the .dmg (notarytool --wait)…"
xcrun notarytool submit "$dmg" \
  --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID" --wait
echo "==> Stapling the .dmg…"
xcrun stapler staple "$dmg"

echo "==> Verifying (should be 'accepted … Notarized Developer ID')…"
spctl -a -vvv -t open --context context:primary-signature "$dmg" 2>&1 | sed 's/^/    /' || true
xcrun stapler validate "$dmg" 2>&1 | sed 's/^/    /' || echo "    (dmg staple validation failed)"
if [ -n "$app" ]; then
  spctl -a -vvv -t install "$app" 2>&1 | sed 's/^/    /' || true
fi

if [ "${SKIP_UPLOAD:-}" = "1" ]; then
  echo "==> SKIP_UPLOAD=1 — done (not uploading)."
  exit 0
fi

echo "==> Uploading the .dmg to the draft release (${RELEASE_TAG:-v0.1.0})…"
bash "$root/scripts/release/release-upload.sh" upload "$dmg"
