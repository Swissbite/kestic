#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause
#
# `gh` in the agent container.
#
# - Reads the token from the Podman secret for this call only, so it is neither
#   in the general environment nor in a gh config file.
# - Refuses to merge a pull request into main: `gh pr merge` and a PUT on the
#   REST endpoint `repos/<owner>/<repo>/pulls/<n>/merge`. Merges into other
#   branches and all other commands go through unchanged. This guards against
#   mistakes only; GraphQL, aliases or the token with curl bypass it.

set -euo pipefail

REAL_GH="${KESTIC_REAL_GH:-/usr/bin/gh}"
TOKEN_FILE=/run/secrets/kestic-agent-gh-token

if [[ -r "$TOKEN_FILE" ]]; then
    GH_TOKEN="$(<"$TOKEN_FILE")"
    export GH_TOKEN
fi

# Refuses when pull request $1 (number, URL, branch; empty = current branch) in
# repository $2 (empty = current) targets main, or when that cannot be found out.
refuse_if_base_main() {
    local args=(pr view)
    [[ -n "$1" ]] && args+=("$1")
    [[ -n "$2" && "$2" != *[{:]* ]] && args+=(--repo "$2")
    local base
    base="$("$REAL_GH" "${args[@]}" --json baseRefName --jq .baseRefName)" || base="?"
    if [[ "$base" == main || "$base" == "?" ]]; then
        echo "gh: pull request ${1:-of the current branch} targets ${base/\?/an unknown branch};" \
            "merging into main is reserved for the maintainer" >&2
        exit 1
    fi
}

if [[ "${1:-}" == pr ]]; then
    words=() repo=""
    set -- "${@:2}"
    while [[ $# -gt 0 ]]; do
        case "$1" in
            -R | --repo) repo="${2:-}"; shift ;;
            -R?* | --repo=*) repo="${1#-R}"; repo="${repo#--repo=}"; repo="${repo#=}" ;;
            -b | --body | -F | --body-file | -t | --subject | -A | --author-email | --match-head-commit) shift ;;
            -*) ;;
            *) words+=("$1") ;;
        esac
        shift || true
    done
    if [[ "${words[0]:-}" == merge ]]; then
        refuse_if_base_main "${words[1]:-}" "$repo"
    fi
elif [[ "${1:-}" == api ]]; then
    merge_path='^(https?://[^/]+/(api/v3/)?)?/?repos/([^/]+/[^/]+)/pulls/([0-9]+)/merge/?$'
    put=0 pr="" repo=""
    for arg in "${@:2}"; do
        [[ "${arg^^}" =~ ^(PUT|-XPUT|--METHOD=PUT)$ ]] && put=1
        if [[ "$arg" =~ $merge_path ]]; then
            repo="${BASH_REMATCH[3]}" pr="${BASH_REMATCH[4]}"
        fi
    done
    if [[ -n "$pr" && "$put" == 1 ]]; then
        refuse_if_base_main "$pr" "$repo"
    fi
fi

exec "$REAL_GH" "$@"
