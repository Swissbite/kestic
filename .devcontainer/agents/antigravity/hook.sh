#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause
#
# Antigravity CLI lifecycle hook bridge for kestic-agent turn/idle tracking.
# Receives hook JSON on stdin and prints the required JSON response on stdout.

set -euo pipefail

AGENT_BIN="${KESTIC_AGENT_BIN:-/usr/local/bin/kestic-agent}"
STATE_DIR="$HOME/.kestic-agent"
payload="$(cat || true)"

case "${1:-}" in
    pre-invocation)
        # Only start a new turn when coming out of idle (or on first turn), so
        # intermediate model calls inside one turn do not advance the turn
        # timestamp past a maintainer comment posted mid-turn.
        if [[ -e "$STATE_DIR/idle" || ! -e "$STATE_DIR/turn" ]]; then
            "$AGENT_BIN" hook turn >/dev/null 2>&1 || true
        fi
        printf '{}\n'
        ;;
    stop)
        # Do not mark the session idle while background tasks are still active
        # and will wake the agent again.
        fully_idle="$(jq -r 'if type == "object" and has("fullyIdle") then .fullyIdle else true end' <<< "$payload" 2>/dev/null || echo true)"
        if [[ "$fully_idle" != false ]]; then
            "$AGENT_BIN" hook idle >/dev/null 2>&1 || true
        fi
        printf '{"decision":"allow"}\n'
        ;;
    *)
        echo "antigravity/hook.sh: usage: $0 pre-invocation|stop" >&2
        exit 2
        ;;
esac
