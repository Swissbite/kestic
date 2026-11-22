#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause
#
# `gh` in the agent container.
#
# - Reads the token from the Podman secret for this call only, so it is neither
#   in the general environment nor in a gh config file.
# - Refuses to merge into main: `gh pr merge` of a pull request whose base is
#   main, `gh api` PUT on the merge endpoint of such a pull request, the branch
#   merge endpoint with base main (or an unknown base), and GraphQL merge
#   mutations whatever their base. Merges into other branches through
#   `gh pr merge` or the REST endpoints go through. This guards against
#   mistakes only; aliases, the token with curl and similar detours bypass it.

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
    # `{owner}/{repo}` and `:owner/:repo` placeholders mean the current repository.
    [[ -n "$repo" && "$repo" != *"{"* && "$repo" != *:* ]] && args+=(--repo "$repo")
    "$REAL_GH" "${args[@]}" --json baseRefName --jq .baseRefName
}

# `gh pr [-R repo] merge [<pr>] [flags]`: refuses when the base is main.
check_pr_merge() {
    local pr="" repo="" sub=""
    while [[ $# -gt 0 ]]; do
        case "$1" in
            -R | --repo) repo="${2:-}"; shift 2 || shift ;;
            --repo=* | -R=*) repo="${1#*=}"; shift ;;
            -R?*) repo="${1#-R}"; shift ;;
            -b | --body | -F | --body-file | -t | --subject | -A | --author-email | --match-head-commit)
                shift 2 || shift ;;
            --) shift ;;
            -*) shift ;;
            *)
                if [[ -z "$sub" ]]; then
                    sub="$1"
                    [[ "$sub" == merge ]] || return 0
                elif [[ -z "$pr" ]]; then
                    pr="$1"
                fi
                shift
                ;;
        esac
    done
    [[ "$sub" == merge ]] || return 0
    local base
    base="$(pr_base "$pr" "$repo")" || refuse "cannot determine the base branch of the pull request"
    if [[ "$base" == "$PROTECTED_BRANCH" ]]; then
        refuse "pull request ${pr:-of the current branch} targets $base"
    fi
}

check_api() {
    local prefix='^(https?://[^/]+/(api/v3/)?)?/?repos/([^/]+/[^/]+)/'
    local arg prev="" endpoint="" method="" base="" queries=()
    for arg in "$@"; do
        case "$prev" in
            -X | --method) method="$arg" ;;
            -f | -F | --field | --raw-field)
                [[ "$arg" == base=* ]] && base="${arg#base=}"
                [[ "$arg" == query=* ]] && queries+=("${arg#query=}")
                ;;
            --input) queries+=("@$arg") ;;
            -H | --header | -q | --jq | -t | --template | --cache | -p | --preview | --hostname) ;;
            *)
                case "$arg" in
                    --method=*) method="${arg#*=}" ;;
                    -X?*) method="${arg#-X}" ;;
                    -fbase=* | -Fbase=*) base="${arg#-?base=}" ;;
                    --field=base=* | --raw-field=base=*) base="${arg#*=base=}" ;;
                    -fquery=* | -Fquery=*) queries+=("${arg#-?query=}") ;;
                    --field=query=* | --raw-field=query=*) queries+=("${arg#*=query=}") ;;
                    -*) ;;
                    *) [[ -z "$endpoint" ]] && endpoint="$arg" ;;
                esac
                ;;
        esac
        prev="$arg"
    done

    if [[ "$endpoint" == graphql ]]; then
        local query text
        for query in "${queries[@]+"${queries[@]}"}"; do
            text="$query"
            if [[ "$query" == @* && "$query" != @- && -r "${query#@}" ]]; then
                text="$(<"${query#@}")"
            fi
            if [[ "$text" =~ (mergePullRequest|enablePullRequestAutoMerge|mergeBranch) ]]; then
                refuse "GraphQL merge mutations are not allowed here; use gh pr merge"
            fi
        done
        return 0
    fi

    if [[ "$endpoint" =~ ${prefix}pulls/([0-9]+)/merge/?(\?.*)?$ ]]; then
        # Without -X it is the GET "is it merged" call.
        [[ "${method^^}" == PUT ]] || return 0
        local repo="${BASH_REMATCH[3]}" pr="${BASH_REMATCH[4]}" pr_base_branch
        pr_base_branch="$(pr_base "$pr" "$repo")" \
            || refuse "cannot determine the base branch of pull request $pr"
        if [[ "$pr_base_branch" == "$PROTECTED_BRANCH" ]]; then
            refuse "pull request $pr targets $pr_base_branch"
        fi
    elif [[ "$endpoint" =~ ${prefix}merges/?(\?.*)?$ ]]; then
        if [[ -z "$base" || "$base" == "$PROTECTED_BRANCH" ]]; then
            refuse "branch merge into ${base:-an unknown base}"
        fi
    fi
}

case "${1:-}" in
    pr) check_pr_merge "${@:2}" ;;
    api) check_api "${@:2}" ;;
esac

exec "$REAL_GH" "$@"
