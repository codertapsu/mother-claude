---
description: Run all quality gates (fmt, clippy, cargo test, Angular lint/build) and report what passes.
---

Run every quality gate this repo requires for a green commit, then report a
concise PASS/FAIL table:

1. `cargo fmt --manifest-path src-tauri/Cargo.toml --check`
2. `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`
3. `cargo test --manifest-path src-tauri/Cargo.toml`
4. `npm run lint`
5. `npm run build`

Do not fix anything unless asked — just report results.
