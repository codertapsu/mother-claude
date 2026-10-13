#!/usr/bin/env python3
"""Resolve the per-OS CI build matrix for .github/workflows/release.yml.

Installers are built LOCALLY (scripts/release/*, see docs/RELEASING.md). These
per-OS flags are the only way to have CI build an OS instead, and starting the
workflow by hand does not count as one. Disabled OSes are omitted from the
emitted matrix, so the `build` job provisions NO runner for them; with every OS
disabled the run fails here, saying why, rather than succeeding at nothing.

Prints two GitHub Actions step outputs to stdout (append to $GITHUB_OUTPUT):
  matrix=<json {"include":[...]}>   - the build matrix
  any=<true|false>                  - whether anything is selected

Env:
  CI_MACOS    repo var RELEASE_CI_MACOS    (default: off -> macOS built locally)
  CI_WINDOWS  repo var RELEASE_CI_WINDOWS  (default: off -> Windows built locally)
An unset/empty flag uses the default; "true" forces on, anything else forces off.
"""
import json
import os
import sys

# The build-matrix entries per OS (mirrors what release.yml's build job needs:
# os / label / rust-target / tauri-args). Linux is intentionally omitted —
# Mother Claude targets macOS + Windows; add a "linux" entry here (and the apt
# system-deps step in release.yml) if that changes.
OSES = {
    "macos": [
        {"os": "macos-14", "label": "macos-arm64", "rust-target": "aarch64-apple-darwin", "tauri-args": "--target aarch64-apple-darwin"},
        {"os": "macos-13", "label": "macos-x64", "rust-target": "x86_64-apple-darwin", "tauri-args": "--target x86_64-apple-darwin"},
    ],
    "windows": [
        {"os": "windows-latest", "label": "windows-x64", "rust-target": "x86_64-pc-windows-msvc", "tauri-args": ""},
    ],
}

# Default when a repo variable is unset: every OS builds LOCALLY. CI is opt-in
# per OS (the safe, cost-free default — no surprise 10x macOS minutes). Set
# RELEASE_CI_<OS>=true to opt that OS into a CI build.
DEFAULTS = {"macos": False, "windows": False}


def enabled(key: str) -> bool:
    raw = os.environ.get("CI_" + key.upper(), "").strip().lower()
    return DEFAULTS[key] if raw == "" else raw == "true"


def main() -> None:
    include = []
    for key in ("macos", "windows"):
        if enabled(key):
            include.extend(OSES[key])
    if not include:
        print(
            "::error::No operating system is switched on for CI builds "
            "(RELEASE_CI_MACOS / RELEASE_CI_WINDOWS). Installers are built "
            "locally; see docs/RELEASING.md.",
            file=sys.stderr,
        )
        sys.exit(1)
    print("matrix=" + json.dumps({"include": include}))
    print("any=" + ("true" if include else "false"))


if __name__ == "__main__":
    main()
