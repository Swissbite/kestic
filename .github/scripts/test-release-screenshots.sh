#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause

# Tests the screenshot assets of a release (release-screenshots.sh) against temp folders, with the
# real packaging/screenshots.tsv: every listed name is attached, and a missing, linked or non-PNG
# source stops the release. Part of `just _static-checks`.
set -euo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd -- "$here/../.." && pwd)"
# shellcheck source=release-screenshots.sh
source "$here/release-screenshots.sh"

failures=0
check() {
    local expected="$1" actual="$2" what="$3"
    if [ "$expected" != "$actual" ]; then
        echo "FAIL: $what: expected '$expected', got '$actual'" >&2
        failures=$((failures + 1))
    fi
}

outcome() {
    if ("$@") > /dev/null 2>&1; then echo pass; else echo fail; fi
}

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
list="$root/packaging/screenshots.tsv"

# Smoke screenshots as the Verify run leaves them: every listed source as a PNG, plus another.
smoke="$tmp/smoke"
mkdir -p "$smoke"
mapfile -t names < <(grep -v '^#' "$list" | grep -v '^$' | cut -f1)
mapfile -t sources < <(grep -v '^#' "$list" | grep -v '^$' | cut -f2)
for source in "${sources[@]}" other-screen.png; do
    printf '\x89PNG\r\n\x1a\n%s' "$source" > "$smoke/$source"
done
check true "$([ "${#names[@]}" -gt 0 ] && echo true || echo false)" "the list names screenshots"

# AC-M6-23-4 (c): each listed name is attached, with the content of its source, and nothing else.
check pass "$(outcome release_screenshots "$list" "$smoke" "$tmp/assets")" "AC-M6-23-4 (c): attach"
for i in "${!names[@]}"; do
    check true "$(cmp -s "$smoke/${sources[$i]}" "$tmp/assets/${names[$i]}" && echo true || echo false)" \
        "AC-M6-23-4 (c): ${names[$i]} is ${sources[$i]}"
done
check "${#names[@]}" "$(find "$tmp/assets" -type f | wc -l)" "AC-M6-23-4 (c): only the listed images"

# AC-M6-23-4 (c): a missing source stops the release before anything is copied.
rm "$smoke/${sources[0]}"
check fail "$(outcome release_screenshots "$list" "$smoke" "$tmp/missing")" \
    "AC-M6-23-4 (c): missing ${sources[0]} fails"
check false "$([ -e "$tmp/missing" ] && echo true || echo false)" "AC-M6-23-4 (c): nothing copied"

# A symlink or a file that is no PNG is refused.
ln -s "$smoke/other-screen.png" "$smoke/${sources[0]}"
check fail "$(outcome release_screenshots "$list" "$smoke" "$tmp/link")" "symlinked source fails"
rm "$smoke/${sources[0]}"
printf 'not a png' > "$smoke/${sources[0]}"
check fail "$(outcome release_screenshots "$list" "$smoke" "$tmp/text")" "non-PNG source fails"

printf '\x89PNG\r\n\x1a\n%s' "${sources[0]}" > "$smoke/${sources[0]}"

# AC-M6-23-4 (c): the list is read like build-stamp reads it: a last line without newline and
# CRLF line ends still attach every name.
head -c -1 "$list" > "$tmp/no-newline.tsv"
check pass "$(outcome release_screenshots "$tmp/no-newline.tsv" "$smoke" "$tmp/nonl")" "no final newline"
check "${#names[@]}" "$(find "$tmp/nonl" -type f | wc -l)" "AC-M6-23-4 (c): last line without newline attached"
sed 's/$/\r/' "$list" > "$tmp/crlf.tsv"
check pass "$(outcome release_screenshots "$tmp/crlf.tsv" "$smoke" "$tmp/crlf")" "CRLF list"
check true "$([ -f "$tmp/crlf/${names[0]}" ] && echo true || echo false)" "AC-M6-23-4 (c): CRLF names without CR"

# Malformed lists are refused, as by build-stamp: a name outside the fixed pattern, a repeated
# name, four fields, two fields.
bad() {
    printf "$1" > "$tmp/bad.tsv"
    check fail "$(outcome release_screenshots "$tmp/bad.tsv" "$smoke" "$tmp/bad-$2")" "$2 fails"
}
bad 'Overview.png\tother-screen.png\tOverview\n' "malformed name"
bad 'screenshot-a.png\tother-screen.png\tA\nscreenshot-a.png\tother-screen.png\tB\n' "repeated name"
bad 'screenshot-a.png\tother-screen.png\tA\tB\n' "four fields"
bad 'screenshot-a.png\tother-screen.png\n' "two fields"

if [ "$failures" -ne 0 ]; then
    echo "$failures check(s) failed" >&2
    exit 1
fi
echo "release screenshots: all checks passed"
