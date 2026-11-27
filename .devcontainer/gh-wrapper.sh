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
# - Refuses to read GitHub text (titles, bodies, comments, reviews, commit
#   messages of pull requests, run titles): GitHub text reaches the agent only
#   through `kestic-agent thread`, which frames and filters it.
#   `issue|pr view|list` and `run list` pass only with `--json` and fields
#   from an allowlist; other `issue`, `pr` and `run` subcommands only when they
#   print no such text; `gh api` reads only from an allowlist of endpoints;
#   `gh search` and `gh alias set|import` are refused.
# - Refuses to add the label `adopted`: only the maintainer adopts an issue.
#
# Other commands go through unchanged. This guards against habits and
# mistakes only: the token is readable in the container, so curl bypasses it.

set -euo pipefail

REAL_GH="${KESTIC_REAL_GH:-/usr/bin/gh}"
TOKEN_FILE=/run/kestic-agent-token/gh-token
TRUST_FILE="${KESTIC_TRUST_JQ:-/usr/local/lib/kestic-agent/github-trust.jq}"
# --json fields without text written by people, for issues and pull requests
# and for workflow runs.
JSON_FIELDS=" number state labels baseRefName headRefName headRefOid isCrossRepository isDraft mergeable mergeCommit mergedAt statusCheckRollup url "
RUN_FIELDS=" databaseId status conclusion headSha workflowName event createdAt updatedAt url "

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

# pr_target <subcommand> <arguments after `pr`…>: the pull request a merge,
# checkout or diff acts on (empty = current branch), parsed with that
# subcommand's own flags. Refuses more than one.
pr_target() {
    local sub="$1" values target="" found=0 seen_sub=0
    shift
    case "$sub" in
        merge) values=" -A --author-email -b --body -F --body-file --match-head-commit -t --subject -R --repo " ;;
        checkout | co) values=" -b --branch -R --repo " ;;
        diff) values=" -R --repo --color " ;;
    esac
    while [[ $# -gt 0 ]]; do
        if [[ "$1" == -* ]]; then
            [[ "$1" != *=* && "$values" == *" $1 "* ]] && shift
        elif [[ "$seen_sub" == 0 ]]; then
            seen_sub=1
        elif [[ "$found" == 0 ]]; then
            target="$1" found=1
        else
            refuse "pr $sub: more than one pull request named"
        fi
        shift || true
    done
    printf '%s' "$target"
}

# check_json <fields> <what> <allowlist>: refuses a --json value with a field
# outside the allowlist, and a view or list without --json.
check_json() {
    local fields="$1" field
    [[ -n "$fields" ]] || refuse_read "$2 without --json"
    IFS=, read -ra list <<< "$fields"
    for field in "${list[@]}"; do
        [[ "$3" == *" $field "* ]] || refuse_read "--json field '$field'"
    done
}

# Refuses `adopted` in a comma-separated label list.
check_labels() {
    local label
    IFS=, read -ra list <<< "$1"
    for label in "${list[@]}"; do
        label="${label// /}"
        [[ "${label,,}" != adopted ]] || refuse "only the maintainer adds the label 'adopted'"
    done
}

original_args=("$@")

case "${1:-}" in
    search)
        refuse_read "gh search"
        ;;
    alias)
        [[ "${2:-}" != set && "${2:-}" != import ]] || refuse "gh alias set|import is refused in the agent container"
        ;;
    issue | pr | run)
        group="$1"
        words=() repo="" json="" log=0
        set -- "${@:2}"
        while [[ $# -gt 0 ]]; do
            case "$1" in
                -R | --repo) repo="${2:-}"; shift ;;
                -R?* | --repo=*) repo="${1#-R}"; repo="${repo#--repo=}"; repo="${repo#=}" ;;
                # gh joins repeated --json flags.
                --json) json+="${json:+,}${2:-}"; shift ;;
                --json=*) json+="${json:+,}${1#--json=}" ;;
                --log | --log-failed) log=1 ;;
                -l | --label | --add-label) check_labels "${2:-}"; shift ;;
                --label=* | --add-label=*) check_labels "${1#*=}" ;;
                -l?*) check_labels "${1#-l}" ;;
                # Flags that take a value in every issue, pr and run subcommand.
                -b | --body | -F | --body-file | -t | --title | --subject | --author-email | \
                    --match-head-commit | -q | --jq | -T | --template | --search | --state | -L | --limit | \
                    --assignee | --add-assignee | --remove-assignee | --remove-label | --milestone | \
                    --project | --add-project | --remove-project | --base | --head | --reviewer | \
                    --add-reviewer | --remove-reviewer | --app | --author | --mention | --branch | --comment | \
                    --job | --workflow | --user | --event | --commit | --created | --interval | --attempt | \
                    --color) shift ;;
                -*) ;;
                *) words+=("$1") ;;
            esac
            shift || true
        done
        case "$group ${words[0]:-}" in
            "issue view" | "issue list" | "issue ls" | "pr view" | "pr list" | "pr ls")
                check_json "$json" "$group ${words[0]}" "$JSON_FIELDS" ;;
            "run list" | "run ls")
                check_json "$json" "run ${words[0]}" "$RUN_FIELDS" ;;
            "run view")
                # A job's log (CI logs of the agent's own pull request) or allowlisted fields.
                [[ "$log" == 1 ]] || check_json "$json" "run view" "$RUN_FIELDS" ;;
            "pr merge" | "pr checkout" | "pr co" | "pr diff")
                target="$(pr_target "${words[0]}" "${original_args[@]:1}")" || exit 1
                check_pr "$target" "$repo" "${words[0]}" ;;
            "issue create" | "issue new" | "issue comment" | "issue edit" | "issue close" | "issue reopen" | \
                "pr create" | "pr new" | "pr comment" | "pr edit" | "pr close" | "pr reopen" | "pr ready" | \
                "pr review" | "pr checks" | "pr update-branch" | "run watch" | "run rerun" | "run cancel") ;;
            *) refuse_read "gh $group ${words[0]:-} is not on the list of commands without GitHub text" ;;
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
                    [[ "${2,,}" == *adopted* ]] && labels_arg=1
                    shift ;;
                --raw-field=* | --field=* | --input=*)
                    has_fields=1
                    [[ "${1,,}" == *adopted* ]] && labels_arg=1 ;;
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
        [[ "$path" =~ ^[Hh][Tt][Tt][Pp][Ss]?://[^/]+/([Aa][Pp][Ii]/[Vv]3/)?(.*)$ ]] && path="${BASH_REMATCH[2]}"
        path="${path#/}"
        path="${path%%\?*}"
        path="${path%/}"
        while [[ "$path" == *//* ]]; do path="${path//\/\//\/}"; done
        # Text checks ignore case, as GitHub does for owner and repository.
        lpath="${path,,}"
        # Dot segments and escapes would let an allowed prefix reach another endpoint.
        if [[ "/$lpath/" == */./* || "/$lpath/" == */../* || "$lpath" == *%* ]]; then
            refuse "gh api: path with dot segments or escapes refused: $path"
        fi
        if [[ "$method" == GET || "$method" == HEAD ]]; then
            # Reads only from endpoints that carry no text written by people.
            [[ "$lpath" =~ ^(rate_limit|repos/[^/]+/[^/]+(/(pulls/[0-9]+/merge|issues/[0-9]+/labels|labels(/[^/]+)?|branches(/[^/]+)*|actions/jobs/[0-9]+(/logs)?|actions/runs/[0-9]+/(jobs|artifacts)|commits/[^/]+/(check-runs|check-suites|status|statuses)|installation|dependency-graph/.*|git/(refs|ref|matching-refs)/.*))?)$ ]] ||
                refuse_read "GET $path"
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
