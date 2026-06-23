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
