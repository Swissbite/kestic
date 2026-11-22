#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause
#
# One pass of the milestone watcher, run on the host, e.g. every five minutes by
# a systemd user timer (README.md):
#
#   kestic-agent-watch.sh "<milestone title>" <base branch>
#
# - The container is off or the agent is busy: nothing.
# - The agent's current issue is open and not labelled needs-maintainer: it
#   stays the only story. A comment newer than the agent's last turn that does
#   not carry the agent's mark (on the issue or on a pull request from its
#   branch) continues the idle session; a session that is gone is resumed.
# - Otherwise the open issue of the milestone with the lowest number that is not
#   labelled needs-maintainer and has no open "blocked by" issue starts in a
#   fresh session. Issue numbers give the order, so new issues queue at the end.
#
# The watcher never starts the container: switching it off stops the milestone.

set -euo pipefail

REPO="${KESTIC_AGENT_REPO:-Swissbite/kestic}"
AGENT="${KESTIC_AGENT_SH:-$(dirname -- "${BASH_SOURCE[0]}")/kestic-agent.sh}"
HOLD=needs-maintainer
MARK='<!-- kestic-agent -->'

[[ $# -eq 2 ]] || { echo "usage: $0 <milestone title> <base branch>" >&2; exit 2; }
milestone="$1"
base="$2"

log() {
    echo "kestic-agent-watch: $*"
}

# `<number> <held>` per issue; held is true when it waits for the maintainer.
HELD_JQ="\"\(.number) \([.labels[].name] | index(\"$HOLD\") != null)\""

# `<epoch> <marked>` per comment or submitted review; marked is true for the
# agent's own. A pending review has no time yet and is skipped.
COMMENT_JQ=".[] | select((.created_at // .submitted_at) != null)
    | \"\((.created_at // .submitted_at) | fromdateiso8601) \((.body // \"\") | contains(\"$MARK\"))\""

# Newest unmarked comment (epoch seconds) on the issue and on pull requests from
# its branch; empty if there is none.
latest_comment() {
    local issue="$1" pr
    {
        gh api "repos/$REPO/issues/$issue/comments" --paginate --jq "$COMMENT_JQ"
        for pr in $(gh pr list --repo "$REPO" --head "agent/issue-$issue" --state all --json number --jq '.[].number'); do
            gh api "repos/$REPO/issues/$pr/comments" --paginate --jq "$COMMENT_JQ"
            gh api "repos/$REPO/pulls/$pr/comments" --paginate --jq "$COMMENT_JQ"
            gh api "repos/$REPO/pulls/$pr/reviews" --paginate --jq "$COMMENT_JQ"
        done
    } | awk '$2 == "false" { print $1 }' | sort -n | tail -n 1
}

read -r state issue turn < <("$AGENT" agent-state)
case "$state" in
    off | busy) exit 0 ;;
esac

if [[ "$issue" != 0 ]]; then
    read -r _ held < <(gh issue view "$issue" --repo "$REPO" --json number,labels,state \
        --jq "if .state == \"OPEN\" then $HELD_JQ else \"\(.number) closed\" end")
    if [[ "$held" == false ]]; then
        if [[ "$state" != idle ]]; then
            log "resuming #$issue ($state)"
            "$AGENT" resume "$issue"
        else
            latest="$(latest_comment "$issue")"
            if [[ -n "$latest" ]] && ((latest > turn)); then
                log "new comment on #$issue, continuing the session"
                "$AGENT" nudge "There is a new comment on issue #$issue or its pull request. Read it and continue."
            fi
        fi
        exit 0
    fi
fi

candidates="$(gh issue list --repo "$REPO" --milestone "$milestone" --state open --limit 500 \
    --json number,labels --jq ".[] | $HELD_JQ" | awk '$2 == "false" { print $1 }' | sort -n)"
for next in $candidates; do
    open_blockers="$(gh api "repos/$REPO/issues/$next/dependencies/blocked_by" --paginate \
        --jq '.[] | select(.state == "open") | .number')"
    if [[ -z "$open_blockers" ]]; then
        log "starting #$next on $base"
        "$AGENT" start "$next" "$base"
        exit 0
    fi
done
