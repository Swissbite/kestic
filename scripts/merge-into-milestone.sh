#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause

# Merges a pull request into a milestone branch (m<n>) and nowhere else.
#
#   scripts/merge-into-milestone.sh <pr-number>
#
# Merges only when all of these hold, otherwise it stops without changing anything:
#   - the base branch is m<n> (never the default branch),
#   - the pull request is open, no draft, and its head is still the commit checked below,
#   - the latest `verify` and `tripwire` check runs on that commit succeeded,
#   - the head contains the current base (CI ran on the current milestone state).
# Squash merge with --match-head-commit, so a push after the checks cannot slip in.
# Needs `gh` (logged in) and `jq`.
set -euo pipefail

die() {
    echo "merge-into-milestone: $*" >&2
    exit 1
}

[ "$#" -eq 1 ] || die "usage: $0 <pr-number>"
pr="$1"
[[ "$pr" =~ ^[0-9]+$ ]] || die "not a pull request number: $pr"

repo="$(gh repo view --json nameWithOwner --jq .nameWithOwner)"
default_branch="$(gh repo view --json defaultBranchRef --jq .defaultBranchRef.name)"
info="$(gh pr view "$pr" --repo "$repo" \
    --json number,state,isDraft,baseRefName,headRefName,headRefOid,title)"

base="$(jq -r .baseRefName <<<"$info")"
head_sha="$(jq -r .headRefOid <<<"$info")"
title="$(jq -r .title <<<"$info")"

[[ "$base" =~ ^m[0-9]+$ ]] || die "#$pr targets '$base'; only milestone branches m<n> are allowed"
[ "$base" != "$default_branch" ] || die "#$pr targets the default branch"
[ "$(jq -r .state <<<"$info")" = "OPEN" ] || die "#$pr is not open"
[ "$(jq -r .isDraft <<<"$info")" = "false" ] || die "#$pr is a draft"

check_passed() {
    local name="$1" run
    run="$(gh api "repos/$repo/commits/$head_sha/check-runs?check_name=$name&filter=latest" \
        --jq '.check_runs | sort_by(.id) | last // empty | "\(.status) \(.conclusion)"')"
    [ "$run" = "completed success" ] || die "check '$name' on ${head_sha:0:7} is '${run:-missing}', not 'completed success'"
}
check_passed verify
check_passed tripwire

behind="$(gh api "repos/$repo/compare/$base...$head_sha" --jq .behind_by)"
[ "$behind" = "0" ] || die "#$pr is $behind commit(s) behind '$base'; merge '$base' into the branch and let CI run again"

echo "Merging #$pr ($title) into $base at ${head_sha:0:7}"
gh pr merge "$pr" --repo "$repo" --squash --match-head-commit "$head_sha" --subject "$title (#$pr)"
merged="$(gh pr view "$pr" --repo "$repo" --json mergeCommit --jq .mergeCommit.oid)"
echo "Merged #$pr into $base as $merged"
