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
#   stays the only story. Something the maintainer (by user ID, github-trust.jq)
#   did after the agent's last turn started (a comment or review on the issue or
#   on a pull request the App opened from its branch, merging that pull request,
#   removing needs-maintainer) continues the idle session; a session that is
#   gone is resumed, twice at most until it reaches a turn again. An idle
#   session that stopped at a usage limit is continued once the limit has reset.
# - Otherwise the open issue of the milestone with the lowest number that is not
#   labelled needs-maintainer and has no open "blocked by" issue starts in a
#   fresh session. Issue numbers give the order, so new issues queue at the end.
# - An issue opened by someone other than the maintainer or the App is skipped
#   and reported once, unless the maintainer added the label `adopted` and
#   neither title nor body changed afterwards (github-thread.sh --issue-status).
# - With KESTIC_AGENT_LANE set to a label (for example lane:core), only issues
#   with that label are started. Two agents on one milestone each get their own
#   lane, so they never start the same issue.
#
# The maintainer hears about it in the log and, where notify-send exists, on the
# desktop: an issue waiting for them, a session that does not come back, an
# agent idle for hours without a result, a container that is not running, and
# a failed token renewal.
#
# Everything the agent reports (state line, issue) is treated as untrusted.

set -euo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO="${KESTIC_AGENT_REPO:-Swissbite/kestic}"
AGENT="${KESTIC_AGENT_SH:-$here/kestic-agent.sh}"
THREAD="$here/github-thread.sh"
HOLD=needs-maintainer
# An idle agent without a result for this long is reported once.
IDLE_ALERT=14400

[[ $# -eq 2 ]] || { echo "usage: $0 <milestone title> <base branch>" >&2; exit 2; }
LANE="${KESTIC_AGENT_LANE:-}"
# The lane goes into a jq filter, so it is limited to plain label characters.
[[ "$LANE" =~ ^[A-Za-z0-9:._-]*$ ]] || { echo "KESTIC_AGENT_LANE: only letters, digits and :._- allowed" >&2; exit 2; }
milestone="$1"
base="$2"
STATE_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/kestic-agent"
# One watcher per agent instance (KESTIC_AGENT_NAME, see kestic-agent.sh).
instance="${KESTIC_AGENT_NAME:-kestic-agent}"
suffix=""
[[ "$instance" == kestic-agent ]] || suffix="-$instance"
NOTE_FILE="$STATE_DIR/watch-notes$suffix"
RESUME_FILE="$STATE_DIR/watch-resume$suffix"
LIMIT_FILE="$STATE_DIR/watch-limit$suffix"
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

# Whose activity counts and whose issues start: github-trust.jq (by numeric
# user ID); prepended to the --jq filters below.
TRUST="$(<"${KESTIC_TRUST_JQ:-$here/github-trust.jq}")"
# `<epoch> <trust class>` per comment or submitted review.
COMMENT_JQ="$TRUST"' .[] | select((.created_at // .submitted_at) != null)
    | "\((.created_at // .submitted_at) | fromdateiso8601) \(.user | trust_class)"'
# The same for removals of the label.
UNLABEL_JQ="$TRUST .[] | select(.event == \"unlabeled\" and .label.name == \"$HOLD\")
    | \"\(.created_at | fromdateiso8601) \(.actor | trust_class)\""

# Newest thing the maintainer did on the issue or its pull requests (epoch
# seconds); empty if nothing. Only the maintainer's user ID counts: comments
# by outsiders, other collaborators, bots and the agent itself never wake the
# agent. Pull requests count only when the App opened them in this repository:
# a fork's branch may carry the same name.
latest_event() {
    local issue="$1" events prs pr merged by
    events="$(gh api "repos/$REPO/issues/$issue/comments" --paginate --jq "$COMMENT_JQ")" || return 1
    events+=$'\n'"$(gh api "repos/$REPO/issues/$issue/events" --paginate --jq "$UNLABEL_JQ")" || return 1
    prs="$(gh api --paginate "repos/$REPO/pulls?state=all&head=${REPO%%/*}:agent/issue-$issue&per_page=100" \
        --jq "$TRUST .[] | select(.head.ref == \"agent/issue-$issue\" and agent_pull)
            | \"\(.number) \(if .merged_at then (.merged_at | fromdateiso8601) else 0 end)\"")" || return 1
    while read -r pr merged; do
        [[ "$pr" =~ ^[0-9]+$ ]] || continue
        if [[ "$merged" =~ ^[1-9][0-9]*$ ]]; then
            by="$(gh api "repos/$REPO/pulls/$pr" --jq "$TRUST .merged_by | trust_class")" || return 1
            events+=$'\n'"$merged $by"
        fi
        events+=$'\n'"$(gh api "repos/$REPO/issues/$pr/comments" --paginate --jq "$COMMENT_JQ")" || return 1
        events+=$'\n'"$(gh api "repos/$REPO/pulls/$pr/comments" --paginate --jq "$COMMENT_JQ")" || return 1
        events+=$'\n'"$(gh api "repos/$REPO/pulls/$pr/reviews" --paginate --jq "$COMMENT_JQ")" || return 1
    done <<< "$prs"
    awk '$1 ~ /^[0-9]+$/ && $2 == "maintainer" { print $1 }' <<< "$events" | sort -n | tail -n 1
}

read -r state issue turn token reset _ < <("$AGENT" agent-state) || true
if [[ "${token:-}" == failed ]]; then
    notify "token-$(date +%Y%m%d%H)" "renewing the agent's GitHub token failed (see kestic-agent.sh token)"
fi
case "$state" in
    off)
        # The container restarts by itself after a failure (--restart=on-failure);
        # off means it was stopped on purpose, the host rebooted, or it failed again.
        notify "off-$(date +%Y%m%d%H)" "the container is not running; continue with: kestic-agent.sh resume <issue>"
        exit 0
        ;;
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
[[ "${reset:-}" =~ ^[0-9]{1,12}$ ]] || reset=0
reset=$((10#$reset))
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
            # A session stopped at a usage limit waits until the limit has
            # reset, also for the maintainer's news (a nudge would only meet the
            # limit again); then it is continued once per turn.
            if ((reset > 0 && now < reset + 60)); then
                log "#$issue waits for the usage limit to reset at $(date -d "@$reset" +%H:%M)"
            elif [[ -n "$latest" ]] && ((latest > turn)); then
                log "the maintainer acted on #$issue, continuing the session"
                "$AGENT" nudge "There is something new from the maintainer on issue #$issue or its pull request (a comment, a review, a merge or a removed label). Read it and continue."
            elif ((reset > 0)) && [[ "$(cat "$LIMIT_FILE" 2>/dev/null || true)" != "$turn" ]]; then
                echo "$turn" > "$LIMIT_FILE"
                log "the usage limit has reset, continuing #$issue"
                "$AGENT" nudge "The usage limit has reset. Continue where you stopped; read your state comment on the issue first."
            elif ((turn > 0 && now - turn > IDLE_ALERT)); then
                notify "idle-$issue-$turn" "the agent is idle on #$issue for $(((now - turn) / 3600)) h without a result"
            fi
        fi
        exit 0
    fi
fi

candidates="$(gh issue list --repo "$REPO" --milestone "$milestone" --state open --limit 500 \
    --json number,labels --jq ".[] | \"\(.number) \([.labels[].name] | index(\"$HOLD\") != null) \([.labels[].name] | index(\"$LANE\") != null)\"" |
    awk -v lane="$LANE" '$1 ~ /^[0-9]+$/ && $2 == "false" && (lane == "" || $3 == "true") { print $1 }' | sort -n)"
for next in $candidates; do
    if ! open_blockers="$(gh api "repos/$REPO/issues/$next/dependencies/blocked_by" --paginate \
        --jq '.[] | select(.state == "open") | .number')"; then
        log "cannot read the blockers of #$next, skipping it this time"
        continue
    fi
    if [[ -n "$open_blockers" ]]; then
        continue
    fi
    # Issues by someone else start only once the maintainer adopted them.
    if ! status="$(KESTIC_AGENT_REPO="$REPO" "$THREAD" --issue-status "$next")"; then
        log "cannot read who wrote #$next, skipping it this time"
        continue
    fi
    case "$status" in
        trusted | adopted) ;;
        edited)
            notify "edited-$next" "#$next was edited after it was adopted; check it and adopt it again (remove and add the label adopted)"
            continue
            ;;
        *)
            notify "outsider-$next" "#$next was opened by an outsider; restate the task, or adopt it with the label adopted"
            continue
            ;;
    esac
    log "starting #$next on $base"
    "$AGENT" start "$next" "$base"
    exit 0
done
