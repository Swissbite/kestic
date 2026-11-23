#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause
#
# `kestic-agent` inside the agent container, run as kesticci.
#
#   kestic-agent setup              check the token, clone or update the repository,
#                                   git identity and hooks, bootstrap.sh (toolchain, restic)
#   kestic-agent login              setup, then Claude Code interactively (one-time /login)
#   kestic-agent run <issue> [<base>]  setup, then Claude Code in tmux session `agent` on the
#                                   issue, branching from and opening the pull request against
#                                   <base> (default main)
#   kestic-agent resume <issue>     Claude Code continues its last session (after a stop)
#   kestic-agent nudge <text…>      types <text> into the running Claude session
#   kestic-agent status [lines]     the last lines of the agent's terminal
#   kestic-agent state              one line `<state> <issue> <turn start>` for the watcher;
#                                   state is none, stopped (Claude Code exited), idle or busy
#   kestic-agent hook turn|idle     called by Claude Code hooks (agent-settings.json) when a
#                                   turn starts and when it ends
#   kestic-agent store-token        reads a GitHub token from stdin into the token file
#   kestic-agent token-age          seconds since the token file was written, or none
#   kestic-agent show-token         prints the token (for revoking it on stop)
#
# The GitHub issue is the agent's whole assignment; progress goes back as issue
# comments and a pull request.

set -euo pipefail

REPO_SLUG="${KESTIC_AGENT_REPO:-Swissbite/kestic}"
REPO_DIR="$HOME/kestic"
# A GitHub App installation token, written by kestic-agent.sh on the host
# through `store-token` (tmpfs, not in a volume).
TOKEN_FILE="${KESTIC_AGENT_TOKEN_FILE:-/run/kestic-agent-token/gh-token}"
SESSION=agent
STATE_DIR="$HOME/.kestic-agent"
# Hooks, attribution, deny rules and effort for every agent session (root-owned).
AGENT_SETTINGS=/usr/local/lib/kestic-agent/agent-settings.json
# A busy session whose terminal shows no change for this long is waiting for
# input although the Stop hook did not run (interrupt, API error, usage limit).
STALE_AFTER=900

die() {
    echo "kestic-agent: $*" >&2
    exit 1
}

setup() {
    [[ -r "$TOKEN_FILE" ]] || die "no GitHub token at $TOKEN_FILE (kestic-agent.sh writes it)"
    gh api "repos/$REPO_SLUG" --jq .full_name >/dev/null || die "the GitHub token is not accepted"
    local name="${KESTIC_AGENT_GIT_NAME:-}" email="${KESTIC_AGENT_GIT_EMAIL:-}"
    [[ -n "$name" && -n "$email" ]] || die "KESTIC_AGENT_GIT_NAME and KESTIC_AGENT_GIT_EMAIL are not set (agent.conf)"

    git config --global user.name "$name"
    git config --global user.email "$email"
    git config --global core.hooksPath /usr/local/lib/kestic-agent/git-hooks
    git config --global init.defaultBranch main
    # The token is read from its file for each git operation and never stored.
    git config --global --replace-all credential.https://github.com.helper \
        "!f() { test \"\$1\" = get || exit 0; echo username=x-access-token; echo \"password=\$(cat $TOKEN_FILE)\"; }; f"

    if [[ ! -d "$REPO_DIR/.git" ]]; then
        git clone "https://github.com/$REPO_SLUG.git" "$REPO_DIR"
    else
        git -C "$REPO_DIR" fetch --prune origin
    fi

    local wanted_tag image_tag
    wanted_tag="$(git -C "$REPO_DIR" show origin/main:.github/ci-image/Dockerfile origin/main:scripts/bootstrap.sh | sha256sum | cut -c1-16)"
    image_tag="${KESTIC_CI_IMAGE##*:}"
    if [[ "$wanted_tag" != "$image_tag" ]]; then
        echo "kestic-agent: warning: image is based on CI image $image_tag, origin/main needs $wanted_tag;" \
            "system packages may differ (rebuild with kestic-agent.sh build)" >&2
    fi

    (
        cd "$REPO_DIR"
        KESTIC_BOOTSTRAP_SKIP_APT=1 \
        KESTIC_BOOTSTRAP_COMPONENTS=rust,restic-0.18.0,restic-latest,rest-server,rclone \
            ./scripts/bootstrap.sh
    )

    # Bypass mode is the point of this sandbox; skip its one-time confirmation.
    local settings="$CLAUDE_CONFIG_DIR/settings.json"
    if [[ ! -s "$settings" ]]; then
        echo '{}' > "$settings"
    fi
    jq '.skipDangerousModePermissionPrompt = true' "$settings" > "$settings.tmp"
    mv "$settings.tmp" "$settings"
    echo "kestic-agent: ready in $REPO_DIR, commits as $name <$email>"
}

base_branch() {
    local base="${1:-main}"
    [[ "$base" =~ ^[A-Za-z0-9][A-Za-z0-9._/-]*$ ]] || die "invalid base branch: $base"
    printf '%s' "$base"
}

issue_number() {
    [[ "${1:-}" =~ ^[0-9]+$ ]] || die "usage: kestic-agent $2 <issue number>"
    printf '%s' "$1"
}

start_prompt() {
    local issue="$1" base="$2" skill=".agents/skills/kestic-milestone-autonomy/SKILL.md"
    cat <<EOF
You are the Kestic coding agent, working autonomously in an isolated container. Your assignment is GitHub issue #$issue in $REPO_SLUG. Your base branch is \`$base\`.

1. Run \`git fetch origin\`; the working tree must be clean. Read AGENTS.md, then the issue with \`gh issue view $issue --comments\`. The issue is your specification; comments by the maintainer refine it.
   If the base branch is not main, read \`git show origin/$base:$skill\`: the issues of that milestone follow it, and where it differs from the steps below, it wins. If that file does not exist on origin/$base, ask (step 6) and do nothing else.
2. Post a short comment on the issue that you have started (\`gh issue comment $issue\`).
3. If the branch \`agent/issue-$issue\` or a pull request from it already exists, continue from there and read its comments. Otherwise create the branch from origin/$base. Implement the issue as AGENTS.md describes (spec, one test per acceptance criterion, i18n, REUSE).
4. Run \`just verify\` until it is green. Then let two review subagents check your diff (one for correctness, one for tests and conformity with the spec and AGENTS.md) and fix what they find. After the fixes run the complete \`just verify\` again, not only single checks. Open the pull request only when it is green; if a failure comes from the container and not from your change, say so in the pull request body.
5. Push the branch and open a pull request against $base with \`gh pr create --base $base\`. The body explains the change and how it was verified; against main it ends with \`Closes #$issue\`. Comment the pull request link on the issue. A pull request into another branch than main does not close the issue.
6. If you are blocked or a decision belongs to the maintainer (AGENTS.md, "Stop & Ask"), post the question with options and your recommendation as an issue comment, add the label \`needs-maintainer\` to the issue (\`gh issue edit $issue --add-label needs-maintainer\`) and end your turn.

Waiting: do not poll for comments. When you wait for the maintainer, end your turn; a watcher continues this session when the maintainer comments, reviews, merges or removes the label. To wait for CI, run \`gh pr checks <pr> --watch\` with a Bash timeout of 40 minutes.

CI logs: \`gh pr checks\` lists the jobs; read a job's log with \`gh api repos/$REPO_SLUG/actions/jobs/<job id>/logs\` (\`gh run view --log\` prints nothing with the gh version in this container).

Rules: never merge into main (the maintainer does that), never push to main, never change .github/workflows. Commits, pull requests and comments carry no AI attribution: no Co-Authored-By trailer, no "Generated with" line. Keep comments short, factual and in English.
EOF
}

resume_prompt() {
    local issue="$1"
    cat <<EOF
Continue with GitHub issue #$issue. First run \`git fetch origin\` and read what is new on the issue (\`gh issue view $issue --comments\`) and on your pull request, if there is one (\`gh pr view --comments\`, its state and checks), then go on from where you stopped. The rules from the start of this session still apply.
EOF
}

session_state() {
    if ! tmux has-session -t "$SESSION" 2>/dev/null; then
        echo none
    elif [[ "$(tmux list-panes -t "$SESSION" -F '#{pane_dead}')" == 1 ]]; then
        echo stopped
    elif [[ -e "$STATE_DIR/idle" ]] ||
        (($(date +%s) - $(tmux display-message -p -t "$SESSION" '#{window_activity}') > STALE_AFTER)); then
        echo idle
    else
        echo busy
    fi
}

start_session() {
    local issue="$1" prompt="$2"
    shift 2
    # The pane stays after Claude Code exits (tmux.conf). A session that is
    # waiting for input after a finished turn is replaced too; only a busy one blocks.
    case "$(session_state)" in
        stopped | idle) tmux kill-session -t "$SESSION" ;;
        busy) die "Claude Code is busy in tmux session '$SESSION' (attach, nudge or stop it first)" ;;
    esac
    mark_turn
    # Remote Control shows the session on claude.ai/code and in the Claude app.
    # The settings come by flag, not as managed settings in /etc/claude-code:
    # an organisation's server-managed settings replace that file completely.
    tmux new-session -d -s "$SESSION" -x 200 -y 50 -c "$REPO_DIR" \
        claude --dangerously-skip-permissions --settings "$AGENT_SETTINGS" \
        --remote-control "$HOSTNAME #$issue" "$@" "$prompt"
    echo "kestic-agent: Claude Code runs in tmux session '$SESSION'"
}

mark_turn() {
    mkdir -p "$STATE_DIR"
    date +%s > "$STATE_DIR/turn"
    rm -f "$STATE_DIR/idle"
}

case "${1:-}" in
    setup)
        setup
        ;;
    login)
        setup
        cd "$REPO_DIR"
        exec claude --dangerously-skip-permissions --settings "$AGENT_SETTINGS"
        ;;
    run)
        issue="$(issue_number "${2:-}" run)"
        base="$(base_branch "${3:-}")"
        setup
        start_session "$issue" "$(start_prompt "$issue" "$base")"
        printf '%s\n' "$issue" > "$STATE_DIR/issue"
        ;;
    resume)
        issue="$(issue_number "${2:-}" resume)"
        start_session "$issue" "$(resume_prompt "$issue")" --continue
        ;;
    nudge)
        shift
        [[ $# -gt 0 ]] || die "usage: kestic-agent nudge <text…>"
        tmux has-session -t "$SESSION" 2>/dev/null || die "no running session '$SESSION'"
        tmux send-keys -t "$SESSION" -l -- "$*"
        tmux send-keys -t "$SESSION" Enter
        ;;
    status)
        tmux has-session -t "$SESSION" 2>/dev/null || die "no running session '$SESSION'"
        tmux capture-pane -p -t "$SESSION" -S "-${2:-40}"
        ;;
    state)
        # stderr first: a missing file (first run) is not an error.
        issue="$(tr -dc 0-9 2>/dev/null < "$STATE_DIR/issue" || true)"
        turn="$(tr -dc 0-9 2>/dev/null < "$STATE_DIR/turn" || true)"
        echo "$(session_state) ${issue:-0} ${turn:-0}"
        ;;
    limit-reset)
        # When Claude Code stopped at a usage limit, its last lines say
        # «… hit your … limit · resets 4:10am (UTC)». Prints that time as epoch
        # (the next one after the turn started), or 0. A limit resets within
        # five hours, so a time further ahead comes from an older message.
        turn="$(tr -dc 0-9 2>/dev/null < "$STATE_DIR/turn" || true)"
        at="$(tmux capture-pane -p -t "$SESSION" 2>/dev/null | grep -v '^[[:space:]]*$' | tail -n 15 |
            grep 'hit your' | grep -o 'resets [0-9][0-9:]*[ap]m (UTC)' | tail -n 1 || true)"
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
        ;;
    store-token)
        IFS= read -r token || true
        [[ "$token" =~ ^gh[a-z]_[A-Za-z0-9_.-]+$ ]] || die "store-token: no GitHub token on stdin"
        rm -f "$TOKEN_FILE.new"
        (
            umask 077
            printf '%s\n' "$token" > "$TOKEN_FILE.new"
        )
        chmod 0400 "$TOKEN_FILE.new"
        mv -f "$TOKEN_FILE.new" "$TOKEN_FILE"
        ;;
    show-token)
        cat "$TOKEN_FILE"
        ;;
    token-age)
        if [[ -e "$TOKEN_FILE" ]]; then
            echo $(($(date +%s) - $(stat -c %Y "$TOKEN_FILE")))
        else
            echo none
        fi
        ;;
    hook)
        case "${2:-}" in
            turn) mark_turn ;;
            idle) mkdir -p "$STATE_DIR" && touch "$STATE_DIR/idle" ;;
            *) die "usage: kestic-agent hook turn|idle" ;;
        esac
        ;;
    *)
        sed -n '5,25p' "$0" | sed 's/^# \{0,1\}//'
        exit 2
        ;;
esac
