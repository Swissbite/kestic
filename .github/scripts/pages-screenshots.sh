#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause

# Finds and downloads the GUI smoke screenshots for the Pages site (pages.yml) into directory $1.
#
#   1. The `smoke-artifacts` artifact of the Verify run that started the workflow (EVENT_NAME
#      workflow_run, with RUN_ID and RUN_SHA), otherwise of the newest successful Verify run of a
#      push to main.
#   2. When that artifact has expired (the repository keeps artifacts for a few days) or there is
#      no such run: the bundle `kestic_<version>_screenshots.zip` of the newest release that has
#      one (release.yml attaches it).
#
# Prints `sha=<commit the screenshots were rendered from>` and `version=<release tag, empty for a
# Verify run>` for $GITHUB_OUTPUT, and fails with an error when neither source has screenshots.
# Needs `gh` with GH_TOKEN and GITHUB_REPOSITORY. Only PNG files are taken from the bundle, without
# their directories; nothing from the artifact or the bundle is executed.
set -euo pipefail

out="${1:?usage: pages-screenshots.sh <output directory>}"
repo="${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is not set}"
event="${EVENT_NAME:-}"

fail() {
    echo "::error::$1" >&2
    exit 1
}

# "<run id> <commit>" of the Verify run that provides the screenshots; empty when there is none.
verify_run() {
    if [ "$event" = workflow_run ]; then
        echo "${RUN_ID:?RUN_ID is not set} ${RUN_SHA:?RUN_SHA is not set}"
        return
    fi
    gh run list --repo "$repo" --workflow verify.yml --branch main --event push \
        --status success --limit 1 --json databaseId,headSha \
        --jq '.[] | "\(.databaseId) \(.headSha)"' ||
        fail "Could not list the Verify runs on main"
}

# Whether Verify run $1 still has the artifact `smoke-artifacts`.
has_screenshot_artifact() {
    local artifacts name expired
    artifacts="$(gh api "repos/$repo/actions/runs/$1/artifacts" --paginate \
        --jq '.artifacts[] | "\(.name) \(.expired)"')" ||
        fail "Could not list the artifacts of Verify run $1"
    while read -r name expired; do
        if [ "$name" = smoke-artifacts ] && [ "$expired" = false ]; then
            return 0
        fi
    done <<< "$artifacts"
    return 1
}

# "<tag> <asset>" of the newest published release with a screenshot bundle; empty when there is
# none. `gh release list` lists the newest release first.
release_bundle() {
    local tags tag assets asset
    tags="$(gh release list --repo "$repo" --limit 100 --json tagName,isDraft \
        --jq '.[] | select(.isDraft | not) | .tagName')" ||
        fail "Could not list the releases"
    while read -r tag; do
        [ -n "$tag" ] || continue
        assets="$(gh release view "$tag" --repo "$repo" --json assets --jq '.assets[].name')" ||
            fail "Could not list the assets of release $tag"
        while read -r asset; do
            if [[ "$asset" == kestic_*_screenshots.zip ]]; then
                echo "$tag $asset"
                return
            fi
        done <<< "$assets"
    done <<< "$tags"
}

mkdir -p "$out"
run="$(verify_run)"
read -r run_id sha <<< "$run" || true
if [ -n "${run_id:-}" ] && [ -n "${sha:-}" ] && has_screenshot_artifact "$run_id"; then
    gh run download "$run_id" --repo "$repo" --name smoke-artifacts --dir "$out" ||
        fail "Could not download the smoke-artifacts artifact of Verify run $run_id"
    version=""
    echo "Screenshots of Verify run $run_id at $sha" >&2
else
    if [ -n "${run_id:-}" ]; then
        reason="the smoke-artifacts artifact of Verify run $run_id has expired"
    else
        reason="there is no successful Verify run on main"
    fi
    bundle="$(release_bundle)"
    read -r version asset <<< "$bundle" || true
    if [ -z "${version:-}" ]; then
        fail "No screenshots for the site: $reason, and no release has a kestic_<version>_screenshots.zip"
    fi
    echo "::warning::Taking the screenshots of release $version: $reason" >&2
    bundle_dir="$(mktemp -d)"
    trap 'rm -rf "$bundle_dir"' EXIT
    gh release download "$version" --repo "$repo" --pattern "$asset" --dir "$bundle_dir" ||
        fail "Could not download $asset of release $version"
    unzip -q -j -o "$bundle_dir/$asset" '*.png' -d "$out" ||
        fail "$asset of release $version has no screenshots"
    sha="$(gh api "repos/$repo/commits/$version" --jq .sha)" ||
        fail "Could not find the commit of release $version"
    [ -n "$sha" ] || fail "Could not find the commit of release $version"
fi

if ! compgen -G "$out/*.png" > /dev/null; then
    fail "The screenshots for the site contain no PNG file"
fi
echo "sha=$sha"
echo "version=$version"
