#!/usr/bin/env bash
#
# release-macos.sh — one command to cut the macOS half of a Mother Claude
# release on this Mac: preflight, quality gate, build, sign, notarize, staple,
# verify, upload.
#
# Artifacts are built locally by design, not in CI (docs/RELEASING.md). The
# Windows desktop runs release-windows.ps1 against the same tag, at the same
# time if you like; both upload to the same draft.
#
# `tauri build` signs the .app with the hardened runtime and notarizes it, but it
# does NOT notarize or staple the .dmg wrapper. An unstapled dmg is rejected by
# Gatekeeper on a machine that has never seen it — "Apple could not verify" — and
# the failure only shows up on someone else's computer, after release. So this
# script adds an explicit notarize + staple pass over the dmg and then verifies
# the result rather than assuming it.
#
# Usage:
#   source scripts/apple-credentials.env        # git-ignored; see the .example
#   RELEASE_TAG=v0.4.0 bash scripts/release/release-macos.sh
#
#   SKIP_UPLOAD=1 ...   build + notarize, stop before touching GitHub
#   SKIP_BUILD=1  ...   reuse the bundle already built (no gate, no build)
#   ALLOW_DIRTY=1 ...   build uncommitted work or another commit (rehearsals only)
#
# Required env: APPLE_SIGNING_IDENTITY, APPLE_ID, APPLE_PASSWORD, APPLE_TEAM_ID.
set -euo pipefail

cd "$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

TAG="${RELEASE_TAG:?set RELEASE_TAG (e.g. RELEASE_TAG=v0.4.0)}"

: "${APPLE_SIGNING_IDENTITY:?set APPLE_SIGNING_IDENTITY — run: source scripts/apple-credentials.env}"
: "${APPLE_ID:?set APPLE_ID — run: source scripts/apple-credentials.env}"
: "${APPLE_PASSWORD:?set APPLE_PASSWORD (an app-specific password, not your Apple ID password)}"
: "${APPLE_TEAM_ID:?set APPLE_TEAM_ID — run: source scripts/apple-credentials.env}"

[ "$(uname -s)" = "Darwin" ] || {
  echo "error: this is the macOS leg; run scripts/release/release-windows.ps1 on the Windows desktop." >&2
  exit 1
}

# The updater signing key. `bundle.createUpdaterArtifacts` is on, so the build
# produces a .tar.gz and signs it — and stops with "A public key has been found,
# but no private key" if this is missing. The key has an empty password; the
# file's permissions are what protect it.
if [ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ]; then
  UPDATER_KEY="${HOME}/.tauri/mother-claude-updater.key"
  [ -f "$UPDATER_KEY" ] || {
    echo "error: no updater signing key at $UPDATER_KEY, and TAURI_SIGNING_PRIVATE_KEY is unset." >&2
    echo "       Without it the build cannot sign an update, and shipping an unsigned one" >&2
    echo "       is not possible — the public key is compiled into every installed copy." >&2
    echo "       See docs/RELEASING.md, 'The updater signing key'." >&2
    exit 1
  }
  case "$(stat -f '%Lp' "$UPDATER_KEY")" in
    600 | 400) ;;
    *) echo "    WARNING: $UPDATER_KEY is readable by other users; chmod 600 it." >&2 ;;
  esac
  TAURI_SIGNING_PRIVATE_KEY="$(cat "$UPDATER_KEY")"
  export TAURI_SIGNING_PRIVATE_KEY
fi
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}"

# Apple silicon only, and built for this Mac's own architecture. Not universal,
# as Gitweaver is: the bundle carries sidecar/node_modules, and the Agent SDK in
# it ships Claude Code as a native binary chosen by npm for the machine that
# installed it (@anthropic-ai/claude-agent-sdk-darwin-arm64 here). A universal
# or x86_64 app built on this Mac would carry the arm64 binary, and every bridge
# conversation on an Intel Mac would fail. Shipping Intel means bundling the
# darwin-x64 package too, deliberately — see docs/RELEASING.md.
MAC_TARGET="aarch64-apple-darwin"
SDK_PACKAGE="claude-agent-sdk-darwin-arm64"
[ "$(uname -m)" = "arm64" ] || {
  echo "error: build the macOS leg on an Apple silicon Mac (this one is $(uname -m))." >&2
  exit 1
}
BUNDLE="src-tauri/target/$MAC_TARGET/release/bundle"

# ---------------------------------------------------------------------------
# 1. Preflight. Everything cheap that can invalidate the release happens BEFORE
#    the multi-minute notarize, because notarization cannot be taken back.
# ---------------------------------------------------------------------------
echo "==> preflight: the version spots agree, and match $TAG"
node scripts/release/set-version.mjs --check --expect "$TAG"

echo "==> preflight: the signing identity is in the keychain"
security find-identity -v -p codesigning | grep -qF "$APPLE_SIGNING_IDENTITY" || {
  echo "error: '$APPLE_SIGNING_IDENTITY' is not in the login keychain." >&2
  echo "       Available identities:" >&2
  security find-identity -v -p codesigning | sed 's/^/         /' >&2
  exit 1
}

echo "==> preflight: the working tree is clean, and is $TAG"
# A release built from uncommitted work, or from a commit other than the tag's,
# cannot be reproduced from its tag, and nobody can tell later which of the two
# the users actually got. Both legs must build the tag's commit.
TAG_COMMIT="$(git rev-parse -q --verify "refs/tags/$TAG^{commit}" || true)"
HEAD_COMMIT="$(git rev-parse HEAD)"
if [ -n "$(git status --porcelain)" ] || [ "$TAG_COMMIT" != "$HEAD_COMMIT" ]; then
  if [ "${ALLOW_DIRTY:-}" = "1" ]; then
    echo "    WARNING: ALLOW_DIRTY=1 — this build is not reproducible from $TAG"
  elif [ -z "$TAG_COMMIT" ]; then
    echo "error: there is no tag $TAG here. Tag before building: git tag $TAG && git push origin $TAG" >&2
    exit 1
  elif [ "$TAG_COMMIT" != "$HEAD_COMMIT" ]; then
    echo "error: HEAD is not $TAG. Build the tag, not whatever the branch has moved to:" >&2
    echo "         git checkout $TAG" >&2
    exit 1
  else
    git status --short | sed 's/^/      /'
    echo "error: uncommitted changes. Commit them, or set ALLOW_DIRTY=1 if you mean it." >&2
    exit 1
  fi
fi

# Before the build, not after it. A token that cannot write to the release
# repository is a two-second check and a twenty-minute wasted build.
if [ "${SKIP_UPLOAD:-}" != "1" ]; then
  echo "==> preflight: this machine can publish to the release repository"
  RELEASE_TAG="$TAG" bash scripts/release/release-upload.sh preflight
fi

if [ "${SKIP_BUILD:-}" = "1" ]; then
  echo "==> SKIP_BUILD=1 — reusing the existing bundle (it was gated when it was built)"
else
  echo "==> preflight: the Rust target $MAC_TARGET is installed"
  rustup target list --installed | grep -qx "$MAC_TARGET" || {
    echo "error: the Rust target $MAC_TARGET is missing. Fix: rustup target add $MAC_TARGET" >&2
    exit 1
  }

  # The dependencies the lockfiles name, exactly — not whatever node_modules
  # happens to hold from the last `npm install`.
  echo "==> install: npm ci"
  npm ci

  echo "==> the full quality gate"
  cargo fmt --manifest-path src-tauri/Cargo.toml --check
  cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
  # The Windows leg compiles this commit too, on another machine, much later:
  # a Unix-only item it trips over is found here instead (see the script).
  bash scripts/release/check-windows.sh
  cargo test --manifest-path src-tauri/Cargo.toml
  npm run lint
  npm test
  npm run test:release

  # Delete the previous bundle first. A failed build leaves the last good dmg in
  # place, and packaging that under a new version ships a stale binary under a
  # version number it does not contain.
  rm -rf "$BUNDLE/dmg" "$BUNDLE/macos"

  echo "==> build + sign + notarize the .app (tauri build --target $MAC_TARGET)"
  echo "    identity: $APPLE_SIGNING_IDENTITY"
  # beforeBuildCommand bundles the sidecar (npm run sidecar:bundle) and builds
  # the Angular app; both end up inside the .app.
  npx tauri build --target "$MAC_TARGET"
fi

DMG="$(find "$BUNDLE/dmg" -name '*.dmg' -print -quit 2>/dev/null || true)"
APP="$(find "$BUNDLE/macos" -maxdepth 1 -name '*.app' -print -quit 2>/dev/null || true)"
[ -n "$DMG" ] || {
  echo "error: no .dmg under $BUNDLE/dmg — the build did not produce one." >&2
  exit 1
}
[ -n "$APP" ] || {
  echo "error: no .app under $BUNDLE/macos — the build did not produce one." >&2
  exit 1
}
echo "==> artifact: $DMG"

# The architecture, and the native piece the bridge needs, proven rather than
# assumed: both sign, notarize and install perfectly when they are wrong, and
# the first person to find out is the one whose conversations never start.
EXE="$APP/Contents/MacOS/$(plutil -extract CFBundleExecutable raw "$APP/Contents/Info.plist")"
ARCHS="$(lipo -archs "$EXE" 2>/dev/null || echo '?')"
echo "    architectures: $ARCHS"
[ "$ARCHS" = "arm64" ] || {
  echo "error: expected an arm64 executable, found: $ARCHS" >&2
  exit 1
}
SDK_BINARY="$APP/Contents/Resources/sidecar/node_modules/@anthropic-ai/$SDK_PACKAGE/claude"
[ -x "$SDK_BINARY" ] || {
  echo "error: the bundle has no $SDK_PACKAGE binary; bridge conversations could not start." >&2
  exit 1
}
echo "    sidecar SDK binary: $(lipo -archs "$SDK_BINARY" 2>/dev/null || echo '?')"

# ---------------------------------------------------------------------------
# 2. Notarize + staple the dmg itself, then prove it worked.
# ---------------------------------------------------------------------------
echo "==> notarize the .dmg (notarytool --wait; usually 1-5 minutes)"
xcrun notarytool submit "$DMG" \
  --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID" --wait

echo "==> staple the ticket to the .dmg"
xcrun stapler staple "$DMG"

echo "==> verify (this is the check that catches a Gatekeeper failure here rather than on a user's Mac)"
xcrun stapler validate "$DMG" || {
  echo "error: the .dmg is not stapled — Gatekeeper will reject it on download." >&2
  exit 1
}
spctl -a -vvv -t open --context context:primary-signature "$DMG" 2>&1 | sed 's/^/    /'
# The app as well, and fatally: `-t exec` is the question Gatekeeper asks when
# somebody opens it, and this .app is the same bytes as the one in the dmg and
# the one in the update archive, so it answers for all three.
spctl -a -vvv -t exec "$APP" 2>&1 | sed 's/^/    /'

if [ "${SKIP_UPLOAD:-}" = "1" ]; then
  echo "==> SKIP_UPLOAD=1 — stopping before upload."
  echo "    Built: $DMG"
  exit 0
fi

# ---------------------------------------------------------------------------
# 3. Upload to the tag's draft release. The Windows leg uploads to the same one.
# ---------------------------------------------------------------------------
echo "==> upload to the draft release $TAG"
RELEASE_TAG="$TAG" bash scripts/release/release-upload.sh upload "$DMG"

# The updater's own artifact. Separate from the .dmg: a person downloads the
# dmg, the application downloads this. Both describe the same build.
TARBALL="$(find "$BUNDLE/macos" -maxdepth 1 -name '*.app.tar.gz' -print -quit 2>/dev/null || true)"
[ -n "$TARBALL" ] && [ -f "$TARBALL.sig" ] || {
  echo "error: no signed update archive under $BUNDLE/macos." >&2
  echo "       Expected a .app.tar.gz and its .sig — check that" >&2
  echo "       bundle.createUpdaterArtifacts is true in src-tauri/tauri.conf.json." >&2
  exit 1
}

# The updater looks up `darwin-aarch64-app`, then `darwin-aarch64`. One key is
# enough; an Intel build, if there ever is one, adds darwin-x86_64 beside it.
echo "==> publish the update manifest"
RELEASE_TAG="$TAG" bash scripts/release/release-upload.sh manifest \
  "$TARBALL" "$TARBALL.sig" darwin-aarch64

echo
echo "macOS leg done for $TAG — its 3 of the 8 assets, and its entry in latest.json."
echo
echo "The Windows leg runs independently and may already have finished. Check:"
echo "  RELEASE_TAG=$TAG bash scripts/release/release-upload.sh verify"
echo
echo "When that reports all 8, write the notes and publish:"
echo "  https://github.com/codertapsu/mother-claude/releases"
