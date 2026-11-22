#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause

# Version arithmetic of the release workflow on tags v<major>.<minor>.<patch>[-<pre-release>].
# Sourced by release-notes.sh and test-release-version.sh, or run directly:
#
#   release-version.sh next <bump>   next tag after the tags on stdin (one per line)
#   release-version.sh debian <tag>  Debian version of a tag, as build-stamp/src/version.rs maps it
#   release-version.sh max           highest version tag on stdin
#
# <bump> is major, minor, patch, premajor, preminor, prepatch or prerelease, with the meaning of
# npm's `semver inc`; pre-releases started here are release candidates (`-rc.<n>`). Lines on stdin
# that are no version tags are ignored. Versions are compared with `dpkg --compare-versions` on
# their Debian form, the order in which apt sees the packages.
set -euo pipefail

readonly RELEASE_TAG_RE='^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-([0-9A-Za-z]+(\.[0-9A-Za-z]+)*))?$'

# Whether $1 is a version tag.
is_version_tag() {
    [[ "$1" =~ $RELEASE_TAG_RE ]]
}

# Debian version of tag $1: v0.4.0 → 0.4.0, v0.4.0-m4.3 → 0.4.0~m4.3.
debian_version() {
    is_version_tag "$1" || return 1
    local version="${1#v}"
    if [[ "$version" == *-* ]]; then
        version="${version%%-*}~${version#*-}"
    fi
    echo "$version"
}

# Highest version tag among the lines on stdin; prints nothing without one.
max_tag() {
    local best="" tag
    while IFS= read -r tag; do
        is_version_tag "$tag" || continue
        if [ -z "$best" ] || dpkg --compare-versions "$(debian_version "$tag")" gt "$(debian_version "$best")"; then
            best="$tag"
        fi
    done
    echo "$best"
}

# Next tag after the highest version tag on stdin (v0.0.0 without one) for bump $1.
next_tag() {
    local bump="$1" base
    base="$(max_tag)"
    [[ "${base:-v0.0.0}" =~ $RELEASE_TAG_RE ]]
    local major="${BASH_REMATCH[1]}" minor="${BASH_REMATCH[2]}" patch="${BASH_REMATCH[3]}"
    local pre="${BASH_REMATCH[5]}"
    case "$bump" in
        major)
            if [ -z "$pre" ] || [ "$minor" -ne 0 ] || [ "$patch" -ne 0 ]; then
                major=$((major + 1)) minor=0 patch=0
            fi
            pre=""
            ;;
        minor)
            if [ -z "$pre" ] || [ "$patch" -ne 0 ]; then
                minor=$((minor + 1)) patch=0
            fi
            pre=""
            ;;
        patch)
            if [ -z "$pre" ]; then patch=$((patch + 1)); fi
            pre=""
            ;;
        premajor) major=$((major + 1)) minor=0 patch=0 pre="rc.1" ;;
        preminor) minor=$((minor + 1)) patch=0 pre="rc.1" ;;
        prepatch) patch=$((patch + 1)) pre="rc.1" ;;
        prerelease)
            if [[ "$pre" =~ ^rc\.([0-9]+)$ ]]; then
                pre="rc.$((BASH_REMATCH[1] + 1))"
            elif [ -n "$pre" ]; then
                pre="rc.1"
            else
                patch=$((patch + 1)) pre="rc.1"
            fi
            ;;
        *)
            echo "unknown bump '$bump'" >&2
            return 1
            ;;
    esac
    echo "v$major.$minor.$patch${pre:+-$pre}"
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    case "${1:-}" in
        next) next_tag "${2:?bump}" ;;
        debian) debian_version "${2:?tag}" ;;
        max) max_tag ;;
        *)
            echo "usage: release-version.sh next <bump> | debian <tag> | max" >&2
            exit 2
            ;;
    esac
fi
