#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause
#
# Tests gh-wrapper.sh against a fake gh: merges into main and merges or
# checkouts of pull requests that are not the agent's own are refused, reads of
# GitHub text are refused unless --json names only allowlisted fields, the
# label `adopted` is refused, and everything else goes through unchanged.

set -euo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# Fake gh, printing what the real one prints after its --jq filter:
# - `pr view [<pr>] [--repo <r>] --json number,baseRefName,isCrossRepository`
#   answers "<number> <base> <cross>" from FAKE_PRS
#   ("<repo>#<pr>=<number>:<base>:<cross>:<class> …"; repo "-" = current
#   repository, pr "current" = no argument);
# - `api repos/<r>/pulls/<n> --jq …` answers the <class> (agent or other) of
#   that entry, "{owner}/{repo}" being the current repository;
# every other call is logged.
cat > "$tmp/gh" <<'FAKE'
#!/usr/bin/env bash
if [[ "$1 $2" == "pr view" && " $* " == *" --json number,baseRefName,isCrossRepository "* ]]; then
    shift 2
    pr=current repo=-
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --repo) repo="$2"; shift 2 ;;
            --json | --jq) shift 2 ;;
            *) pr="${1##*/}"; shift ;;
        esac
    done
    for entry in $FAKE_PRS; do
        if [[ "${entry%%=*}" == "$repo#$pr" ]]; then
            IFS=: read -r number base cross _ <<< "${entry#*=}"
            echo "$number $base $cross"
            exit 0
        fi
    done
    echo "no pull requests found" >&2
    exit 1
fi
if [[ "$1" == api && "$2" =~ ^repos/(.+)/pulls/([0-9]+)$ && " $* " == *" --jq "* && "$*" == *trust_class* ]]; then
    repo="${BASH_REMATCH[1]}" number="${BASH_REMATCH[2]}"
    [[ "$repo" == "{owner}/{repo}" ]] && repo=-
    for entry in $FAKE_PRS; do
        IFS=: read -r n _ _ class <<< "${entry#*=}"
        if [[ "${entry%%#*}" == "$repo" && "$n" == "$number" ]]; then echo "$class"; exit 0; fi
    done
    echo "Not Found" >&2
    exit 1
fi
printf '%s\n' "$*" >> "$FAKE_LOG"
FAKE
chmod +x "$tmp/gh"

export KESTIC_REAL_GH="$tmp/gh" FAKE_LOG="$tmp/log" KESTIC_TRUST_JQ="$here/github-trust.jq"
DEFAULT_PRS="-#12=12:main:false:agent -#34=34:m4:false:agent -#current=40:m4:false:agent
    Swissbite/kestic#12=12:main:false:agent Swissbite/kestic#34=34:m4:false:agent
    o/r#12=12:m4:false:agent o/r#34=34:main:false:agent
    -#50=50:m4:true:agent -#51=51:m4:false:other -#52=52:m4:true:other
    -#agent/issue-5=50:m4:true:agent"
export FAKE_PRS="$DEFAULT_PRS"
failures=0

expect() {
    local want="$1"
    shift
    : > "$FAKE_LOG"
    local got=passed
    "$here/gh-wrapper.sh" "$@" 2>/dev/null || got=refused
    if [[ "$got" == passed && ! -s "$FAKE_LOG" ]]; then got="passed without calling gh"; fi
    if [[ "$got" == passed && -s "$FAKE_LOG" && "$(<"$FAKE_LOG")" != "$*" ]]; then got="passed with changed arguments"; fi
    if [[ "$got" == refused && -s "$FAKE_LOG" ]]; then got="refused after calling gh"; fi
    if [[ "$got" != "$want" ]]; then
        echo "FAIL: gh $* -> $got, expected $want" >&2
        failures=$((failures + 1))
    fi
}

# gh pr merge: base main refused, other bases pass; the pull request is looked
# up in the repository given by -R in any spelling
expect refused pr merge 12
expect refused pr merge 12 --squash --delete-branch
expect refused pr merge --auto 12
expect refused pr merge https://github.com/Swissbite/kestic/pull/12
expect refused pr merge 99
expect passed pr merge 34 --squash
expect passed pr merge
expect passed pr merge --body 12 34
expect passed pr merge --subject 12 34
expect passed pr merge -R o/r 12
expect refused pr merge -R o/r 34
expect refused pr -R o/r merge 34
expect refused pr merge -Ro/r 34
expect refused pr merge --repo=o/r 34
FAKE_PRS="-#current=40:main:false:agent" expect refused pr merge

# gh api: PUT on the merge endpoint of a main pull request refused, relative
# or as a full URL; GET and other bases pass
expect refused api -X PUT repos/Swissbite/kestic/pulls/12/merge
expect refused api /repos/Swissbite/kestic/pulls/12/merge --method put
expect refused api --method=PUT '/repos/{owner}/{repo}/pulls/12/merge'
expect refused api -XPUT https://api.github.com/repos/o/r/pulls/34/merge
expect passed api -X PUT repos/Swissbite/kestic/pulls/34/merge
expect passed api -X PUT repos/o/r/pulls/12/merge
expect passed api repos/Swissbite/kestic/pulls/12/merge

# Fork pull requests and pull requests the App did not open: merge (gh pr
# merge, REST) and checkout refused, also by branch name agent/issue-<n>
expect refused pr merge 50 --squash
expect refused pr merge 51
expect refused pr merge 52
expect refused pr merge agent/issue-5
expect refused api -X PUT 'repos/{owner}/{repo}/pulls/50/merge'
expect refused api -X PUT 'repos/{owner}/{repo}/pulls/51/merge'
expect refused pr checkout 50
expect refused pr checkout 51
expect refused pr checkout agent/issue-5
expect refused pr checkout 99
expect passed pr checkout 34
KESTIC_TRUST_JQ=/nonexistent expect refused pr merge 34

# Reading GitHub text: views and lists only with allowlisted --json fields
expect refused issue view 5
expect refused issue view 5 --comments
expect refused pr view 7
expect refused pr view 7 --comments
expect refused issue list --milestone M6
expect refused pr list --base m6
expect refused issue view 5 --json body
expect refused issue view 5 --json number,comments
expect refused issue view 5 --json title
expect refused pr view 7 --json reviews
expect refused pr view 7 --json=number,body
expect refused pr view 7 --json author
expect refused pr status
expect refused issue status
expect passed issue view 5 --json number,state,labels
expect passed pr view 7 --json headRefOid,statusCheckRollup --jq .headRefOid
expect passed pr list --head agent/issue-5 --state all --json number,state,isCrossRepository,url
expect passed pr list --base m6 --label level-2 --state all --json number
expect passed issue list --milestone M6 --json=number,labels

# gh api: comments, reviews, issues, pull requests, timelines, GraphQL and
# search are refused for reading
expect refused api repos/Swissbite/kestic/issues/5/comments
expect refused api 'repos/{owner}/{repo}/issues/5/comments' --paginate
expect refused api /repos/Swissbite/kestic/issues/comments/123
expect refused api repos/Swissbite/kestic/pulls/7/comments
expect refused api repos/Swissbite/kestic/pulls/7/reviews
expect refused api repos/Swissbite/kestic/issues/5
expect refused api 'repos/Swissbite/kestic/issues?state=open'
expect refused api repos/Swissbite/kestic/issues/5/timeline
expect refused api repos/Swissbite/kestic/issues/5/dependencies/blocked_by
expect refused api repos/Swissbite/kestic/pulls/12
expect refused api repos/Swissbite/kestic/pulls
expect refused api https://api.github.com/repos/o/r/issues/5/comments
expect refused api repos/Swissbite/kestic//issues/5/comments
expect refused api REPOS/Swissbite/kestic/Issues/5/Comments
expect refused api -X GET repos/Swissbite/kestic/issues/5/comments -f per_page=100
expect refused api repos/Swissbite/kestic/commits/abc/comments
expect refused api graphql -f query='{ viewer { login } }'
expect refused api -X GET graphql
expect refused api 'search/issues?q=repo:Swissbite/kestic'
expect refused search issues kestic
expect refused search prs --repo Swissbite/kestic
expect refused api -X PATCH repos/Swissbite/kestic/issues/5 -f state=closed
expect passed api -X PATCH repos/Swissbite/kestic/issues/5 -f state=closed --silent
expect passed api repos/Swissbite/kestic --jq .full_name
expect passed api repos/Swissbite/kestic/actions/jobs/123/logs
expect passed api repos/Swissbite/kestic/issues/5/labels

# Aliases, repeated flags and other endpoints that carry the same text
expect refused issue ls --json title
expect refused pr ls --json number,body
expect passed pr ls --base m6 --json number
expect refused pr view 5 --json number --json body
expect refused pr view 5 --json body --json number
expect passed pr view 5 --json number --json state
expect refused pr co 50
expect refused pr co 51
expect passed pr co 34
expect refused pr diff 50
expect passed pr diff 34 --name-only
expect refused pr merge -s 50
expect refused pr merge -m -d 51
expect refused pr merge -r 50 --delete-branch
expect passed pr merge -s -d 34
expect refused pr merge 34 50
expect refused pr checkout --detach 50
expect refused pr lock 5
expect refused issue develop 5 --list
expect refused issue transfer 5 o/other
expect refused run list
expect refused run list --json displayTitle
expect passed run list --json databaseId,conclusion,headSha,workflowName,event
expect refused run view 123
expect passed run view 123 --log-failed
expect passed run view 123 --job 456 --log
expect passed run view 123 --json conclusion,headSha
expect refused alias set iv 'issue view'
expect refused alias import aliases.yml
expect passed alias list
expect refused api repositories/1234/issues/5
expect refused api issues
expect refused api user/issues
expect refused api orgs/Swissbite/issues
expect refused api users/mallory
expect refused api repos/Swissbite/kestic/commits
expect refused api repos/Swissbite/kestic/pulls/12/commits
expect refused api repos/Swissbite/kestic/pulls/12/files
expect refused api repos/Swissbite/kestic/issues/5/events
expect refused api repos/Swissbite/kestic/actions/runs
expect refused api repos/Swissbite/kestic/releases
expect refused api HTTPS://API.GITHUB.COM/repos/o/r/issues/5/comments
expect passed api repos/Swissbite/kestic/actions/jobs/123
expect passed api 'repos/{owner}/{repo}/commits/abc123/check-runs'
expect passed api rate_limit

# The label adopted is the maintainer's
expect refused issue edit 5 --add-label adopted
expect refused issue edit 5 --add-label needs-maintainer,adopted
expect refused issue edit 5 --add-label=adopted
expect refused issue edit 5 --add-label 'level-2, adopted'
expect refused issue edit 5 --add-label Adopted
expect refused issue edit 5 --add-label ADOPTED
expect refused issue create --title x --body y --label adopted
expect refused pr edit 7 --add-label adopted
expect refused api repos/Swissbite/kestic/issues/5/labels -f 'labels[]=adopted'
expect refused api -X PATCH repos/Swissbite/kestic/issues/5 --silent -f 'labels[]=adopted'
expect passed issue edit 5 --add-label needs-maintainer
expect passed issue edit 5 --remove-label adopted
expect passed issue edit 5 --body 'not adopted yet' --add-label needs-maintainer

# Texts that mention merge, main or the forbidden words never block anything
expect passed pr create --title merge --base main --body 'merge into main'
expect passed pr comment 12 --body 'merge main'
expect passed pr edit 12 --title merge
expect passed pr review 12 --approve -b merge
expect passed issue comment 42 --body 'gh pr merge 12 into main'
expect passed issue comment 42 --body 'gh issue view 5 --json body'
expect passed api -X POST repos/Swissbite/kestic/issues/12/comments -f 'body=PUT repos/o/r/pulls/12/merge'
expect passed api -X POST repos/Swissbite/kestic/issues/12/comments -f 'body=the label adopted'
expect passed pr checks 12
expect passed pr diff 12 --name-only

if [[ "$failures" -gt 0 ]]; then
    exit 1
fi
echo "gh-wrapper: ok"
