#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause
#
# `gh` in the agent container.
#
# - Reads the GitHub App token from its file for this call only, so it is
#   neither in the general environment nor in a gh config file.
# - Refuses to merge a pull request into main: `gh pr merge` and a PUT on the
#   REST endpoint `repos/<owner>/<repo>/pulls/<n>/merge`.
# - Refuses to merge or check out a pull request that is not the agent's own:
#   one from a fork, or one the App did not open (by numeric user ID,
#   github-trust.jq).
# - Refuses to read GitHub text (titles, bodies, comments, reviews): GitHub
#   text reaches the agent only through `kestic-agent thread`, which frames
#   and filters it. `issue view|list` and `pr view|list` pass only with
#   `--json` and fields from an allowlist; `gh api` reads of comments,
#   reviews, issues, pull requests and timelines, `gh api graphql`, `gh search`
#   and `issue|pr status` are refused.
# - Refuses to add the label `adopted`: only the maintainer adopts an issue.
#
# Everything else goes through unchanged. This guards against habits and
# mistakes only: the token is readable in the container, so curl bypasses it.

set -euo pipefail

REAL_GH="${KESTIC_REAL_GH:-/usr/bin/gh}"
TOKEN_FILE=/run/kestic-agent-token/gh-token
TRUST_FILE="${KESTIC_TRUST_JQ:-/usr/local/lib/kestic-agent/github-trust.jq}"
# --json fields without text written by people.
JSON_FIELDS=" number state labels baseRefName headRefName headRefOid isCrossRepository isDraft mergeable mergeCommit mergedAt statusCheckRollup url "

if [[ -r "$TOKEN_FILE" ]]; then
    GH_TOKEN="$(<"$TOKEN_FILE")"
    export GH_TOKEN
fi

refuse() {
    echo "gh: $*" >&2
    exit 1
}

refuse_read() {
    refuse "reading GitHub text this way is refused ($1); use \`kestic-agent thread <issue> [<pr>]\`," \
        "or --json with only these fields:$JSON_FIELDS"
}

# check_pr <pr> <repo> <action>: refuses unless pull request <pr> (number,
# URL, branch; empty = current branch) in repository <repo> (empty = current)
# is in that repository, opened by the App and, for a merge, not into main.
# Refuses when that cannot be found out.
check_pr() {
    local args=(pr view) info number base cross class
    [[ -n "$1" ]] && args+=("$1")
    [[ -n "$2" && "$2" != *[{:]* ]] && args+=(--repo "$2")
    info="$("$REAL_GH" "${args[@]}" --json number,baseRefName,isCrossRepository \
        --jq '"\(.number) \(.baseRefName) \(.isCrossRepository)"')" || info="? ? ?"
    read -r number base cross <<< "$info"
    if [[ "$3" == merge && ("$base" == main || "$base" == "?") ]]; then
        refuse "pull request ${1:-of the current branch} targets ${base/\?/an unknown branch};" \
            "merging into main is reserved for the maintainer"
    fi
    class="?"
    if [[ "$number" =~ ^[0-9]+$ && "$cross" == false && -r "$TRUST_FILE" ]]; then
        local repo_path="{owner}/{repo}"
        [[ -n "$2" && "$2" != *[{:]* ]] && repo_path="$2"
        class="$("$REAL_GH" api "repos/$repo_path/pulls/$number" \
            --jq "$(<"$TRUST_FILE") if agent_pull then \"agent\" else \"other\" end")" || class="?"
    fi
    if [[ "$class" != agent ]]; then
        refuse "pull request ${1:-of the current branch} is not the agent's own (a fork, another author," \
            "or unknown); $3 refused"
    fi
}

# Refuses a --json value with a field outside the allowlist, and a view or
# list without --json.
check_json() {
    local fields="$1" field
    [[ -n "$fields" ]] || refuse_read "$2 without --json"
    IFS=, read -ra list <<< "$fields"
    for field in "${list[@]}"; do
        [[ "$JSON_FIELDS" == *" $field "* ]] || refuse_read "--json field '$field'"
    done
}

# Refuses `adopted` in a comma-separated label list.
check_labels() {
    local label
    IFS=, read -ra list <<< "$1"
    for label in "${list[@]}"; do
        [[ "${label// /}" != adopted ]] || refuse "only the maintainer adds the label 'adopted'"
    done
}

original_args=("$@")

case "${1:-}" in
    search)
        refuse_read "gh search"
        ;;
    issue | pr)
        group="$1"
        words=() repo="" json="" has_json=0
        set -- "${@:2}"
        while [[ $# -gt 0 ]]; do
            case "$1" in
                -R | --repo) repo="${2:-}"; shift ;;
                -R?* | --repo=*) repo="${1#-R}"; repo="${repo#--repo=}"; repo="${repo#=}" ;;
                --json) has_json=1; json="${2:-}"; shift ;;
                --json=*) has_json=1; json="${1#--json=}" ;;
                -l | --label | --add-label) check_labels "${2:-}"; shift ;;
                --label=* | --add-label=*) check_labels "${1#*=}" ;;
                -l?*) check_labels "${1#-l}" ;;
                -b | --body | -F | --body-file | -t | --title | --subject | -A | --author-email | \
                    --match-head-commit | -q | --jq | -T | --template | -S | --search | -s | --state | \
                    -L | --limit | -a | --assignee | --add-assignee | --remove-assignee | --remove-label | \
                    -m | --milestone | -p | --project | --add-project | --remove-project | -B | --base | \
                    -H | --head | -r | --reviewer | --add-reviewer | --remove-reviewer | --app | --author | \
                    --mention | --type | --branch) shift ;;
                -*) ;;
                *) words+=("$1") ;;
            esac
            shift || true
        done
        case "${words[0]:-}" in
            view | list)
                [[ "$has_json" == 1 ]] || json=""
                check_json "$json" "$group ${words[0]}"
                ;;
            status) refuse_read "$group status" ;;
            merge) check_pr "${words[1]:-}" "$repo" merge ;;
            checkout) [[ "$group" == pr ]] && check_pr "${words[1]:-}" "$repo" checkout ;;
        esac
        ;;
    api)
        method="" endpoint="" has_fields=0 silent=0 labels_arg=0
        set -- "${@:2}"
        while [[ $# -gt 0 ]]; do
            case "$1" in
                -X | --method) method="${2:-}"; shift ;;
                -X?*) method="${1#-X}" ;;
                --method=*) method="${1#--method=}" ;;
                -f | --raw-field | -F | --field | --input)
                    has_fields=1
                    [[ "${2:-}" == *adopted* ]] && labels_arg=1
                    shift ;;
                --raw-field=* | --field=* | --input=*)
                    has_fields=1
                    [[ "$1" == *adopted* ]] && labels_arg=1 ;;
                -H | --header | -q | --jq | -t | --template | --cache | -p | --preview | --hostname) shift ;;
                --silent) silent=1 ;;
                -*) ;;
                *) [[ -z "$endpoint" ]] && endpoint="$1" ;;
            esac
            shift || true
        done
        method="${method^^}"
        [[ -n "$method" ]] || { if [[ "$has_fields" == 1 ]]; then method=POST; else method=GET; fi; }
        # The path without host, API prefix, leading slash and query.
        path="$endpoint"
        [[ "$path" =~ ^https?://[^/]+/(api/v3/)?(.*)$ ]] && path="${BASH_REMATCH[2]}"
        path="${path#/}"
        path="${path%%\?*}"
        path="${path%/}"
        while [[ "$path" == *//* ]]; do path="${path//\/\//\/}"; done
        # Text checks ignore case, as GitHub does for owner and repository.
        lpath="${path,,}"
        if [[ "$method" == GET || "$method" == HEAD ]]; then
            case "$lpath" in
                graphql) refuse_read "gh api graphql" ;;
                search/* | notifications* | gists* | */comments | */comments/* | repos/*/*/discussions*)
                    refuse_read "GET $path" ;;
                repos/*/*/issues* | repos/*/*/pulls*)
                    # Under issues and pulls only reads without people's text pass.
                    [[ "$lpath" =~ ^repos/[^/]+/[^/]+/(pulls/[0-9]+/(merge|files|commits)|issues/[0-9]+/(labels|events))$ ]] ||
                        refuse_read "GET $path" ;;
            esac
        elif [[ "$lpath" == graphql ]]; then
            refuse_read "gh api graphql"
        elif [[ "$lpath" =~ ^repos/[^/]+/[^/]+/(issues|pulls)/[0-9]+$ && "$silent" == 0 ]]; then
            # A change to an issue or a pull request answers with its full text.
            refuse_read "$method $path prints the text; add --silent"
        fi
        if [[ "$lpath" =~ /issues/[0-9]+(/labels)?$ && "$method" != GET && "$labels_arg" == 1 ]]; then
            refuse "only the maintainer adds the label 'adopted'"
        fi
        if [[ "$lpath" =~ ^repos/[^/]+/[^/]+/pulls/[0-9]+/merge$ && "$method" == PUT ]]; then
            [[ "$path" =~ ^[^/]+/([^/]+/[^/]+)/[^/]+/([0-9]+)/ ]]
            check_pr "${BASH_REMATCH[2]}" "${BASH_REMATCH[1]}" merge
        fi
        ;;
esac

exec "$REAL_GH" "${original_args[@]}"
