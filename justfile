# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause

set fallback := false

default:
    @just --list

# Run all verification checks (AC-M0-01-1, AC-M0-06-1)
verify:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo build --bin kestic
    cargo deb -p kestic-gui --fast
    just smoke
    cargo test --workspace
    cargo deny check
    reuse lint

fmt:
    cargo fmt --all

deb:
    cargo deb -p kestic-gui --fast

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
    QT_QPA_PLATFORM=offscreen \
    ./target/debug/kestic --smoke-test --screenshot target/smoke-artifacts/m0-s06-main.png

test:
    cargo build --bin kestic
    cargo deb -p kestic-gui --fast
    cargo test --workspace
