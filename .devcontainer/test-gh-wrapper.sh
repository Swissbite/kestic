#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause
#
# Tests the merge rule of gh-wrapper.sh against a fake gh: merges into main are
# refused before gh runs, merges into other branches and other commands go
# through unchanged.

set -euo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# Fake gh: `pr view [<pr>] [--repo <r>] --json baseRefName` answers from
# FAKE_BASES ("<repo>#<pr>=<base> …"; repo "-" = current repository, pr
# "current" = no argument); every other call is logged.
cat > "$tmp/gh" <<'FAKE'
#!/usr/bin/env bash
if [[ "$1 $2" == "pr view" && " $* " == *" --json baseRefName "* ]]; then
    shift 2
    pr=current repo=-
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --repo) repo="$2"; shift 2 ;;
            --json | --jq) shift 2 ;;
            *) pr="${1##*/}"; shift ;;
        esac
    done
    for entry in $FAKE_BASES; do
        if [[ "${entry%%=*}" == "$repo#$pr" ]]; then echo "${entry#*=}"; exit 0; fi
    done
    echo "no pull requests found" >&2
    exit 1
fi
printf '%s\n' "$*" >> "$FAKE_LOG"
FAKE
chmod +x "$tmp/gh"

export KESTIC_REAL_GH="$tmp/gh" FAKE_LOG="$tmp/log"
export FAKE_BASES="-#12=main -#34=m4 -#current=m4 Swissbite/kestic#12=main Swissbite/kestic#34=m4 o/r#12=m4 o/r#34=main"
failures=0

expect() {
    local want="$1"
    shift
    : > "$FAKE_LOG"
    local got=passed
    "$here/gh-wrapper.sh" "$@" 2>/dev/null || got=refused
    if [[ "$got" == passed && ! -s "$FAKE_LOG" ]]; then got="passed without calling gh"; fi
    if [[ "$got" == refused && -s "$FAKE_LOG" ]]; then got="refused after calling gh"; fi
    if [[ "$got" != "$want" ]]; then
        echo "FAIL: gh $* -> $got, expected $want" >&2
        failures=$((failures + 1))
    fi
}

printf '%s\n' '{"query": "mutation { mergePullRequest(input: {pullRequestId: \"x\"}) { clientMutationId } }"}' > "$tmp/merge.json"
printf '%s\n' 'query { viewer { login } }' > "$tmp/read.graphql"

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
expect refused pr --repo o/r merge 34
expect refused pr merge -Ro/r 34
expect refused pr merge --repo=o/r 34
FAKE_BASES="-#current=main" expect refused pr merge
expect passed pr view 12
expect passed pr create --title merge --base main

# gh api: PUT on the merge endpoint of a main pull request refused, in every
# spelling of the path; GET and other bases pass
expect refused api -X PUT repos/Swissbite/kestic/pulls/12/merge
expect refused api -XPUT repos/Swissbite/kestic/pulls/12/merge
expect refused api /repos/Swissbite/kestic/pulls/12/merge --method PUT
expect refused api --method=put '/repos/{owner}/{repo}/pulls/12/merge'
expect refused api -X PUT https://api.github.com/repos/o/r/pulls/34/merge
expect refused api -H 'Accept: application/json' repos/o/r/pulls/34/merge -X PUT
expect passed api -X PUT repos/Swissbite/kestic/pulls/34/merge
expect passed api -X PUT repos/o/r/pulls/12/merge
expect passed api repos/Swissbite/kestic/pulls/12/merge
expect passed api repos/Swissbite/kestic/pulls/12

# branch merge endpoint: base main or unknown refused
expect refused api repos/Swissbite/kestic/merges -f base=main -f head=topic
expect refused api repos/Swissbite/kestic/merges -fbase=main -f head=topic
expect refused api repos/Swissbite/kestic/merges -f head=topic
expect passed api repos/Swissbite/kestic/merges -f base=m4 -f head=topic

# GraphQL merge mutations refused, also from a file; other GraphQL and text
# that merely names a mutation pass
expect refused api graphql -f 'query=mutation { mergePullRequest(input: {pullRequestId: "x"}) { clientMutationId } }'
expect refused api graphql -F 'query=mutation { enablePullRequestAutoMerge(input: {pullRequestId: "x"}) { clientMutationId } }'
expect refused api graphql --input "$tmp/merge.json"
expect passed api graphql -F "query=@$tmp/read.graphql"
expect passed api repos/Swissbite/kestic/issues/1/comments -f 'body=see mergePullRequest'
expect passed issue comment 42 --body 'merge into main later'

if [[ "$failures" -gt 0 ]]; then
    exit 1
fi
echo "gh-wrapper: merge rule ok"
