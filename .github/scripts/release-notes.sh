#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause

# Plans a release and writes the hand-made part of its notes. Two kinds:
#
#   release-notes.sh release <bump> <commit> <out-dir>
#       The next version after the version tags on main (release-version.sh). A release needs
#       the highlights file docs/releases/<tag>.md (format in docs/releases/README.md); a release
#       candidate uses docs/releases/v<major>.<minor>.<patch>.md when it exists.
#   release-notes.sh branch <branch> <commit> <out-dir>
#       A pre-release of the integration branch m<n>: v0.<n>.0-m<n>.<k>, k being the number of
#       first-parent commits since the tag m<n>-start. Its notes name the closed story issues
#       that the pull request merged by <commit> refers to.
#
# Writes <out-dir>/notes-head.md and prints only `key=value` lines to stdout, for $GITHUB_OUTPUT: tag, version
# (Debian form), prerelease, title, previous-tag (start of GitHub's generated notes). Needs git
# with all tags and origin/main, gh (GH_TOKEN), jq and dpkg; the repository comes from
# GITHUB_REPOSITORY or `gh repo view`. Errors and warnings go to stderr as workflow commands.
set -euo pipefail

# shellcheck source=release-version.sh
source "$(dirname "$0")/release-version.sh"

error() {
    echo "::error::$*" >&2
    exit 1
}

warning() {
    echo "::warning::$*" >&2
}

[ $# -eq 4 ] || error "usage: release-notes.sh release <bump> <commit> <out-dir> | branch <branch> <commit> <out-dir>"
kind="$1"
commit="$(git rev-parse --verify "$3^{commit}")"
out="$4"
repo="${GITHUB_REPOSITORY:-$(gh repo view --json nameWithOwner --jq .nameWithOwner)}"
mkdir -p "$out"
head="$out/notes-head.md"
: >"$head"

all_milestones="$(gh api "repos/$repo/milestones?state=all&per_page=100" --paginate | jq -s add)"

# The GitHub milestone titled exactly $1, as JSON; empty when there is none.
milestone_by_title() {
    jq -c --arg t "$1" '[.[] | select(.title == $t)] | first // empty' <<<"$all_milestones"
}

# Markdown list `- <title> ([#<n>](<url>))` of the issues in the JSON arrays on stdin (objects
# with title, number and html_url); `- none` without any.
issue_list() {
    jq -rs '[.[][]] | if length == 0 then "- none"
            else .[] | "- \(.title) ([#\(.number)](\(.html_url)))" end'
}

# Front matter of highlights file $1: `milestones:` with one `  - <title>` line per GitHub
# milestone, optional `title: <text>`. Anything else is an error, so a typo never passes silently.
# Prints `milestone<TAB>…` and `title<TAB>…` lines and appends the highlights to notes-head.md.
parse_highlights() {
    awk -v file="$1" -v head="$head" '
        function fail(msg) { print "::error::" file ": " msg " (format in docs/releases/README.md)" > "/dev/stderr"; failed = 1; exit 1 }
        function trim(s) { sub(/^[ \t]+/, "", s); sub(/[ \t]+$/, "", s); return s }
        NR == 1 { if ($0 != "---") fail("must start with a front matter line ---"); next }
        !done && $0 == "---" { done = 1; next }
        done { body = body $0 "\n"; next }
        /^milestones:[ \t]*$/ { in_list = 1; next }
        in_list && /^[ \t]+-[ \t]/ { sub(/^[ \t]+-[ \t]+/, ""); m = trim($0); if (m == "") fail("empty milestone"); print "milestone\t" m; next }
        /^title:/ { in_list = 0; sub(/^title:/, ""); t = trim($0); if (t == "") fail("empty title"); print "title\t" t; next }
        /^[ \t]*(#.*)?$/ { next }
        { fail("unexpected front matter line: " $0) }
        END {
            if (failed) exit 1
            if (!done) fail("front matter is not closed with ---")
            gsub(/^\n+|\n+$/, "", body)
            if (body == "") fail("has no highlights below the front matter")
            print body >> head
        }
    ' "$1"
}

# Notes of release or release candidate $1 from highlights file $2, which may be missing when $3
# is `optional`. Appends highlights and closed stories to notes-head.md and prints the title that
# follows the tag (empty without highlights).
release_notes() {
    local tag="$1" highlights="$2" required="$3"
    if [ ! -f "$highlights" ]; then
        [ "$required" = optional ] ||
            error "$highlights is missing. The closing pull request of the milestone adds it (see docs/releases/README.md)."
        echo "Release candidate for ${tag%%-*}." >>"$head"
        return
    fi
    local parsed title name milestones
    parsed="$(parse_highlights "$highlights")" || exit 1
    mapfile -t milestones < <(printf '%s\n' "$parsed" | awk -F'\t' '$1 == "milestone" { print $2 }')
    title="$(printf '%s\n' "$parsed" | awk -F'\t' '$1 == "title" { print $2; exit }')"
    [ "${#milestones[@]}" -gt 0 ] || error "$highlights names no milestone (front matter key milestones)"

    local minor
    minor="$(tag_minor "$tag")"
    if [ "$(tag_major "$tag")" -eq 0 ]; then
        # A 0.x release belongs to exactly one milestone: v0.<n>.<patch> to M<n>.
        [ "${#milestones[@]}" -eq 1 ] || error "$highlights: a 0.x release names exactly one milestone"
        [[ "${milestones[0]}" == "M$minor "* ]] ||
            error "$highlights: $tag belongs to milestone M$minor, but the file names '${milestones[0]}'"
    fi
    if [ -z "$title" ]; then
        [ "${#milestones[@]}" -eq 1 ] || error "$highlights: set title when naming several milestones"
        title="${milestones[0]}"
    fi

    printf '\n## Stories\n' >>"$head"
    for name in "${milestones[@]}"; do
        local milestone number url stories open
        milestone="$(milestone_by_title "$name")"
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
        jq -c '[.[] | select(.state == "closed" and (.state_reason // "completed") == "completed")]' \
            <<<"$stories" | issue_list >>"$head"
    done
    echo "$title"
}

case "$kind" in
    release)
        bump="$2"
        # A release tag on this commit means it was released already, or a run with this commit
        # is being re-run as a whole; re-running only the failed jobs keeps the planned tag.
        released="$(git tag --points-at "$commit" --list 'v*' | grep -v -- '-m[0-9]' || true)"
        [ -z "$released" ] ||
            error "$commit already carries $released. To finish a failed release, re-run its failed jobs."
        tags="$(git tag --merged origin/main --list 'v*')"
        tag="$(next_tag "$bump" <<<"$tags")" || error "Unknown bump '$bump'"
        if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
            error "Tag $tag exists already outside main; pick another bump"
        fi
        if [[ "$tag" == *-* ]]; then
            title="$(release_notes "$tag" "docs/releases/${tag%%-*}.md" optional)"
        else
            title="$(release_notes "$tag" "docs/releases/$tag.md" required)"
        fi
        # Generated notes cover everything since the previous release, pre-releases included.
        previous="$(grep -v -- - <<<"$tags" | max_tag || true)"
        ;;
    branch)
        branch="$2"
        [[ "$branch" =~ ^m([0-9]+)$ ]] || error "'$branch' is no integration branch m<n>"
        n="${BASH_REMATCH[1]}"
        start="m$n-start"
        git rev-parse -q --verify "refs/tags/$start" >/dev/null || error "Tag $start is missing"
        git merge-base --is-ancestor "$start" "$commit" || error "$commit does not descend from $start"
        k="$(git rev-list --first-parent --count "$start..$commit")"
        tag="v0.$n.0-m$n.$k"
        if existing="$(git rev-parse -q --verify "refs/tags/$tag^{commit}")" &&
            [ "$existing" != "$commit" ]; then
            error "Tag $tag exists already on another commit ($existing)"
        fi

        short="$(git rev-parse --short=7 "$commit")"
        milestone="$(jq -c --arg p "M$n " '[.[] | select(.title | startswith($p))] | first // empty' <<<"$all_milestones")"
        if [ -n "$milestone" ]; then
            title="$(jq -r .title <<<"$milestone")"
            printf 'Pre-release of milestone [%s](%s), built from %s. The release v0.%s.0 replaces it.\n' \
                "$title" "$(jq -r .html_url <<<"$milestone")" "$short" "$n" >>"$head"
        else
            title=""
            printf 'Pre-release of milestone M%s, built from %s. The release v0.%s.0 replaces it.\n' \
                "$n" "$short" "$n" >>"$head"
        fi

        # Story issues of this milestone that the pull request merged by this commit refers to
        # (`#<n>` in title or body) and that are closed. Closing keywords do not link issues to
        # pull requests into a branch other than main, so the references are read directly.
        printf '\n## Stories in this pre-release\n\n' >>"$head"
        pulls="$(gh api "repos/$repo/commits/$commit/pulls" --jq '.[] | select(.merged_at != null) | .number')"
        refs="$(for pull in $pulls; do
            gh api "repos/$repo/pulls/$pull" --jq '"\(.title)\n\(.body // "")"' | grep -o '#[0-9]\+' || true
        done | tr -d '#' | sort -un)"
        # A reference to nothing (404) is skipped.
        for ref in $refs; do
            gh api "repos/$repo/issues/$ref" 2>/dev/null | jq -c --arg p "^M$n-S[0-9]+:" \
                '[select(.pull_request == null and .state == "closed" and (.title | test($p)))]' || true
        done | issue_list >>"$head"

        previous="$(git tag --list "v0.$n.0-m$n.*" --merged "$commit" | grep -vx -- "$tag" | max_tag || true)"
        previous="${previous:-$start}"
        ;;
    *)
        error "unknown kind '$kind'"
        ;;
esac

prerelease=false
if [ "$(tag_major "$tag")" -eq 0 ] || tag_is_prerelease "$tag"; then prerelease=true; fi

echo "tag=$tag"
echo "version=$(debian_version "$tag")"
echo "prerelease=$prerelease"
echo "title=$tag${title:+ – $title}"
echo "previous-tag=$previous"
