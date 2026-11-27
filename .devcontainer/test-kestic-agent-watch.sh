#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause
#
# Tests the decisions of kestic-agent-watch.sh against a fake gh, a fake
# kestic-agent.sh and a fake notify-send: which issue starts, when a session is
# continued or resumed, whose activity counts, what is reported to the
# maintainer, and that the agent's state line cannot run code on the host. The
# fake gh prints what the real one prints after its --jq filter; the filters are
# checked against the real GitHub API by hand (README.md).

set -euo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
fix="$tmp/fixtures"

# Fake gh: prints the fixture file named after the call, or nothing; fails if a
# file <name>.fail exists.
#   issue view <n>                  -> issue-<n>   ("OPEN false|true" or "CLOSED false")
#   pr list --head agent/issue-<n>  -> prs-<n>     ("<pr> <merged epoch|0> <User|Bot>")
#   issue list --milestone <m>      -> open-<m>    ("<n> false|true" per open issue)
#   api repos/<o>/<r>/<rest>        -> <rest> with / replaced by _
mkdir -p "$tmp/bin"
cat > "$tmp/bin/gh" <<'FAKE'
#!/usr/bin/env bash
key=
case "$1 $2" in
    "issue view") key="issue-$3" ;;
    "pr list")
        while [[ $# -gt 0 ]]; do [[ "$1" == --head ]] && key="prs-${2#agent/issue-}"; shift; done ;;
    "issue list")
        while [[ $# -gt 0 ]]; do [[ "$1" == --milestone ]] && key="open-$2"; shift; done ;;
    api*)
        rest="${2#repos/*/*/}"
        key="${rest//\//_}" ;;
esac
[[ -n "$key" && -f "$FAKE_FIXTURES/$key.fail" ]] && exit 1
if [[ -n "$key" && -f "$FAKE_FIXTURES/$key" ]]; then cat "$FAKE_FIXTURES/$key"; fi
FAKE
cat > "$tmp/bin/agent" <<'FAKE'
#!/usr/bin/env bash
if [[ "$1" == agent-state ]]; then echo "$FAKE_STATE"; else printf '%s\n' "$*" >> "$FAKE_LOG"; fi
FAKE
cat > "$tmp/bin/notify-send" <<'FAKE'
#!/usr/bin/env bash
printf '%s\n' "${@: -1}" >> "$FAKE_NOTES"
FAKE
chmod +x "$tmp/bin/gh" "$tmp/bin/agent" "$tmp/bin/notify-send"

export PATH="$tmp/bin:$PATH" KESTIC_AGENT_SH="$tmp/bin/agent" FAKE_FIXTURES="$fix" \
    FAKE_LOG="$tmp/log" FAKE_NOTES="$tmp/notes" XDG_STATE_HOME="$tmp/state"
NUDGE="nudge There is something new from the maintainer on issue #5 or its pull request (a comment, a review, a merge or a removed label). Read it and continue."
failures=0

fail() {
    echo "FAIL: $*" >&2
    failures=$((failures + 1))
}

# fixture <name> <line>…
fixture() {
    local name="$1"
    shift
    printf '%s\n' "$@" > "$fix/$name"
}

# pass <agent state>: one watcher pass; the agent calls land in $FAKE_LOG.
pass() {
    export FAKE_STATE="$1"
    : > "$FAKE_LOG"
    "$here/kestic-agent-watch.sh" "M4 Restore" m4 > /dev/null
}

# expect <case> <agent state> <expected agent call, or empty>: one pass with the
# current fixtures, then fixtures, notes and watcher state are cleared.
expect() {
    local got
    pass "$2"
    got="$(cat "$FAKE_LOG")"
    [[ "$got" == "$3" ]] || fail "$1: agent called with '${got//$'\n'/; }', expected '$3'"
    reset
}

reset() {
    rm -rf "$fix" "$XDG_STATE_HOME" "$FAKE_NOTES"
    mkdir -p "$fix"
    touch "$FAKE_NOTES"
}
reset

# Off or busy: nothing, whatever the issues say.
fixture "open-M4 Restore" "5 false"
expect "container off" "off 0 0" ""
fixture "open-M4 Restore" "5 false"
expect "agent busy" "busy 5 100" ""

# No current story: the lowest open issue without the label and without open blockers.
fixture "open-M4 Restore" "10 false" "3 true" "4 false" "9 false" "6 false"
fixture issues_4_dependencies_blocked_by "2"
expect "next story by number, held and blocked skipped" "none 0 0" "start 6 m4"
fixture "open-M4 Restore" "10 false" "9 false"
expect "numeric order, not text order" "none 0 0" "start 9 m4"
fixture "open-M4 Restore" "3 true" "4 false"
fixture issues_4_dependencies_blocked_by "2"
expect "everything held or blocked" "none 0 0" ""
expect "empty milestone" "none 0 0" ""
fixture "open-M4 Restore" "4 false" "6 false"
fixture issues_4_dependencies_blocked_by.fail
expect "unreadable blockers skip that issue only" "none 0 0" "start 6 m4"

# Lanes: the third field says whether the issue carries the lane label.
fixture "open-M4 Restore" "4 false false" "6 false true" "8 true true"
KESTIC_AGENT_LANE=lane:core expect "lane: only issues with the lane label" "none 0 0" "start 6 m4"
fixture "open-M4 Restore" "4 false false"
KESTIC_AGENT_LANE=lane:core expect "lane: nothing in the lane" "none 0 0" ""
fixture "open-M4 Restore" "4 false false" "6 false true"
expect "no lane: every issue" "none 0 0" "start 4 m4"
out="$(KESTIC_AGENT_LANE='x" or true' "$here/kestic-agent-watch.sh" "M4 Restore" m4 2>&1 || true)"
[[ "$out" == *"only letters, digits"* ]] || fail "lane with quotes is refused: $out"

# Current story open and not held: it stays the only one; only the maintainer's
# activity after the turn started continues the session.
fixture issue-5 "OPEN false"
fixture "open-M4 Restore" "5 false" "6 false"
fixture issues_5_comments "90 User OWNER"
expect "idle, nothing new" "idle 5 100" ""
fixture issue-5 "OPEN false"
fixture issues_5_comments "90 User OWNER" "150 User OWNER"
expect "idle, maintainer comment on the issue" "idle 5 100" "$NUDGE"
fixture issue-5 "OPEN false"
fixture issues_5_comments "150 User NONE" "160 User CONTRIBUTOR"
expect "idle, outsiders' comments do not count" "idle 5 100" ""
fixture issue-5 "OPEN false"
fixture issues_5_comments "150 Bot OWNER" "160 Bot NONE"
expect "idle, bot comments (the agent itself) do not count" "idle 5 100" ""
fixture issue-5 "OPEN false"
fixture prs-5 "21 0 User"
fixture pulls_21_reviews "130 User COLLABORATOR"
expect "idle, review on its pull request" "idle 5 100" "$NUDGE"
fixture issue-5 "OPEN false"
fixture prs-5 "21 0 User"
fixture pulls_21_comments "160 User MEMBER"
expect "idle, review comment on its pull request" "idle 5 100" "$NUDGE"
fixture issue-5 "OPEN false"
fixture prs-5 "21 150 User"
expect "idle, maintainer merged its pull request" "idle 5 100" "$NUDGE"
fixture issue-5 "OPEN false"
fixture prs-5 "21 150 Bot"
expect "idle, the agent merged its pull request itself" "idle 5 100" ""
fixture issue-5 "OPEN false"
fixture issues_5_events "150 User OWNER"
expect "idle, maintainer removed the label" "idle 5 100" "$NUDGE"
fixture issue-5 "OPEN false"
fixture issues_5_comments.fail
fixture issues_5_events "150 User OWNER"
expect "idle, activity unreadable: no decision on a partial read" "idle 5 100" ""
fixture issue-5 "OPEN false"
fixture issues_5_events "150 Bot OWNER"
expect "idle, the agent removed the label itself" "idle 5 100" ""
fixture issue-5.fail
fixture "open-M4 Restore" "6 false"
expect "current issue unreadable" "idle 5 100" ""

# A session that is gone is resumed, but not forever.
fixture issue-5 "OPEN false"
expect "session stopped" "stopped 5 100" "resume 5"
fixture issue-5 "OPEN false"
expect "no session (container restarted)" "none 5 100" "resume 5"
fixture issue-5 "OPEN false"
pass "stopped 5 100"
pass "stopped 5 101"
pass "stopped 5 102"
[[ ! -s "$FAKE_LOG" ]] || fail "third resume in a row: agent called with '$(cat "$FAKE_LOG")'"
grep -q "does not come back" "$FAKE_NOTES" || fail "third resume in a row: no notification"
reset
fixture issue-5 "OPEN false"
pass "stopped 5 100"
pass "busy 5 101"
pass "stopped 5 102"
pass "stopped 5 103"
[[ "$(cat "$FAKE_LOG")" == "resume 5" ]] || fail "resumes count again after a busy pass: '$(cat "$FAKE_LOG")'"
reset
fixture issue-5 "OPEN false"
pass "stopped 5 100"
pass "idle 5 101"
pass "stopped 5 102"
pass "stopped 5 103"
[[ "$(cat "$FAKE_LOG")" == "resume 5" ]] || fail "resumes count again after a turn: '$(cat "$FAKE_LOG")'"
reset

# Each agent instance counts its own resumes: two resumes of the default
# instance leave a second instance free to resume its own session.
fixture issue-5 "OPEN false"
pass "stopped 5 100"
pass "stopped 5 101"
KESTIC_AGENT_NAME=other-agent pass "stopped 5 100"
[[ "$(cat "$FAKE_LOG")" == "resume 5" ]] || fail "second instance shares the resume count: '$(cat "$FAKE_LOG")'"
[[ -s "$XDG_STATE_HOME/kestic-agent/watch-resume-other-agent" ]] || fail "second instance: no own resume file"
reset

# Current story closed or held: the next one starts; a hold is reported once.
fixture issue-5 "CLOSED false"
fixture "open-M4 Restore" "6 false"
expect "current story closed" "idle 5 100" "start 6 m4"
fixture issue-5 "OPEN true"
fixture "open-M4 Restore" "5 true" "6 false"
fixture issues_5_comments "150 User OWNER"
pass "idle 5 100"
[[ "$(cat "$FAKE_LOG")" == "start 6 m4" ]] || fail "current story held: '$(cat "$FAKE_LOG")'"
pass "idle 5 100"
[[ "$(grep -c 'waits for the maintainer' "$FAKE_NOTES")" == 1 ]] || fail "hold not reported exactly once"
reset

# An agent idle for hours without a result is reported once.
fixture issue-5 "OPEN false"
pass "idle 5 1000"
pass "idle 5 1000"
[[ ! -s "$FAKE_LOG" ]] || fail "long idle: agent called with '$(cat "$FAKE_LOG")'"
[[ "$(grep -c 'idle on #5' "$FAKE_NOTES")" == 1 ]] || fail "long idle not reported exactly once"
reset

# The state line comes from the container: it must not run code on the host.
fixture issue-5 "OPEN false"
fixture issues_5_comments "150 User OWNER"
pass 'idle 5 base[$(touch '"$tmp"'/pwned)]'
[[ ! -e "$tmp/pwned" ]] || fail "turn from the container ran a command on the host"
reset
pass 'idle 5$(touch '"$tmp"'/pwned2) 100'
[[ ! -e "$tmp/pwned2" ]] || fail "issue from the container ran a command on the host"
[[ ! -s "$FAKE_LOG" ]] || fail "invalid issue: agent called with '$(cat "$FAKE_LOG")'"
reset
expect "unknown state" "weird 5 100" ""
expect "no state line" "" ""
fixture issue-5 "OPEN true"
fixture "open-M4 Restore" "5 true"
expect "stopped session on a held issue is not resumed" "stopped 5 100" ""

# A failed token renewal is reported once per hour.
pass "busy 5 100 failed"
pass "busy 5 100 failed"
[[ "$(grep -c 'token failed' "$FAKE_NOTES")" == 1 ]] || fail "token failure not reported exactly once"
reset

# A session stopped at a usage limit is continued once the limit has reset,
# once per turn; before that it is neither continued nor reported as idle.
LIMIT_NUDGE="nudge The usage limit has reset. Continue where you stopped; read your state comment on the issue first."
fixture issue-5 "OPEN false"
pass "idle 5 100 ok 200"
[[ "$(cat "$FAKE_LOG")" == "$LIMIT_NUDGE" ]] || fail "limit reset: agent called with '$(cat "$FAKE_LOG")'"
pass "idle 5 100 ok 200"
[[ ! -s "$FAKE_LOG" ]] || fail "limit reset, second pass: agent called with '$(cat "$FAKE_LOG")'"
reset
fixture issue-5 "OPEN false"
pass "idle 5 100 ok $(($(date +%s) + 3600))"
[[ ! -s "$FAKE_LOG" ]] || fail "limit not yet reset: agent called with '$(cat "$FAKE_LOG")'"
! grep -q 'idle on #5' "$FAKE_NOTES" || fail "limit not yet reset: reported as idle"
reset
fixture issue-5 "OPEN false"
pass 'idle 5 100 ok $(touch '"$tmp"'/pwned3)'
[[ ! -e "$tmp/pwned3" ]] || fail "reset time from the container ran a command on the host"
[[ "$(cat "$FAKE_LOG")" != "$LIMIT_NUDGE" ]] || fail "invalid reset time: continued as after a limit"
reset

# While the limit has not reset, the maintainer's news waits too; afterwards it
# continues the session as usual.
fixture issue-5 "OPEN false"
fixture prs-5 ""
fixture issues_5_comments "150 User OWNER"
pass "idle 5 100 ok $(($(date +%s) + 3600))"
[[ ! -s "$FAKE_LOG" ]] || fail "news during a limit: agent called with '$(cat "$FAKE_LOG")'"
pass "idle 5 100 ok 200"
[[ "$(cat "$FAKE_LOG")" == "$NUDGE" ]] || fail "news after the limit: agent called with '$(cat "$FAKE_LOG")'"
reset

# Once the limit nudge is used and the turn does not move on, the long-idle
# report still comes.
fixture issue-5 "OPEN false"
pass "idle 5 100 ok 200"
pass "idle 5 100 ok 200"
grep -q 'idle on #5' "$FAKE_NOTES" || fail "limit nudge used: long idle not reported"
reset

# A reset time with a leading zero does not stop the watcher.
fixture issue-5 "OPEN false"
pass "idle 5 100 ok 0200" || fail "reset time 0200: watcher failed"
[[ "$(cat "$FAKE_LOG")" == "$LIMIT_NUDGE" ]] || fail "reset time 0200: agent called with '$(cat "$FAKE_LOG")'"
reset

# A container that is not running is reported once per hour, nothing is started.
fixture "open-M4 Restore" "6 false"
pass "off 0 0 ok 0"
pass "off 0 0 ok 0"
[[ ! -s "$FAKE_LOG" ]] || fail "container off: agent called with '$(cat "$FAKE_LOG")'"
[[ "$(grep -c 'not running' "$FAKE_NOTES")" == 1 ]] || fail "container off not reported exactly once"
reset

if ((failures > 0)); then
    echo "test-kestic-agent-watch: $failures failure(s)" >&2
    exit 1
fi
echo "test-kestic-agent-watch: ok"
