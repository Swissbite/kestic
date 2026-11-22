#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause

# Checks a release tag and writes the hand-made part of its release notes.
#
# Usage: release-notes.sh <tag> <commit> <out-dir>
#
# Fails when the tag is not `v<major>.<minor>.<patch>`, differs from the version in the workspace
# Cargo.toml, or has no valid highlights file docs/releases/<tag>.md (format in
# docs/releases/README.md). Writes <out-dir>/notes-head.md (highlights, then the closed story
# issues of the milestones named in the highlights file) and prints `key=value` lines for
# $GITHUB_OUTPUT: version, prerelease, title, previous-tag. Needs git, gh (GH_TOKEN) and jq; the
# repository comes from GITHUB_REPOSITORY or `gh repo view`. Errors and warnings go to stderr as
# workflow commands.
set -euo pipefail

error() {
    echo "::error::$*" >&2
    exit 1
}

warning() {
    echo "::warning::$*" >&2
}

[ $# -eq 3 ] || error "usage: release-notes.sh <tag> <commit> <out-dir>"
tag="$1"
commit="$2"
out="$3"
repo="${GITHUB_REPOSITORY:-$(gh repo view --json nameWithOwner --jq .nameWithOwner)}"

semver='^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'
[[ "$tag" =~ $semver ]] || error "Tag '$tag' is not v<major>.<minor>.<patch>"
version="${tag#v}"
major="${BASH_REMATCH[1]}"
minor="${BASH_REMATCH[2]}"

# The .deb takes its version from kestic-gui, which inherits [workspace.package] version.
grep -qx 'version.workspace = true' gui/Cargo.toml ||
    error "gui/Cargo.toml no longer inherits the workspace version (version.workspace = true)"
cargo_version="$(awk '
    /^\[/ { in_section = ($0 == "[workspace.package]") }
    in_section && /^version *= *"/ { sub(/^version *= *"/, ""); sub(/".*/, ""); print; exit }
' Cargo.toml)"
[ -n "$cargo_version" ] || error "No version in [workspace.package] of Cargo.toml"
[ "$cargo_version" = "$version" ] ||
    error "Tag $tag does not match version $cargo_version in Cargo.toml ([workspace.package]). Bump the version in the closing pull request of the milestone, then tag."

highlights="docs/releases/$tag.md"
[ -f "$highlights" ] ||
    error "$highlights is missing. The closing pull request of the milestone adds it (see docs/releases/README.md)."

# Front matter: `milestones:` with one `  - <title>` line per GitHub milestone, optional
# `title: <text>`. Anything else is an error, so a typo never passes silently.
parsed="$(awk -v file="$highlights" '
    function fail(msg) { print "::error::" file ": " msg " (format in docs/releases/README.md)" > "/dev/stderr"; failed = 1; exit 1 }
    function unquote(s) {
        sub(/^[ \t]+/, "", s); sub(/[ \t]+$/, "", s)
        if (s ~ /^".*"$/ || s ~ /^\047.*\047$/) s = substr(s, 2, length(s) - 2)
        return s
    }
    NR == 1 { if ($0 != "---") fail("must start with a front matter line ---"); next }
    !done && $0 == "---" { done = 1; next }
    done { body = body $0 "\n"; next }
    /^milestones:[ \t]*$/ { in_list = 1; next }
    in_list && /^[ \t]+-[ \t]/ { sub(/^[ \t]+-[ \t]+/, ""); m = unquote($0); if (m == "") fail("empty milestone"); print "milestone\t" m; next }
    /^title:/ { in_list = 0; sub(/^title:/, ""); t = unquote($0); if (t == "") fail("empty title"); print "title\t" t; next }
    /^[ \t]*(#.*)?$/ { next }
    { fail("unexpected front matter line: " $0) }
    END {
        if (failed) exit 1
        if (!done) fail("front matter is not closed with ---")
        gsub(/^\n+|\n+$/, "", body)
        if (body == "") fail("has no highlights below the front matter")
        print "body\t" length(body)
    }
' "$highlights")" || exit 1

mapfile -t milestones < <(printf '%s\n' "$parsed" | awk -F'\t' '$1 == "milestone" { print $2 }')
title="$(printf '%s\n' "$parsed" | awk -F'\t' '$1 == "title" { print $2; exit }')"
[ "${#milestones[@]}" -gt 0 ] || error "$highlights names no milestone (front matter key milestones)"

if [ "$major" -eq 0 ]; then
    # A 0.x release belongs to exactly one milestone: v0.<n>.<patch> to M<n>.
    [ "${#milestones[@]}" -eq 1 ] || error "$highlights: a 0.x release names exactly one milestone"
    [[ "${milestones[0]}" == "M$minor "* ]] ||
        error "$highlights: $tag belongs to milestone M$minor, but the file names '${milestones[0]}'"
    prerelease=true
else
    prerelease=false
fi
if [ -z "$title" ]; then
    [ "${#milestones[@]}" -eq 1 ] || error "$highlights: set title when naming several milestones"
    title="${milestones[0]}"
fi

all_milestones="$(gh api "repos/$repo/milestones?state=all&per_page=100" --paginate | jq -s add)"

mkdir -p "$out"
head="$out/notes-head.md"
awk 'f { print } /^---$/ && ++n == 2 { f = 1 }' "$highlights" |
    awk 'NF { started = 1 } started' >"$head"
printf '\n## Stories\n' >>"$head"

for name in "${milestones[@]}"; do
    milestone="$(jq -c --arg t "$name" '[.[] | select(.title == $t)] | first // empty' <<<"$all_milestones")"
    [ -n "$milestone" ] || error "$highlights: no GitHub milestone titled '$name'"
    number="$(jq -r .number <<<"$milestone")"
    url="$(jq -r .html_url <<<"$milestone")"

    # Story issues are titled `M<n>-S<nn>: …`; pull requests and other issues are left out.
    stories="$(gh api "repos/$repo/issues?milestone=$number&state=all&per_page=100" --paginate |
        jq -s '[.[][] | select(.pull_request == null)
                      | select(.title | test("^M[0-9]+-S[0-9]+:"))
                      | . + (.title | capture("^M(?<m>[0-9]+)-S(?<s>[0-9]+)") | {m: (.m | tonumber), s: (.s | tonumber)})]
               | sort_by(.m, .s)')"
    open="$(jq -r '[.[] | select(.state == "open") | .title] | join("; ")' <<<"$stories")"
    [ -z "$open" ] || warning "Milestone '$name' still has open stories: $open"

    if [ "${#milestones[@]}" -gt 1 ]; then
        printf '\n### %s\n' "$name" >>"$head"
    fi
    printf '\nClosed stories of milestone [%s](%s):\n\n' "$name" "$url" >>"$head"
    jq -r '[.[] | select(.state == "closed" and (.state_reason // "completed") == "completed")]
           | if length == 0 then "- none" else .[] | "- \(.title) ([#\(.number)](\(.html_url)))" end' \
        <<<"$stories" >>"$head"
done

# Generated notes start at the previous version tag before this commit; without one they cover
# every pull request so far.
previous="$(git describe --tags --abbrev=0 --match 'v[0-9]*' --exclude "$tag" "$commit" 2>/dev/null || true)"

echo "version=$version"
echo "prerelease=$prerelease"
echo "title=$tag – $title"
echo "previous-tag=$previous"
