#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause

# Tests check-file-sizes.sh on a temp workspace: a `.rs`, `.qml` or `.cpp` file below a crate's
# `src/` with 1500 lines passes, one with 1501 lines fails and is named. Part of
# `just _static-checks`.
set -euo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
check="$here/check-file-sizes.sh"

failures=0
fail() {
    echo "FAIL: $1" >&2
    failures=$((failures + 1))
}

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# A fresh workspace with the source directories the check walks.
new_workspace() {
    rm -rf "$tmp/ws"
    for d in build-stamp core i18n daemon gui test-support; do
        mkdir -p "$tmp/ws/$d/src"
    done
}

# Writes $2 lines to the file $1 of the workspace.
lines() {
    mkdir -p "$(dirname -- "$tmp/ws/$1")"
    seq "$2" > "$tmp/ws/$1"
}

for ext in rs qml cpp; do
    file="gui/src/sub/big.$ext"

    new_workspace
    lines "$file" 1500
    if ! "$check" "$tmp/ws" 2> "$tmp/err"; then
        fail ".$ext with 1500 lines is refused: $(cat "$tmp/err")"
    fi

    new_workspace
    lines "$file" 1501
    if "$check" "$tmp/ws" 2> "$tmp/err"; then
        fail ".$ext with 1501 lines passes"
    elif ! grep -qF "$file has 1501 lines" "$tmp/err"; then
        fail ".$ext with 1501 lines is not named: $(cat "$tmp/err")"
    fi
done

# Only sources below `src/` count.
new_workspace
lines "gui/tests/big.cpp" 1501
if ! "$check" "$tmp/ws" 2> "$tmp/err"; then
    fail "a long file outside src/ is refused: $(cat "$tmp/err")"
fi

if [ "$failures" -ne 0 ]; then
    echo "$failures check(s) failed" >&2
    exit 1
fi
echo "check-file-sizes.sh: all checks passed"
