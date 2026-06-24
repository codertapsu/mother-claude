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

# --- Release (see docs/RELEASING.md) ---
# Sync the app version across package.json / tauri.conf.json / Cargo.toml.
set-version version:
    node scripts/release/set-version.mjs {{version}}

# Upload locally-built installers to the draft GitHub release for a tag.
# e.g. `just release-upload v0.2.0 src-tauri/target/release/bundle/dmg/*.dmg`
release-upload tag *files:
    RELEASE_TAG={{tag}} bash scripts/release/release-upload.sh upload {{files}}

# Build + sign + notarize the macOS app and upload it to the tag's draft release.
# Requires APPLE_SIGNING_IDENTITY / APPLE_ID / APPLE_PASSWORD / APPLE_TEAM_ID
# exported (cert in the login keychain). e.g. `just macos-release v0.1.0`
macos-release tag:
    RELEASE_TAG={{tag}} bash scripts/release/macos-release.sh
