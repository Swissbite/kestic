#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause

# Copies the screenshots of the AppStream metainfo into the assets of a release, under the fixed
# names of packaging/screenshots.tsv. The metainfo in the .deb links exactly these assets of its
# own release (build-stamp/src/screenshots.rs), so every listed image must be there.
#
#   release-screenshots.sh <list> <smoke screenshot dir> <asset dir>
#       For each line `name<TAB>source<TAB>caption` of <list> (blank lines and `#` lines skipped,
#       the same rules as build-stamp/src/screenshots.rs), copies <smoke screenshot dir>/<source>
#       to <asset dir>/<name>. Fails, before copying anything, for a line without exactly three
#       fields, a name that is not a plain `screenshot-….png` or listed twice, or a source that is
#       missing, no regular file, a symlink or no PNG.
#
# Sourced by test-release-screenshots.sh, which tests the function below.

set -euo pipefail

readonly PNG_SIGNATURE="89504e470d0a1a0a"

release_screenshots() {
    local list="$1" smoke="$2" assets="$3" line name source count=0
    local -a fields=() names=() sources=()
    local -A seen=()
    # `|| [ -n "$line" ]` keeps a last line without newline; a CR of a CRLF list is dropped.
    while IFS= read -r line || [ -n "$line" ]; do
        line="${line%$'\r'}"
        if [[ "$line" =~ ^[[:space:]]*$ ]] || [[ "$line" == \#* ]]; then
            continue
        fi
        IFS=$'\t' read -r -a fields <<< "$line"
        if [ "${#fields[@]}" -ne 3 ] || [[ "$line" == *$'\t\t'* ]]; then
            echo "::error::Malformed line in $list (name, source and caption separated by tabs): '$line'" >&2
            return 1
        fi
        name="${fields[0]}"
        source="${fields[1]}"
        if ! [[ "$name" =~ ^screenshot-[a-z0-9-]+\.png$ ]]; then
            echo "::error::Screenshot name '$name' in $list is not screenshot-<a-z0-9->.png" >&2
            return 1
        fi
        if [ -n "${seen[$name]+set}" ]; then
            echo "::error::Screenshot name $name is listed twice in $list" >&2
            return 1
        fi
        seen[$name]=1
        if ! [[ "$source" =~ ^[A-Za-z0-9_-][A-Za-z0-9._-]*\.png$ ]] || [ -L "$smoke/$source" ] ||
            [ ! -f "$smoke/$source" ]; then
            echo "::error::Screenshot $source for $name is missing in $smoke" >&2
            return 1
        fi
        if [ "$(head -c 8 -- "$smoke/$source" | od -An -tx1 | tr -d ' \n')" != "$PNG_SIGNATURE" ]; then
            echo "::error::$smoke/$source is no PNG file" >&2
            return 1
        fi
        names+=("$name")
        sources+=("$source")
    done < "$list"
    if [ "${#names[@]}" -eq 0 ]; then
        echo "::error::No screenshots listed in $list" >&2
        return 1
    fi
    mkdir -p -- "$assets"
    for count in "${!names[@]}"; do
        cp -- "$smoke/${sources[$count]}" "$assets/${names[$count]}"
        echo "${names[$count]} <- ${sources[$count]}"
    done
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    if [ "$#" -ne 3 ]; then
        echo "usage: $0 <list> <smoke screenshot dir> <asset dir>" >&2
        exit 2
    fi
    release_screenshots "$@"
fi
