#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause

# Tests the version arithmetic of the release workflow (release-version.sh). Part of
# `just _static-checks`.
set -euo pipefail

# shellcheck source=release-version.sh
source "$(dirname "$0")/release-version.sh"

failures=0
check() {
    local expected="$1" actual="$2" what="$3"
    if [ "$expected" != "$actual" ]; then
        echo "FAIL: $what: expected '$expected', got '$actual'" >&2
        failures=$((failures + 1))
    fi
}

next() {
    local bump="$1"
    shift
    printf '%s\n' "$@" | next_tag "$bump"
}

# Debian form, the same as build-stamp/src/version.rs gives for a tagged commit.
check 0.4.0 "$(debian_version v0.4.0)" "debian of a release"
check 0.4.0~m4.3 "$(debian_version v0.4.0-m4.3)" "debian of a branch pre-release"
check 1.0.0~rc.2 "$(debian_version v1.0.0-rc.2)" "debian of a release candidate"
if debian_version v0.4 > /dev/null; then check fail ok "debian of an invalid tag"; fi

# The highest tag in apt order; other tags are ignored.
check v0.4.0 "$(printf '%s\n' v0.4.0-m4.12 m4-start v0.4.0 v0.4.0-rc.1 v0.3.9 | max_tag)" "max: release after its pre-releases"
check v0.4.0-m4.10 "$(printf '%s\n' v0.4.0-m4.9 v0.4.0-m4.10 v0.4.0-m4.2 | max_tag)" "max: numeric pre-release order"
check "" "$(printf '%s\n' m4-start m5-start | max_tag)" "max: no version tag"

# Releases from the milestone pre-releases: a minor bump turns the pre-release into its release.
check v0.4.0 "$(next minor v0.4.0-m4.12 m4-start)" "minor after a branch pre-release"
check v0.5.0 "$(next minor v0.4.0)" "minor after a release"
check v0.1.0 "$(next minor)" "minor without any tag"
check v1.0.0 "$(next major v0.6.0)" "major after a 0.x release"
check v1.0.0 "$(next major v1.0.0-rc.3)" "major after its release candidate"
check v2.0.0 "$(next major v1.2.0-rc.1)" "major after a minor release candidate"
check v0.4.1 "$(next patch v0.4.0)" "patch"
check v0.4.1 "$(next patch v0.4.1-rc.1)" "patch after its release candidate"
check v0.5.0 "$(next minor v0.4.1-rc.1)" "minor after a patch release candidate"

# Release candidates.
check v1.0.0-rc.1 "$(next premajor v0.6.0)" "premajor"
check v0.5.0-rc.1 "$(next preminor v0.4.0)" "preminor"
check v0.4.1-rc.1 "$(next prepatch v0.4.0)" "prepatch"
check v0.4.0-rc.2 "$(next prerelease v0.4.0-rc.1 v0.4.0-m4.12)" "prerelease after a release candidate"
check v0.4.0-rc.1 "$(next prerelease v0.4.0-m4.12)" "prerelease after a branch pre-release"
check v0.4.1-rc.1 "$(next prerelease v0.4.0)" "prerelease after a release"
if next sideways v0.4.0 > /dev/null 2>&1; then check fail ok "unknown bump"; fi

if [ "$failures" -ne 0 ]; then
    exit 1
fi
echo "release-version.sh: all checks passed"
