#!/usr/bin/env bash
#
# check-windows.sh — type-check the Windows build of the app, from this Mac.
#
# The Rust code is written and tested on macOS, and most of what differs on
# Windows sits behind `cfg(unix)`. Until this script, nothing compiled it for
# Windows before the Windows release leg did — which is how v0.4.0's first
# Windows build failed, on the other machine, on a constant only Unix had. This
# finds the same errors here in a minute or two: clippy for
# x86_64-pc-windows-msvc over the app's library and binary, warnings denied.
# release-macos.sh runs it in its gate, before anything is tagged or built.
#
# Nothing is linked, so no Windows SDK is needed — but four build scripts
# compile native code and cannot run here: ring's, libgit2's and zlib's (the
# last two through git2) compile C, and Tauri's compiles the Windows resource
# file. The C ones are overridden through their `links` keys, and Tauri's gets
# a stand-in resource compiler; both live in a temporary directory, never in
# the repository. If a new dependency compiles C for Windows, this fails in its
# build script: add an override for its `links` key below.
#
# Needs: rustup target add x86_64-pc-windows-msvc
set -euo pipefail

cd "$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

TARGET=x86_64-pc-windows-msvc
rustup target list --installed | grep -qx "$TARGET" || {
  echo "error: the Rust target $TARGET is missing. Fix: rustup target add $TARGET" >&2
  exit 1
}

TMP="$(mktemp -d -t mother-claude-windows-check)"
trap 'rm -rf "$TMP"' EXIT

# ring's `links` key carries its version: ring_core_0_17_14_ for 0.17.14.
RING_VERSION="$(awk '$0 == "name = \"ring\"" { getline; gsub(/version = |"/, ""); print; exit }' src-tauri/Cargo.lock)"
[ -n "$RING_VERSION" ] || {
  echo "error: no ring in src-tauri/Cargo.lock — update the overrides in $0." >&2
  exit 1
}

cat > "$TMP/config.toml" <<EOF
[target.$TARGET.ring_core_${RING_VERSION//./_}_]
rustc-link-lib = []
[target.$TARGET.z]
rustc-link-lib = []
[target.$TARGET.git2]
rustc-link-lib = []
rustc-cfg = ["libgit2_vendored"]
EOF

# embed-resource asks the compiler named by RC_<target> for `/?` and expects
# llvm-rc's banner, then runs it with `/fo <output>`. An empty output is enough
# for a check, which never links it.
cat > "$TMP/llvm-rc" <<'EOF'
#!/bin/sh
case "$*" in
  *"/?"*) echo "OVERVIEW: LLVM Resource Converter"; echo "  /no-preprocess"; exit 0 ;;
esac
out=""
while [ $# -gt 0 ]; do
  if [ "$1" = "/fo" ]; then out="$2"; shift; fi
  shift
done
[ -z "$out" ] || : > "$out"
EOF
chmod +x "$TMP/llvm-rc"

echo "==> clippy for $TARGET (the app's library and binary, warnings denied)"
RC_x86_64_pc_windows_msvc="$TMP/llvm-rc" cargo clippy \
  --manifest-path src-tauri/Cargo.toml \
  --config "$TMP/config.toml" \
  --target "$TARGET" \
  --target-dir src-tauri/target/windows-check \
  --lib --bins -- -D warnings
echo "    the Windows build type-checks."
