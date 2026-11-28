# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause
#
# Runtime driver for Claude Code, sourced by kestic-agent-inner.sh.

export CLAUDE_CONFIG_DIR="${CLAUDE_CONFIG_DIR:-$HOME/.claude}"
export DISABLE_AUTOUPDATER=1
export DISABLE_TELEMETRY=1
export DISABLE_ERROR_REPORTING=1

AGENT_DISPLAY_NAME="Claude Code"

agent_setup() {
    mkdir -p "$CLAUDE_CONFIG_DIR"
    # Bypass mode is the point of this sandbox; skip its one-time confirmation.
    local settings="$CLAUDE_CONFIG_DIR/settings.json"
    if [[ ! -s "$settings" ]]; then
        echo '{}' > "$settings"
    fi
    jq '.skipDangerousModePermissionPrompt = true' "$settings" > "$settings.tmp"
    mv "$settings.tmp" "$settings"
}

agent_login() {
    exec claude --dangerously-skip-permissions --settings "$AGENT_DIR/settings.json"
}

# Populates AGENT_CMD for `start` or `resume`.
agent_launch_cmd() {
    local mode="$1" issue="$2" prompt="$3"
    # The settings come by flag, not as managed settings in /etc/claude-code:
    # an organisation's server-managed settings replace that file completely.
    AGENT_CMD=(claude --dangerously-skip-permissions --settings "$AGENT_DIR/settings.json")
    if [[ "${KESTIC_AGENT_REMOTE_CONTROL:-1}" != 0 ]]; then
        AGENT_CMD+=(--remote-control "$HOSTNAME #$issue")
    fi
    if [[ -n "${KESTIC_AGENT_MODEL:-}" ]]; then
        AGENT_CMD+=(--model "$KESTIC_AGENT_MODEL")
    fi
    if [[ -n "${KESTIC_AGENT_EFFORT:-}" ]]; then
        AGENT_CMD+=(--effort "$KESTIC_AGENT_EFFORT")
    fi
    if [[ "$mode" == resume ]]; then
        AGENT_CMD+=(--continue)
    fi
    AGENT_CMD+=("$prompt")
}

# Extracts the reset epoch when Claude Code stopped at a usage limit
# («… hit your … limit · resets 4:10am (UTC)»), or prints 0.
agent_limit_reset() {
    local turn="$1" pane="$2" at reset
    # The message may wrap onto the next line, so the lines are searched as one.
    at="$(printf '%s\n' "$pane" | grep -v '^[[:space:]]*$' | tail -n 15 |
        tr '\n' ' ' | grep 'hit your' | grep -o 'resets [0-9][0-9:]*[ap]m (UTC)' | tail -n 1 || true)"
    at="${at#resets }"
    at="${at% (UTC)}"
    if [[ -z "$at" || -z "$turn" ]]; then
        echo 0
    elif reset="$(date -u -d "$(date -u -d "@$turn" +%F) $at" +%s 2>/dev/null)"; then
        ((reset >= turn)) || reset=$((reset + 86400))
        if ((reset - $(date +%s) > 5 * 3600 + 900)); then reset=0; fi
        echo "$reset"
    else
        echo 0
    fi
}
