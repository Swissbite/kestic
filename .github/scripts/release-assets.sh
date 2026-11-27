#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause

# Names the assets of a release as GitHub keeps them and writes or checks their SHA256SUMS.
#
# GitHub replaces special characters in the name of an uploaded asset with `.`: the pre-release
# package kestic_0.6.0~m6.3-1_amd64.deb becomes kestic_0.6.0.m6.3-1_amd64.deb. A SHA256SUMS that
# names the local file then fails `sha256sum -c` next to the downloaded assets. So the assets get
# their upload name before they are hashed, attested and uploaded; names without such characters
# (every final release) stay as they are.
#
#   release-assets.sh prepare <dir>
#       Renames every file in <dir> to its upload name, writes <dir>/SHA256SUMS and checks it.
#   release-assets.sh check <dir>
#       Checks <dir>/SHA256SUMS against the files in <dir>: every listed file matches, and every
#       file but SHA256SUMS is listed.
#
# Sourced by test-release-assets.sh, which tests the functions below.

set -euo pipefail

# The name GitHub keeps for an asset named $1: every character but letters, digits, `.`, `_` and
# `-` becomes `.`. GitHub also trims leading and trailing dots, and nothing documents what it does
# with two dots in a row, so such names fail instead of being guessed.
upload_name() {
    local name="$1" upload
    upload="$(LC_ALL=C sed 's/[^A-Za-z0-9._-]/./g' <<< "$name")"
    if [ -z "$upload" ] || [[ "$upload" == .* || "$upload" == *. || "$upload" == *..* ]]; then
        echo "::error::No unambiguous upload name for the asset '$name' (would be '$upload')" >&2
        return 1
    fi
    echo "$upload"
}

# Renames every file in $1 to its upload name, writes $1/SHA256SUMS and checks it. Stops before
# renaming anything when two files would get the same name.
release_assets_prepare() {
    local dir="$1" file name sums
    local -A target=() source=()
    shopt -s nullglob
    local files=("$dir"/*)
    shopt -u nullglob
    if [ "${#files[@]}" -eq 0 ]; then
        echo "::error::No assets in $dir" >&2
        return 1
    fi
    for file in "${files[@]}"; do
        file="${file##*/}"
        if [ "$file" = SHA256SUMS ] || [ ! -f "$dir/$file" ] || [ -L "$dir/$file" ]; then
            echo "::error::$dir/$file is no asset (a regular file other than SHA256SUMS)" >&2
            return 1
        fi
        name="$(upload_name "$file")" || return 1
        if [ -n "${source[$name]+set}" ]; then
            echo "::error::$file and ${source[$name]} would both be uploaded as $name" >&2
            return 1
        fi
        source[$name]="$file"
        target[$file]="$name"
    done
    for file in "${!target[@]}"; do
        if [ "$file" != "${target[$file]}" ]; then
            mv -n -- "$dir/$file" "$dir/${target[$file]}" || return 1
            echo "Renamed $file to ${target[$file]}" >&2
        fi
    done
    sums="$(cd "$dir" && LC_ALL=C sha256sum -- "${target[@]}")" || return 1
    LC_ALL=C sort -k2 <<< "$sums" > "$dir/SHA256SUMS" || return 1
    release_assets_check "$dir"
}

# Checks $1/SHA256SUMS against the files in $1: every listed file is there and matches, and every
# file but SHA256SUMS is listed.
release_assets_check() {
    local dir="$1" listed present
    if [ ! -s "$dir/SHA256SUMS" ]; then
        echo "::error::$dir/SHA256SUMS is missing or empty" >&2
        return 1
    fi
    if ! (cd "$dir" && sha256sum --check --strict SHA256SUMS) >&2; then
        echo "::error::The assets in $dir do not match SHA256SUMS" >&2
        return 1
    fi
    listed="$(LC_ALL=C sed -E 's/^[0-9a-f]{64} [ *]//' "$dir/SHA256SUMS" | LC_ALL=C sort)"
    present="$(cd "$dir" && find . -mindepth 1 -maxdepth 1 ! -name SHA256SUMS -printf '%P\n' | LC_ALL=C sort)"
    if [ "$listed" != "$present" ]; then
        echo "::error::SHA256SUMS in $dir does not list exactly the assets:" >&2
        diff <(echo "$listed") <(echo "$present") >&2 || true
        return 1
    fi
}

release_assets_main() {
    local command="${1:-}"
    case "$command" in
        prepare) release_assets_prepare "${2:?directory missing}" ;;
        check) release_assets_check "${2:?directory missing}" ;;
        *)
            echo "usage: $0 prepare <dir> | check <dir>" >&2
            return 2
            ;;
    esac
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    release_assets_main "$@"
fi
