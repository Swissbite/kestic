# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause
#
# Runtime driver for Antigravity CLI (`agy`), sourced by kestic-agent-inner.sh.

export AGY_CLI_DISABLE_AUTO_UPDATE=true

AGENT_DISPLAY_NAME="Antigravity CLI"

agent_setup() {
    mkdir -p "$HOME/.gemini/antigravity-cli" "$HOME/.gemini/config" \
        "$HOME/.gemini/antigravity-auth" "$HOME/.config"
    chmod 700 "$HOME/.gemini/antigravity-auth"

    # Keep file-based OAuth credentials inside the persistent agent volume
    # (~/.gemini), so recreating the home volume keeps the login intact.
    local auth_link="$HOME/.config/antigravity"
    local auth_target="$HOME/.gemini/antigravity-auth"
    if [[ -d "$auth_link" && ! -L "$auth_link" ]]; then
        rmdir "$auth_link" 2>/dev/null || true
    fi
    if [[ ! -e "$auth_link" && ! -L "$auth_link" ]]; then
        ln -s "$auth_target" "$auth_link"
    elif [[ ! -L "$auth_link" || "$(readlink "$auth_link")" != "$auth_target" ]]; then
        echo "kestic-agent: warning: $auth_link does not point to $auth_target; credentials may live in the home volume" >&2
    fi

    # Pre-trust the cloned repository so autonomous runs do not stop at a
    # workspace trust prompt.
    local settings="$HOME/.gemini/antigravity-cli/settings.json" tmp
    if [[ ! -s "$settings" ]]; then
        echo '{}' > "$settings"
    fi
    tmp="$(mktemp "$HOME/.gemini/antigravity-cli/settings.json.XXXXXX")"
    jq --arg dir "$REPO_DIR" \
        '.trustedWorkspaces = ((.trustedWorkspaces // []) | if index($dir) then . else . + [$dir] end)' \
        "$settings" > "$tmp"
    mv "$tmp" "$settings"

    # Register the kestic-agent lifecycle hooks in ~/.gemini/config/hooks.json
    # (Antigravity CLI loads hooks from hooks.json, not from settings.json).
    local hooks="$HOME/.gemini/config/hooks.json"
    if [[ ! -s "$hooks" ]]; then
        echo '{}' > "$hooks"
    fi
    tmp="$(mktemp "$HOME/.gemini/config/hooks.json.XXXXXX")"
    jq -s '.[0] * .[1]' "$hooks" "$AGENT_DIR/hooks.json" > "$tmp"
    mv "$tmp" "$hooks"
}

agent_login() {
    # Setting SSH_CONNECTION makes agy use the headless copy-paste OAuth URL
    # flow immediately instead of attempting to launch a local browser.
    exec env SSH_CONNECTION="${SSH_CONNECTION:-127.0.0.1 1 127.0.0.1 22}" \
        agy --dangerously-skip-permissions --settings "$AGENT_DIR/settings.json"
}

# Populates AGENT_CMD for `start` or `resume`.
agent_launch_cmd() {
    local mode="$1" _issue="$2" prompt="$3"
    AGENT_CMD=(agy --dangerously-skip-permissions --settings "$AGENT_DIR/settings.json")
    if [[ "${KESTIC_AGENT_REMOTE_CONTROL:-1}" != 0 ]]; then
        AGENT_CMD+=(--remote-control)
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
    AGENT_CMD+=(-i "$prompt")
}

agent_limit_reset() {
    echo 0
}
