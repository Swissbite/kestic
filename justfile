# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause

set fallback := false

# List available recipes
default:
    @just --list

# Full suite pass (fmt, clippy, file sizes, deb build, smoke, nextest, deny, reuse)
verify:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
    just _check-file-sizes
    cargo deb -p kestic-gui --fast
    just smoke
    KESTIC_TEST_FAST_INIT=1 cargo nextest run --workspace
    cargo deny check
    reuse lint

_check-file-sizes:
    #!/usr/bin/env bash
    set -euo pipefail
    oversized=0
    while IFS= read -r -d '' f; do
        lines=$(wc -l < "$f")
        if [ "$lines" -gt 1500 ]; then
            echo "ERROR: $f has $lines lines (limit 1500, NFR-12)" >&2
            oversized=1
        fi
    done < <(find core/src daemon/src gui/src -type f \( -name '*.rs' -o -name '*.qml' \) -print0 | sort -z)
    if [ "$oversized" -ne 0 ]; then
        exit 1
    fi

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
    # Private session bus without service activation (see the fixture): the GUI
    # daemon never lands on the user's real bus, and no portal/ksecretd leaks.
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
    dbus-run-session --config-file=daemon/tests/fixtures/private-session-bus.xml -- \
    ./target/debug/kestic --smoke-test --smoke-lang en --screenshot target/smoke-artifacts/m0-s06-main.png

# Build binary and .deb package, then run workspace tests
test:
    cargo build --bin kestic
    cargo deb -p kestic-gui --fast
    KESTIC_TEST_FAST_INIT=1 cargo nextest run --workspace

# Showcase site (site/). Needs pnpm; pnpm provides Node
# itself (devEngines.runtime). Screenshots come from target/smoke-artifacts/,
# so run `just verify` (or the GUI smoke tests) first.

# Check and build the showcase site into site/dist/
site:
    pnpm --dir site install --frozen-lockfile
    pnpm --dir site build

# Run the showcase site dev server
site-dev:
    pnpm --dir site install --frozen-lockfile
    pnpm --dir site dev

# Serve the built showcase site (after `just site`)
site-preview:
    pnpm --dir site preview
