#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause
#
# `gh` in the agent container.
#
# - Reads the token from the Podman secret for this call only, so it is neither
#   in the general environment nor in a gh config file.
# - Refuses to merge into main: `gh pr merge` of a pull request whose base is
#   main, `gh api` on the pull request merge endpoint of such a pull request or
#   on the branch merge endpoint with base main, and GraphQL merge mutations.
#   Merges into other branches go through. This guards against mistakes only;
#   with the token and curl it can be bypassed.

set -euo pipefail

REAL_GH="${KESTIC_REAL_GH:-/usr/bin/gh}"
PROTECTED_BRANCH=main
TOKEN_FILE=/run/secrets/kestic-agent-gh-token

if [[ -r "$TOKEN_FILE" ]]; then
    GH_TOKEN="$(<"$TOKEN_FILE")"
    export GH_TOKEN
fi

refuse() {
    echo "gh: $1; merging into $PROTECTED_BRANCH is reserved for the maintainer" >&2
    exit 1
}

# Base branch of a pull request (number, URL or branch; empty = current branch).
pr_base() {
    local pr="$1" repo="$2"
    local args=(pr view)
    [[ -n "$pr" ]] && args+=("$pr")
    # `{owner}/{repo}` placeholders of gh api mean the current repository.
    [[ -n "$repo" && "$repo" != *"{"* ]] && args+=(--repo "$repo")
    "$REAL_GH" "${args[@]}" --json baseRefName --jq .baseRefName
}

check_pr_merge() {
    local pr="" repo=""
    while [[ $# -gt 0 ]]; do
        case "$1" in
            -R | --repo) repo="${2:-}"; shift 2 || shift ;;
            --repo=*) repo="${1#*=}"; shift ;;
            -b | --body | -F | --body-file | -t | --subject | -A | --author-email | --match-head-commit)
                shift 2 || shift ;;
            -*) shift ;;
            *) [[ -z "$pr" ]] && pr="$1"; shift ;;
        esac
    done
    local base
    base="$(pr_base "$pr" "$repo")" || refuse "cannot determine the base branch of the pull request"
    if [[ "$base" == "$PROTECTED_BRANCH" ]]; then
        refuse "pull request ${pr:-of the current branch} targets $base"
    fi
}

check_api() {
    local arg base="" path_pr="" path_repo="" branch_merge=0
    local prev=""
    for arg in "$@"; do
        if [[ "$arg" =~ ^/?repos/([^/]+/[^/]+)/pulls/([0-9]+)/merge/?(\?.*)?$ ]]; then
            path_repo="${BASH_REMATCH[1]}"
            path_pr="${BASH_REMATCH[2]}"
        elif [[ "$arg" =~ ^/?repos/[^/]+/[^/]+/merges/?(\?.*)?$ ]]; then
            branch_merge=1
        elif [[ "$arg" =~ (mergePullRequest|enablePullRequestAutoMerge|mergeBranch) ]]; then
            refuse "GraphQL merge mutations are not allowed here; use gh pr merge"
        fi
        case "$prev" in
            -f | -F | --field | --raw-field) [[ "$arg" == base=* ]] && base="${arg#base=}" ;;
        esac
        case "$arg" in
            -fbase=* | -Fbase=*) base="${arg#-?base=}" ;;
            --field=base=* | --raw-field=base=*) base="${arg#*=base=}" ;;
        esac
        prev="$arg"
    done
    if [[ -n "$path_pr" ]]; then
        local pr_base_branch
        pr_base_branch="$(pr_base "$path_pr" "$path_repo")" \
            || refuse "cannot determine the base branch of pull request $path_pr"
        if [[ "$pr_base_branch" == "$PROTECTED_BRANCH" ]]; then
            refuse "pull request $path_pr targets $pr_base_branch"
        fi
    fi
    if [[ "$branch_merge" == 1 && ( -z "$base" || "$base" == "$PROTECTED_BRANCH" ) ]]; then
        refuse "branch merge into ${base:-an unknown base}"
    fi
}

case "${1:-} ${2:-}" in
    "pr merge") check_pr_merge "${@:3}" ;;
esac
if [[ "${1:-}" == api ]]; then
    check_api "${@:2}"
fi

exec "$REAL_GH" "$@"
