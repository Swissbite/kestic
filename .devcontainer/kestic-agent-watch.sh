#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause
#
# One pass of the milestone watcher, run on the host, e.g. every five minutes by
# a systemd user timer (README.md):
#
#   kestic-agent-watch.sh "<milestone title>" <base branch>
#
# It watches the agent instance KESTIC_AGENT_NAME (default kestic-agent); run
# one watcher per instance and milestone.
#
# - The container is off or the agent is busy: nothing. The watcher never
#   starts the container: switching it off pauses the milestone.
# - The agent's current issue is open and not labelled needs-maintainer: it
#   stays the only story. Something the maintainer did after the agent's last
#   turn started (a comment or review on the issue or on a pull request from its
#   branch, merging that pull request, removing needs-maintainer) continues the
#   idle session; a session that is gone is resumed, twice at most until it
#   reaches a turn again.
# - Otherwise the open issue of the milestone with the lowest number that is not
#   labelled needs-maintainer and has no open "blocked by" issue starts in a
#   fresh session. Issue numbers give the order, so new issues queue at the end.
#
# The maintainer hears about it in the log and, where notify-send exists, on the
# desktop: an issue waiting for them, a session that does not come back, an
# agent idle for hours without a result, and a failed token renewal.
#
# Everything the agent reports (state line, issue) is treated as untrusted.

set -euo pipefail

REPO="${KESTIC_AGENT_REPO:-Swissbite/kestic}"
AGENT="${KESTIC_AGENT_SH:-$(dirname -- "${BASH_SOURCE[0]}")/kestic-agent.sh}"
HOLD=needs-maintainer
# An idle agent without a result for this long is reported once.
IDLE_ALERT=14400

[[ $# -eq 2 ]] || { echo "usage: $0 <milestone title> <base branch>" >&2; exit 2; }
milestone="$1"
base="$2"
STATE_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/kestic-agent"
# One watcher per agent instance (KESTIC_AGENT_NAME, see kestic-agent.sh).
instance="${KESTIC_AGENT_NAME:-kestic-agent}"
suffix=""
[[ "$instance" == kestic-agent ]] || suffix="-$instance"
NOTE_FILE="$STATE_DIR/watch-notes$suffix"
RESUME_FILE="$STATE_DIR/watch-resume$suffix"
mkdir -p "$STATE_DIR"
touch "$NOTE_FILE"

log() {
    echo "kestic-agent-watch: $*"
}

# notify <key> <text>: log every time, tell the desktop once per key.
notify() {
    log "$2"
    grep -qxF -- "$1" "$NOTE_FILE" && return 0
    echo "$1" >> "$NOTE_FILE"
    if command -v notify-send >/dev/null; then
        notify-send --app-name="Kestic agent" "$instance" "$2" || true
    fi
}

# `<epoch> <user type> <author association>` per comment or submitted review.
COMMENT_JQ='.[] | select((.created_at // .submitted_at) != null)
    | "\((.created_at // .submitted_at) | fromdateiso8601) \(.user.type) \(.author_association)"'
# The same for removals of the label, by whoever has the rights to do that.
UNLABEL_JQ=".[] | select(.event == \"unlabeled\" and .label.name == \"$HOLD\")
    | \"\(.created_at | fromdateiso8601) \(.actor.type) OWNER\""
# `<number> <merged epoch or 0> <User|Bot>` per pull request from the issue's branch.
PR_JQ='.[] | "\(.number) \(if .mergedAt then (.mergedAt | fromdateiso8601) else 0 end) \(if (.mergedBy.is_bot // false) then "Bot" else "User" end)"'

# Newest thing the maintainer did on the issue or its pull requests (epoch
# seconds); empty if nothing. Only people (no bots, no apps) with write access
# to the repository count; outsiders' comments never wake the agent.
latest_event() {
    local issue="$1" events prs pr merged by
    events="$(gh api "repos/$REPO/issues/$issue/comments" --paginate --jq "$COMMENT_JQ")" || return 1
    events+=$'\n'"$(gh api "repos/$REPO/issues/$issue/events" --paginate --jq "$UNLABEL_JQ")" || return 1
    prs="$(gh pr list --repo "$REPO" --head "agent/issue-$issue" --state all \
        --json number,mergedAt,mergedBy --jq "$PR_JQ")" || return 1
    while read -r pr merged by; do
        [[ "$pr" =~ ^[0-9]+$ ]] || continue
        if [[ "$merged" != 0 ]]; then events+=$'\n'"$merged $by OWNER"; fi
        events+=$'\n'"$(gh api "repos/$REPO/issues/$pr/comments" --paginate --jq "$COMMENT_JQ")" || return 1
        events+=$'\n'"$(gh api "repos/$REPO/pulls/$pr/comments" --paginate --jq "$COMMENT_JQ")" || return 1
        events+=$'\n'"$(gh api "repos/$REPO/pulls/$pr/reviews" --paginate --jq "$COMMENT_JQ")" || return 1
    done <<< "$prs"
    awk '$1 ~ /^[0-9]+$/ && $2 == "User" && $3 ~ /^(OWNER|MEMBER|COLLABORATOR)$/ { print $1 }' <<< "$events" |
        sort -n | tail -n 1
}

read -r state issue turn token _ < <("$AGENT" agent-state) || true
if [[ "${token:-}" == failed ]]; then
    notify "token-$(date +%Y%m%d%H)" "renewing the agent's GitHub token failed (see kestic-agent.sh token)"
fi
case "$state" in
    off) exit 0 ;;
    busy)
        rm -f "$RESUME_FILE"
        exit 0
        ;;
    none | stopped | idle) ;;
    *)
        notify "state-$state" "unexpected agent state '${state:0:20}'"
        exit 0
        ;;
esac
[[ "$issue" =~ ^[0-9]{1,9}$ ]] || issue=0
[[ "$turn" =~ ^[0-9]{1,12}$ ]] || turn=0
issue=$((10#$issue))
turn=$((10#$turn))

if ((issue > 0)); then
    if ! info="$(gh issue view "$issue" --repo "$REPO" --json state,labels \
        --jq "\"\(.state) \([.labels[].name] | index(\"$HOLD\") != null)\"")"; then
        log "cannot read issue #$issue, trying again next time"
        exit 0
    fi
    if [[ "$info" == "OPEN true" ]]; then
        notify "held-$issue-$turn" "#$issue waits for the maintainer (label $HOLD)"
    elif [[ "$info" == "OPEN false" ]]; then
        if [[ "$state" != idle ]]; then
            resumed_issue= resumes=
            if [[ -r "$RESUME_FILE" ]]; then read -r resumed_issue resumes < "$RESUME_FILE" || true; fi
            [[ "${resumed_issue:-}" == "$issue" && "${resumes:-}" =~ ^[0-9]+$ ]] || resumes=0
            if ((resumes >= 2)); then
                notify "gone-$issue-$turn" "the session on #$issue does not come back after $resumes resumes"
            else
                echo "$issue $((resumes + 1))" > "$RESUME_FILE"
                log "resuming #$issue ($state)"
                "$AGENT" resume "$issue"
            fi
        else
            rm -f "$RESUME_FILE"
            if ! latest="$(latest_event "$issue")"; then
                log "cannot read the activity on #$issue, trying again next time"
                exit 0
            fi
            now="$(date +%s)"
            if [[ -n "$latest" ]] && ((latest > turn)); then
                log "the maintainer acted on #$issue, continuing the session"
                "$AGENT" nudge "There is something new from the maintainer on issue #$issue or its pull request (a comment, a review, a merge or a removed label). Read it and continue."
            elif ((turn > 0 && now - turn > IDLE_ALERT)); then
                notify "idle-$issue-$turn" "the agent is idle on #$issue for $(((now - turn) / 3600)) h without a result"
            fi
        fi
        exit 0
    fi
fi

candidates="$(gh issue list --repo "$REPO" --milestone "$milestone" --state open --limit 500 \
    --json number,labels --jq ".[] | \"\(.number) \([.labels[].name] | index(\"$HOLD\") != null)\"" |
    awk '$1 ~ /^[0-9]+$/ && $2 == "false" { print $1 }' | sort -n)"
for next in $candidates; do
    if ! open_blockers="$(gh api "repos/$REPO/issues/$next/dependencies/blocked_by" --paginate \
        --jq '.[] | select(.state == "open") | .number')"; then
        log "cannot read the blockers of #$next, skipping it this time"
        continue
    fi
    if [[ -z "$open_blockers" ]]; then
        log "starting #$next on $base"
        "$AGENT" start "$next" "$base"
        exit 0
    fi
done
