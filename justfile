# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause

set fallback := false

# One id per `just` invocation: test fixtures (template repositories with generated passwords)
# live in target/kestic-test-fixtures/<id> and are removed after the run.
fixture_run := uuid()
fixtures_dir := absolute_path(env_var_or_default("CARGO_TARGET_DIR", "target")) / "kestic-test-fixtures"

# List available recipes
default:
    @just --list

# The CI runs the same recipes in parallel jobs (.github/workflows/verify.yml).
# Full suite pass (fmt, file sizes, deny, reuse, clippy, deb build, smoke, nextest)
verify:
    just _static-checks
    just _clippy
    just deb
    just _build-test-harness
    just _smoke-run
    just _nextest

# Checks that build nothing: formatting, file sizes (and their check), licenses and advisories, REUSE, the merge
# rule, the watcher and the GitHub token of the agent container, the release version arithmetic,
# the release asset names, screenshots and flags, the job selection of the Verify workflow and the
# screenshot source of the Pages site
_static-checks:
    cargo fmt --all -- --check
    just _check-file-sizes
    scripts/test-check-file-sizes.sh
    cargo deny check
    reuse lint
    .devcontainer/test-gh-wrapper.sh
    .devcontainer/test-kestic-agent-watch.sh
    .devcontainer/test-kestic-agent-token.sh
    .github/scripts/test-release-version.sh
    .github/scripts/test-release-assets.sh
    .github/scripts/test-release-screenshots.sh
    .github/scripts/test-release-flags.sh
    .github/scripts/test-ci-scope.sh
    .github/scripts/test-pages-screenshots.sh

# clippy without and with the test harness
_clippy:
    cargo clippy --workspace --all-targets -- -D warnings
    cargo clippy --workspace --all-targets --features kestic-gui/test-harness -- -D warnings

_check-file-sizes:
    scripts/check-file-sizes.sh

# Format all Rust code in the workspace
fmt:
    cargo fmt --all

# Build the Debian package (fast profile). Paths of the build machine stay out of the shipped
# binary: Rust panic locations (workspace, Cargo registry, toolchain sources) and `__FILE__` in
# C/C++ code are remapped to fixed prefixes, so the package neither leaks them nor depends on them.
# A plain `cargo deb` skips this remapping. Flags from RUSTFLAGS are kept. The package version
# comes from the Git tags or KESTIC_VERSION, the same one the binary shows
# (build-stamp/src/version.rs), with Debian revision 1. The AppStream metainfo gets the
# screenshots of packaging/screenshots.tsv as assets of that version's release and its release
# entry (build-stamp/src/screenshots.rs, metainfo.rs), then the German texts of the catalog, and
# must pass `appstreamcli validate` without warnings; cargo-deb takes it from the release profile
# directory like the binary.
deb:
    #!/usr/bin/env bash
    set -euo pipefail
    version="$(cargo run --quiet -p kestic-build-stamp)"
    workspace="$(pwd -P)"
    target="$(realpath -m "${CARGO_TARGET_DIR:-target}")"
    cargo_home="${CARGO_HOME:-$HOME/.cargo}"
    rustup_home="${RUSTUP_HOME:-$HOME/.rustup}"
    for dir in "$workspace" "$target" "$cargo_home" "$rustup_home"; do
        if [[ "$dir" =~ [[:space:]] ]]; then
            echo "ERROR: '$dir' contains whitespace; CFLAGS cannot remap it" >&2
            exit 1
        fi
    done
    read -ra rust_flags <<< "${RUSTFLAGS:-}"
    c_flags=()
    for map in "$cargo_home=/cargo" "$rustup_home=/rustup" "$workspace=/kestic" "$target=/kestic/target"; do
        rust_flags+=("--remap-path-prefix=$map")
        c_flags+=("-ffile-prefix-map=$map")
    done
    metainfo_tmp="$(mktemp -d)"
    trap 'rm -rf "$metainfo_tmp"' EXIT
    metainfo="$target/release/net.daester.Kestic.metainfo.xml"
    mkdir -p "$target/release"
    KESTIC_VERSION="$version" cargo run --quiet -p kestic-build-stamp -- metainfo \
        packaging/net.daester.Kestic.metainfo.xml packaging/screenshots.tsv \
        "$metainfo_tmp/net.daester.Kestic.metainfo.xml"
    msgfmt --xml -l de --template "$metainfo_tmp/net.daester.Kestic.metainfo.xml" \
        -o "$metainfo" po/de/kestic.po
    appstreamcli validate --no-net --explain "$metainfo"
    KESTIC_VERSION="$version" \
    CARGO_ENCODED_RUSTFLAGS="$(IFS=$'\x1f'; echo "${rust_flags[*]}")" \
    CFLAGS="${CFLAGS:-} ${c_flags[*]}" \
    CXXFLAGS="${CXXFLAGS:-} ${c_flags[*]}" \
        cargo deb -p kestic-gui --fast --deb-version "$version-1"

# Start the main window offscreen (QML smoke) and write a screenshot; builds only the binary
smoke:
    cargo build --bin kestic --features kestic-gui/test-harness
    just _smoke-run

# The test-harness `kestic` and every test binary in one build. Building them together gives
# them the same dependency features (dev-dependencies change them), so the tests reuse the
# binary's crates instead of compiling them, including the C++ code, a second time.
_build-test-harness:
    cargo build --workspace --bins --tests --features kestic-gui/test-harness

# Runs the test-harness `kestic` offscreen; `smoke` or `_build-test-harness` builds it first
_smoke-run:
    #!/usr/bin/env bash
    set -euo pipefail
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
    just _build-test-harness
    just deb
    just _nextest

# Workspace tests: fixtures first (a broken restic template fails once, clearly), then all other
# tests; fixture dirs of this run and of runs older than a day are removed afterwards. Both runs
# use the same package selection and features, so the second one builds nothing new.
_nextest:
    #!/usr/bin/env bash
    set -euo pipefail
    export KESTIC_TEST_FIXTURE_RUN={{fixture_run}}
    trap 'rm -rf "{{fixtures_dir}}/{{fixture_run}}"' EXIT
    if [ -d "{{fixtures_dir}}" ]; then
        find "{{fixtures_dir}}" -mindepth 1 -maxdepth 1 -mtime +0 -exec rm -rf {} +
    fi
    cargo nextest run --workspace --features kestic-gui/test-harness -E 'package(kestic-test-support)'
    cargo nextest run --workspace --features kestic-gui/test-harness -E 'not package(kestic-test-support)'

# Only the tests that read files under docs/spec/ (CI, for pull requests that change only the
# specification; .github/scripts/ci-scope.sh lists them)
_spec-tests:
    #!/usr/bin/env bash
    set -euo pipefail
    tests="$(.github/scripts/ci-scope.sh spec-tests)"
    args=()
    while read -r package binary; do
        args+=(-p "$package" --test "$binary")
    done <<< "$tests"
    cargo nextest run "${args[@]}"

# Packs the test binaries of `_build-test-harness` into a nextest archive (CI)
_nextest-archive archive:
    cargo nextest archive --workspace --features kestic-gui/test-harness --archive-file {{quote(archive)}}

# Runs the tests of a nextest archive that match `filter` (a nextest filterset), optionally only
# one `partition` of them. The archive unpacks into ./target, so it must come from a checkout at
# the same path (CI)
_nextest-from-archive archive filter partition="":
    cargo nextest run --archive-file {{quote(archive)}} --workspace-remap . --extract-to . \
        --extract-overwrite -E {{quote(filter)}} \
        {{ if partition == "" { "" } else { "--partition " + quote(partition) } }}

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
