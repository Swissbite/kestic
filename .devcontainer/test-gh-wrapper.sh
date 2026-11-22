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

# Fake gh: `pr view <pr> … --json baseRefName` answers from FAKE_BASES
# ("<pr>=<base> …", "current" for no argument); every other call is logged.
cat > "$tmp/gh" <<'FAKE'
#!/usr/bin/env bash
if [[ "$1 $2" == "pr view" && " $* " == *" --json baseRefName "* ]]; then
    pr="current"
    [[ "$3" != -* && -n "$3" ]] && pr="${3##*/}"
    for entry in $FAKE_BASES; do
        if [[ "${entry%%=*}" == "$pr" ]]; then echo "${entry#*=}"; exit 0; fi
    done
    echo "no pull requests found" >&2
    exit 1
fi
printf '%s\n' "$*" >> "$FAKE_LOG"
FAKE
chmod +x "$tmp/gh"

export KESTIC_REAL_GH="$tmp/gh" FAKE_LOG="$tmp/log" FAKE_BASES="12=main 34=m4 current=m4"
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

expect refused pr merge 12
expect refused pr merge 12 --squash --delete-branch
expect refused pr merge https://github.com/Swissbite/kestic/pull/12
expect refused pr merge -R Swissbite/kestic --subject x 12
expect refused pr merge 99
expect passed pr merge 34 --squash
expect passed pr merge
expect refused api -X PUT repos/Swissbite/kestic/pulls/12/merge
expect refused api /repos/Swissbite/kestic/pulls/12/merge --method PUT
expect refused api --method=PUT '/repos/{owner}/{repo}/pulls/12/merge'
expect passed api -X PUT repos/Swissbite/kestic/pulls/34/merge
expect refused api repos/Swissbite/kestic/merges -f base=main -f head=topic
expect refused api repos/Swissbite/kestic/merges -f head=topic
expect passed api repos/Swissbite/kestic/merges -f base=m4 -f head=topic
expect refused api graphql -f 'query=mutation { mergePullRequest(input: {pullRequestId: "x"}) { clientMutationId } }'
expect passed api repos/Swissbite/kestic/pulls/12
expect passed pr view 12
expect passed issue comment 42 --body 'merge into main later'

FAKE_BASES="12=main current=main" expect refused pr merge

if [[ "$failures" -gt 0 ]]; then
    exit 1
fi
echo "gh-wrapper: merge rule ok"
