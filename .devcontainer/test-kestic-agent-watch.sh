#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause
#
# Tests the decisions of kestic-agent-watch.sh against a fake gh, a fake
# kestic-agent.sh and a fake notify-send: which issue starts, when a session is
# continued or resumed, whose activity counts, what is reported to the
# maintainer, which issues are skipped as written by outsiders, and that the
# agent's state line cannot run code on the host. For comments, events, pull
# requests and issues the fake gh runs the watcher's real --jq filter over raw
# GitHub JSON (fixtures <name>.json); other fixtures hold what the real gh
# prints after its --jq filter, checked against the real GitHub API by hand
# (README.md).

set -euo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
fix="$tmp/fixtures"

# Fake gh: prints the fixture file named after the call, or nothing; fails if a
# file <name>.fail exists. For `api`, a fixture <name>.json is raw GitHub JSON:
# the call's --jq filter runs over it with jq, or it is printed as it is.
#   issue view <n>                  -> issue-<n>   ("OPEN false|true" or "CLOSED false")
#   issue list --milestone <m>      -> open-<m>    ("<n> false|true" per open issue)
#   api [--paginate] repos/<o>/<r>/<rest>
#                                   -> <rest> with / ? & = : replaced by _
#   api graphql … -F number=<n>     -> graphql-<n>
# An issue without a fixture (api repos/<o>/<r>/issues/<n>) is the maintainer's.
mkdir -p "$tmp/bin"
cat > "$tmp/bin/gh" <<'FAKE'
#!/usr/bin/env bash
key= filter=
case "$1 $2" in
    "issue view") key="issue-$3" ;;
    "issue list")
        while [[ $# -gt 0 ]]; do [[ "$1" == --milestone ]] && key="open-$2"; shift; done ;;
    api*)
        shift
        while [[ $# -gt 0 ]]; do
            case "$1" in
                --jq) filter="$2"; shift ;;
                graphql) key=graphql ;;
                number=*) [[ "$key" == graphql ]] && key="graphql-${1#number=}" ;;
                repos/*) [[ -z "$key" ]] && key="$(tr '/?&=:' '_____' <<< "${1#repos/*/*/}")" ;;
            esac
            shift
        done ;;
esac
[[ -n "$key" && -f "$FAKE_FIXTURES/$key.fail" ]] && exit 1
if [[ -n "$key" && -f "$FAKE_FIXTURES/$key.json" ]]; then
    if [[ -n "$filter" ]]; then jq -r "$filter" "$FAKE_FIXTURES/$key.json"; else cat "$FAKE_FIXTURES/$key.json"; fi
elif [[ -n "$key" && -f "$FAKE_FIXTURES/$key" ]]; then
    cat "$FAKE_FIXTURES/$key"
elif [[ "$key" =~ ^issues_([0-9]+)$ ]]; then
    printf '{"number":%s,"user":{"login":"Swissbite","id":205621,"type":"User"},"labels":[]}\n' "${BASH_REMATCH[1]}"
fi
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

# Users by numeric ID: the maintainer, the App, an outsider, a collaborator who
# is not the maintainer, another bot, and someone with the maintainer's login
# but another ID.
M='{"login":"Swissbite","id":205621,"type":"User"}'
A='{"login":"kestic-agent[bot]","id":359840804,"type":"Bot"}'
O='{"login":"mallory","id":4242,"type":"User"}'
C='{"login":"helper","id":777,"type":"User"}'
R='{"login":"renovate[bot]","id":29139614,"type":"Bot"}'
F='{"login":"Swissbite","id":1,"type":"User"}'

iso() {
    date -u -d "@$1" +%Y-%m-%dT%H:%M:%SZ
}

# comments <name> "<epoch> <user variable>"…: raw comments (or reviews with
# created_at, the filter takes either) by those users at those times
comments() {
    local name="$1" items=() e u
    shift
    for entry in "$@"; do
        read -r e u <<< "$entry"
        items+=("{\"created_at\":\"$(iso "$e")\",\"user\":${!u},\"author_association\":\"OWNER\"}")
    done
    (IFS=,; echo "[${items[*]}]") > "$fix/$name.json"
}

# reviews <name> "<epoch> <user variable>"…: submitted reviews
reviews() {
    local name="$1" items=() e u
    shift
    for entry in "$@"; do
        read -r e u <<< "$entry"
        items+=("{\"submitted_at\":\"$(iso "$e")\",\"user\":${!u},\"state\":\"COMMENTED\"}")
    done
    (IFS=,; echo "[${items[*]}]") > "$fix/$name.json"
}

# unlabel <issue> <epoch> <user variable>: needs-maintainer removed
unlabel() {
    echo "[{\"event\":\"unlabeled\",\"label\":{\"name\":\"needs-maintainer\"},\"actor\":${!3},\"created_at\":\"$(iso "$2")\"}]" \
        > "$fix/issues_$1_events.json"
}

# prs <issue> "<pr> <merged epoch|0> <author variable> <head repository>"…:
# pull requests from the branch agent/issue-<issue>
prs() {
    local issue="$1" items=() n m u r merged
    shift
    for entry in "$@"; do
        read -r n m u r <<< "$entry"
        merged=null
        [[ "$m" != 0 ]] && merged="\"$(iso "$m")\""
        items+=("{\"number\":$n,\"merged_at\":$merged,\"user\":${!u},\"head\":{\"ref\":\"agent/issue-$issue\",\"repo\":{\"full_name\":\"$r\"}},\"base\":{\"repo\":{\"full_name\":\"Swissbite/kestic\"}}}")
    done
    (IFS=,; echo "[${items[*]}]") > "$fix/pulls_state_all_head_Swissbite_agent_issue-${issue}_per_page_100.json"
}

# merged_by <pr> <user variable>
merged_by() {
    echo "{\"number\":$1,\"merged_by\":${!2}}" > "$fix/pulls_$1.json"
}

# issue_by <n> <user variable> [<labels json>]: who opened issue <n>
issue_by() {
    echo "{\"number\":$1,\"user\":${!2},\"labels\":${3:-[]},\"title\":\"t\",\"body\":\"b\"}" > "$fix/issues_$1.json"
}

# adopted <n> <user variable> <epoch> [<last edit epoch>]: label adopted
# added, and when the body was last edited
adopted() {
    echo "[{\"event\":\"labeled\",\"label\":{\"name\":\"adopted\"},\"actor\":${!2},\"created_at\":\"$(iso "$3")\"}]" \
        > "$fix/issues_$1_events.json"
    local edited=null
    [[ -n "${4:-}" ]] && edited="\"$(iso "$4")\""
    echo "{\"data\":{\"repository\":{\"issue\":{\"lastEditedAt\":$edited}}}}" > "$fix/graphql-$1.json"
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
comments issues_5_comments "90 M"
expect "idle, nothing new" "idle 5 100" ""
fixture issue-5 "OPEN false"
comments issues_5_comments "90 M" "150 M"
expect "idle, maintainer comment on the issue" "idle 5 100" "$NUDGE"
fixture issue-5 "OPEN false"
comments issues_5_comments "150 O" "160 C"
expect "idle, outsiders' and other collaborators' comments do not count" "idle 5 100" ""
fixture issue-5 "OPEN false"
comments issues_5_comments "150 A" "160 R"
expect "idle, bot comments (the agent itself) do not count" "idle 5 100" ""
fixture issue-5 "OPEN false"
comments issues_5_comments "150 F"
expect "idle, the maintainer's login with another ID does not count" "idle 5 100" ""
fixture issue-5 "OPEN false"
echo '[{"created_at":"1970-01-01T00:02:30Z","user":null}]' > "$fix/issues_5_comments.json"
expect "idle, a comment without user does not count" "idle 5 100" ""
fixture issue-5 "OPEN false"
prs 5 "21 0 A Swissbite/kestic"
reviews pulls_21_reviews "130 M"
expect "idle, review on its pull request" "idle 5 100" "$NUDGE"
fixture issue-5 "OPEN false"
prs 5 "21 0 A Swissbite/kestic"
comments pulls_21_comments "160 M"
expect "idle, review comment on its pull request" "idle 5 100" "$NUDGE"
fixture issue-5 "OPEN false"
prs 5 "21 0 A Swissbite/kestic"
comments issues_21_comments "160 M"
expect "idle, comment on its pull request" "idle 5 100" "$NUDGE"
fixture issue-5 "OPEN false"
prs 5 "21 150 A Swissbite/kestic"
merged_by 21 M
expect "idle, maintainer merged its pull request" "idle 5 100" "$NUDGE"
fixture issue-5 "OPEN false"
prs 5 "21 150 A Swissbite/kestic"
merged_by 21 A
expect "idle, the agent merged its pull request itself" "idle 5 100" ""
fixture issue-5 "OPEN false"
prs 5 "22 0 A mallory/kestic" "23 0 O Swissbite/kestic"
reviews pulls_22_reviews "130 M"
comments issues_22_comments "140 M"
reviews pulls_23_reviews "130 M"
expect "idle, a fork's or someone else's pull request named agent/issue-5 does not count" "idle 5 100" ""
fixture issue-5 "OPEN false"
prs 5 "22 150 A mallory/kestic"
merged_by 22 M
expect "idle, merging a fork's pull request named agent/issue-5 does not count" "idle 5 100" ""
fixture issue-5 "OPEN false"
unlabel 5 150 M
expect "idle, maintainer removed the label" "idle 5 100" "$NUDGE"
fixture issue-5 "OPEN false"
fixture issues_5_comments.fail
unlabel 5 150 M
expect "idle, activity unreadable: no decision on a partial read" "idle 5 100" ""
fixture issue-5 "OPEN false"
unlabel 5 150 A
expect "idle, the agent removed the label itself" "idle 5 100" ""
fixture issue-5 "OPEN false"
unlabel 5 150 C
expect "idle, another collaborator removed the label" "idle 5 100" ""
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
comments issues_5_comments "150 M"
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
comments issues_5_comments "150 M"
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
comments issues_5_comments "150 M"
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

# Issues opened by someone other than the maintainer or the App are skipped and
# reported once; the maintainer's label adopted starts them, unless the text
# changed afterwards.
fixture "open-M4 Restore" "6 false" "7 false"
issue_by 6 O
echo '[]' > "$fix/issues_6_events.json"
pass "none 0 0"
[[ "$(cat "$FAKE_LOG")" == "start 7 m4" ]] || fail "outsider's issue: agent called with '$(cat "$FAKE_LOG")'"
pass "none 0 0"
[[ "$(grep -c '#6 was opened by an outsider' "$FAKE_NOTES")" == 1 ]] || fail "outsider's issue not reported exactly once"
reset
fixture "open-M4 Restore" "6 false"
issue_by 6 R
echo '[]' > "$fix/issues_6_events.json"
expect "another bot's issue is skipped" "none 0 0" ""
fixture "open-M4 Restore" "6 false"
issue_by 6 A
expect "the App's own issue starts" "none 0 0" "start 6 m4"
fixture "open-M4 Restore" "6 false"
issue_by 6 O '[{"name":"adopted"}]'
adopted 6 M 1000
expect "issue adopted by the maintainer starts" "none 0 0" "start 6 m4"
fixture "open-M4 Restore" "6 false"
issue_by 6 O '[{"name":"adopted"}]'
adopted 6 A 1000
expect "label adopted added by the App does not count" "none 0 0" ""
fixture "open-M4 Restore" "6 false"
issue_by 6 O '[{"name":"adopted"}]'
adopted 6 C 1000
expect "label adopted added by another collaborator does not count" "none 0 0" ""
fixture "open-M4 Restore" "6 false" "7 false"
issue_by 6 O '[{"name":"adopted"}]'
adopted 6 M 1000 2000
pass "none 0 0"
[[ "$(cat "$FAKE_LOG")" == "start 7 m4" ]] || fail "edited after adoption: agent called with '$(cat "$FAKE_LOG")'"
grep -q '#6 was edited after it was adopted' "$FAKE_NOTES" || fail "edited after adoption: not reported"
reset
fixture "open-M4 Restore" "6 false" "7 false"
fixture issues_6.fail
expect "author unreadable: that issue is skipped" "none 0 0" "start 7 m4"

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
