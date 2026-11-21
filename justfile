# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause

set fallback := false

# One id per `just` invocation: test fixtures (template repositories with generated passwords)
# live in target/kestic-test-fixtures/<id> and are removed after the run.
fixture_run := uuid()
fixtures_dir := env_var_or_default("CARGO_TARGET_DIR", "target") / "kestic-test-fixtures"

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
    just _nextest
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
    done < <(find core/src i18n/src daemon/src gui/src test-support/src -type f \( -name '*.rs' -o -name '*.qml' \) -print0 | sort -z)
    if [ "$oversized" -ne 0 ]; then
        exit 1
    fi

# Format all Rust code in the workspace
fmt:
    cargo fmt --all

# Build the Debian package (fast profile)
deb:
    cargo deb -p kestic-gui --fast

# Start the main window offscreen (QML smoke) and write a screenshot
smoke:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build --bin kestic
    tmpdir=$(mktemp -d)
    trap 'rm -rf "$tmpdir"' EXIT
    mkdir -p "$tmpdir"/{home,cfg,data,cache,state,run}
    chmod 700 "$tmpdir/run"
    # Private session bus without service activation (see the fixture): the daemon
    # never lands on the user's real bus, and no portal/ksecretd leaks. The daemon is
    # started and stopped here, so no process outlives the smoke.
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
    dbus-run-session --config-file=test-support/fixtures/private-session-bus.xml -- bash -c '
        ./target/debug/kestic daemon --no-job-view --no-portal-inhibit --suppress-ui-spawn \
            --ready-file "$HOME/daemon.ready" &
        daemon=$!
        trap "kill $daemon 2>/dev/null; wait $daemon 2>/dev/null" EXIT
        for _ in $(seq 100); do [ -e "$HOME/daemon.ready" ] && break; sleep 0.1; done
        ./target/debug/kestic --no-autostart --smoke-m1 welcome --smoke-layout wide --smoke-lang en \
            --screenshot target/smoke-artifacts/m0-s06-main.png
    '

# Build binary and .deb package, then run workspace tests
test:
    cargo build --bin kestic
    cargo deb -p kestic-gui --fast
    just _nextest

# Workspace tests: fixtures first (a broken restic template fails once, clearly), then all tests;
# fixture dirs of this run and of runs older than a day are removed afterwards.
_nextest:
    #!/usr/bin/env bash
    set -euo pipefail
    export KESTIC_TEST_FIXTURE_RUN={{fixture_run}}
    trap 'rm -rf "{{fixtures_dir}}/{{fixture_run}}"' EXIT
    if [ -d "{{fixtures_dir}}" ]; then
        find "{{fixtures_dir}}" -mindepth 1 -maxdepth 1 -mtime +0 -exec rm -rf {} +
    fi
    cargo nextest run -p kestic-test-support
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
