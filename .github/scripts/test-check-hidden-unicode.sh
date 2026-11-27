#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause

# Tests check-hidden-unicode.sh on a temporary repository. Part of `just _static-checks`.
set -euo pipefail

check="$(cd "$(dirname "$0")" && pwd)/check-hidden-unicode.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
git -C "$tmp" init -q
failures=0

fail() {
    echo "FAIL: $*" >&2
    failures=$((failures + 1))
}

printf 'plain text, Umlaute äöü, «Anführung», emoji \xf0\x9f\x99\x82\n' > "$tmp/clean.md"
git -C "$tmp" add clean.md
"$check" "$tmp" > /dev/null 2>&1 || fail "clean file reported"

for case in "tag:\xf3\xa0\x81\x81:E0041" "bidi:\xe2\x80\xae:202E" "isolate:\xe2\x81\xa6:2066" "zero-width:\xe2\x80\x8b:200B" \
    "bom:\xef\xbb\xbf:FEFF" "line separator:\xe2\x80\xa8:2028" "soft hyphen:\xc2\xad:00AD"; do
    IFS=: read -r name bytes code <<< "$case"
    printf "line one\nsee %b here\n" "$bytes" > "$tmp/bad.txt"
    git -C "$tmp" add bad.txt
    rc=0
    out="$("$check" "$tmp" 2>/dev/null)" || rc=$?
    [[ "$rc" == 1 ]] || fail "$name: exit $rc"
    [[ "$out" == *"bad.txt:2: U+$code"* ]] || fail "$name: no finding with file, line and code point: '$out'"
done
git -C "$tmp" rm -q --cached bad.txt

printf 'untracked \xe2\x80\xae\n' > "$tmp/untracked.txt"
"$check" "$tmp" > /dev/null 2>&1 || fail "untracked file reported"
printf '\x00binary \xe2\x80\xae\n' > "$tmp/blob.bin"
git -C "$tmp" add blob.bin
"$check" "$tmp" > /dev/null 2>&1 || fail "binary file reported"

if ((failures > 0)); then
    echo "test-check-hidden-unicode: $failures failure(s)" >&2
    exit 1
fi
echo "test-check-hidden-unicode: ok"
