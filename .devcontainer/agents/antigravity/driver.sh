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
    if [[ ! -e "$HOME/.config/antigravity" && ! -L "$HOME/.config/antigravity" ]]; then
        ln -s "$HOME/.gemini/antigravity-auth" "$HOME/.config/antigravity"
    fi

    # Pre-trust the cloned repository so autonomous runs do not stop at a
    # workspace trust prompt.
    local settings="$HOME/.gemini/antigravity-cli/settings.json"
    if [[ ! -s "$settings" ]]; then
        echo '{}' > "$settings"
    fi
    jq --arg dir "$REPO_DIR" \
        '.trustedWorkspaces = ((.trustedWorkspaces // []) | if index($dir) then . else . + [$dir] end)' \
        "$settings" > "$settings.tmp"
    mv "$settings.tmp" "$settings"

    # Register the kestic-agent lifecycle hooks in ~/.gemini/config/hooks.json
    # (Antigravity CLI loads hooks from hooks.json, not from settings.json).
    local hooks="$HOME/.gemini/config/hooks.json"
    if [[ ! -s "$hooks" ]]; then
        echo '{}' > "$hooks"
    fi
    jq -s '.[0] * .[1]' "$hooks" "$AGENT_DIR/hooks.json" > "$hooks.tmp"
    mv "$hooks.tmp" "$hooks"
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
