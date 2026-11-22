#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause
#
# Tests the decisions of kestic-agent-watch.sh against a fake gh and a fake
# kestic-agent.sh: which issue starts, when a session is continued or resumed,
# and when nothing happens. The fake gh answers with what the real one prints
# after its --jq filter, so the filters themselves are not covered here.

set -euo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
fix="$tmp/fixtures"

# Fake gh: prints the fixture file named after the call, or nothing.
#   issue view <n>                  -> issue-<n>      ("<n> false|true" or "<n> closed")
#   pr list --head agent/issue-<n>  -> prs-<n>        (pull request numbers)
#   issue list --milestone <m>      -> open-<m>       ("<n> false|true" per open issue)
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
if [[ -n "$key" && -f "$FAKE_FIXTURES/$key" ]]; then cat "$FAKE_FIXTURES/$key"; fi
FAKE
cat > "$tmp/bin/agent" <<'FAKE'
#!/usr/bin/env bash
if [[ "$1" == agent-state ]]; then echo "$FAKE_STATE"; else printf '%s\n' "$*" >> "$FAKE_LOG"; fi
FAKE
chmod +x "$tmp/bin/gh" "$tmp/bin/agent"

export PATH="$tmp/bin:$PATH" KESTIC_AGENT_SH="$tmp/bin/agent" FAKE_FIXTURES="$fix" FAKE_LOG="$tmp/log"
failures=0

# fixture <name> <line>…
fixture() {
    local name="$1"
    shift
    printf '%s\n' "$@" > "$fix/$name"
}

# expect <case> <agent state> <expected agent call, or empty>
expect() {
    local case="$1" want="$3" got
    export FAKE_STATE="$2"
    : > "$FAKE_LOG"
    "$here/kestic-agent-watch.sh" "M4 Restore" m4 > /dev/null
    got="$(cat "$FAKE_LOG")"
    if [[ "$got" != "$want" ]]; then
        echo "FAIL: $case: agent called with '${got//$'\n'/; }', expected '$want'" >&2
        failures=$((failures + 1))
    fi
    rm -rf "$fix"
    mkdir -p "$fix"
}
mkdir -p "$fix"

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

# Current story open and not held: it stays the only one.
fixture issue-5 "5 false"
fixture "open-M4 Restore" "5 false" "6 false"
fixture issues_5_comments "90 false" "150 true"
expect "idle, no new comment from the maintainer" "idle 5 100" ""
fixture issue-5 "5 false"
fixture issues_5_comments "90 false" "150 false"
expect "idle, new comment on the issue" "idle 5 100" \
    "nudge There is a new comment on issue #5 or its pull request. Read it and continue."
fixture issue-5 "5 false"
fixture prs-5 "21"
fixture issues_21_comments "120 true"
fixture pulls_21_comments "80 false"
fixture pulls_21_reviews "130 false"
expect "idle, new review on its pull request" "idle 5 100" \
    "nudge There is a new comment on issue #5 or its pull request. Read it and continue."
fixture issue-5 "5 false"
fixture prs-5 "21"
fixture pulls_21_comments "160 false"
expect "idle, new review comment on its pull request" "idle 5 100" \
    "nudge There is a new comment on issue #5 or its pull request. Read it and continue."
fixture issue-5 "5 false"
expect "session stopped" "stopped 5 100" "resume 5"
fixture issue-5 "5 false"
expect "no session (container restarted)" "none 5 100" "resume 5"

# Current story closed or held: the next one starts.
fixture issue-5 "5 closed"
fixture "open-M4 Restore" "6 false"
expect "current story closed" "idle 5 100" "start 6 m4"
fixture issue-5 "5 true"
fixture "open-M4 Restore" "5 true" "6 false"
fixture issues_5_comments "150 false"
expect "current story waits for the maintainer" "idle 5 100" "start 6 m4"

if ((failures > 0)); then
    echo "test-kestic-agent-watch: $failures failure(s)" >&2
    exit 1
fi
echo "test-kestic-agent-watch: ok"
