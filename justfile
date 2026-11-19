# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause

set fallback := false

# List available recipes
default:
    @just --list

# Full suite pass (fmt, clippy, deb build, smoke, nextest, deny, reuse)
verify:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo deb -p kestic-gui --fast
    just smoke
    KESTIC_TEST_FAST_INIT=1 cargo nextest run --workspace
    cargo deny check
    reuse lint

# Format all Rust code in the workspace
fmt:
    cargo fmt --all

# Build the Debian package (fast profile)
deb:
    cargo deb -p kestic-gui --fast

# Run offscreen QML smoke test and write screenshot
smoke:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build --bin kestic
    tmpdir=$(mktemp -d)
    trap 'rm -rf "$tmpdir"' EXIT
    mkdir -p "$tmpdir"/{home,cfg,data,cache,state,run}
    chmod 700 "$tmpdir/run"
    HOME="$tmpdir/home" \
    XDG_CONFIG_HOME="$tmpdir/cfg" \
    XDG_DATA_HOME="$tmpdir/data" \
    XDG_CACHE_HOME="$tmpdir/cache" \
    XDG_STATE_HOME="$tmpdir/state" \
    XDG_RUNTIME_DIR="$tmpdir/run" \
    LC_ALL= \
    LANG=C.UTF-8 \
    LANGUAGE=en \
    QT_QPA_PLATFORM=offscreen \
    ./target/debug/kestic --smoke-test --smoke-lang en --screenshot target/smoke-artifacts/m0-s06-main.png

# Build binary and .deb package, then run workspace tests
test:
    cargo build --bin kestic
    cargo deb -p kestic-gui --fast
    KESTIC_TEST_FAST_INIT=1 cargo nextest run --workspace
