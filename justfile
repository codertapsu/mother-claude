# Mother Claude task runner. `just <target>`. Mirrors package.json scripts.
set shell := ["bash", "-cu"]

manifest := "src-tauri/Cargo.toml"

# Run all quality gates (what CI enforces).
ci: fmt-check clippy test lint build

# --- Rust ---
fmt:
    cargo fmt --manifest-path {{manifest}}

fmt-check:
    cargo fmt --manifest-path {{manifest}} --check

clippy:
    cargo clippy --manifest-path {{manifest}} --all-targets -- -D warnings

clippy-experimental:
    cargo clippy --manifest-path {{manifest}} --all-targets --features experimental -- -D warnings

test:
    cargo test --manifest-path {{manifest}}

# --- Angular ---
lint:
    npm run lint

build:
    npm run build

# --- App ---
dev:
    npm run tauri:dev

bundle:
    npm run tauri:build

# --- Release (see docs/RELEASING.md; installers are built locally, not in CI) ---
# Write the version to every spot (package.json, the lockfiles, Cargo.toml,
# tauri.conf.json), or check they agree: `just set-version --check`.
set-version *args:
    node scripts/release/set-version.mjs {{args}}

# The release tooling's own tests (signature checks, manifest merge, versions).
test-release:
    npm run test:release

# The macOS leg: gate, build, sign, notarize, upload to the tag's draft release.
# Run on the Mac with the tag checked out, after `source scripts/apple-credentials.env`.
# e.g. `just release-macos v0.4.0`
release-macos tag:
    RELEASE_TAG={{tag}} bash scripts/release/release-macos.sh

# Both legs finished, and every platform's update verifies? Run before publishing.
release-verify tag:
    RELEASE_TAG={{tag}} bash scripts/release/release-upload.sh verify
