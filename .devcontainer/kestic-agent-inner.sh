#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause
#
# `kestic-agent` inside the agent container, run as kesticci.
#
#   kestic-agent setup              check the token, clone or update the repository,
#                                   git identity and hooks, bootstrap.sh (toolchain, restic)
#   kestic-agent login              setup, then Claude Code interactively (one-time /login)
#   kestic-agent run <issue>        setup, then Claude Code in tmux session `agent` on the issue
#   kestic-agent resume <issue>     Claude Code continues its last session (after a stop)
#   kestic-agent nudge <text…>      types <text> into the running Claude session
#   kestic-agent status [lines]     the last lines of the agent's terminal
#
# The GitHub issue is the agent's whole assignment; progress goes back as issue
# comments and a pull request.

set -euo pipefail

REPO_SLUG="${KESTIC_AGENT_REPO:-Swissbite/kestic}"
REPO_DIR="$HOME/kestic"
TOKEN_FILE=/run/secrets/kestic-agent-gh-token
SESSION=agent

die() {
    echo "kestic-agent: $*" >&2
    exit 1
}

setup() {
    [[ -r "$TOKEN_FILE" ]] || die "no GitHub token at $TOKEN_FILE (Podman secret kestic-agent-gh-token)"

    local login user_id name email
    login="$(gh api user --jq .login)" || die "the GitHub token is not accepted"
    user_id="$(gh api user --jq .id)"
    name="${KESTIC_AGENT_GIT_NAME:-$(gh api user --jq '.name // .login')}"
    email="${KESTIC_AGENT_GIT_EMAIL:-${user_id}+${login}@users.noreply.github.com}"

    git config --global user.name "$name"
    git config --global user.email "$email"
    git config --global core.hooksPath /usr/local/lib/kestic-agent/git-hooks
    git config --global init.defaultBranch main
    # The token is read from the secret for each git operation and never stored.
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
    echo "kestic-agent: ready as $login in $REPO_DIR"
}

issue_number() {
    [[ "${1:-}" =~ ^[0-9]+$ ]] || die "usage: kestic-agent $2 <issue number>"
    printf '%s' "$1"
}

start_prompt() {
    local issue="$1"
    cat <<EOF
You are the Kestic coding agent, working autonomously in an isolated container. Your assignment is GitHub issue #$issue in $REPO_SLUG.

1. Read AGENTS.md, then the issue with \`gh issue view $issue --comments\`. The issue is your specification; comments by the maintainer refine it.
2. Post a short comment on the issue that you have started (\`gh issue comment $issue\`).
3. Create the branch \`agent/issue-$issue\` from origin/main and implement the issue as AGENTS.md describes (spec, one test per acceptance criterion, i18n, REUSE).
4. Run \`just verify\` until it is green. Then let two review subagents check your diff (one for correctness, one for tests and conformity with the spec and AGENTS.md), fix what they find and run \`just verify\` again.
5. Push the branch and open a pull request against main with \`gh pr create\`. The body explains the change and how it was verified and ends with \`Closes #$issue\`. Comment the pull request link on the issue.
6. If you are blocked or a decision belongs to the maintainer (AGENTS.md, "Stop & Ask"), also post the question as an issue comment, then stop.

CI logs: \`gh pr checks\` lists the jobs; read a job's log with \`gh api repos/$REPO_SLUG/actions/jobs/<job id>/logs\` (\`gh run view --log\` prints nothing with the gh version in this container).

Rules: never merge a pull request, never push to main, never change .github/workflows. Commits, pull requests and comments carry no AI attribution: no Co-Authored-By trailer, no "Generated with" line. Keep comments short, factual and in English.
EOF
}

resume_prompt() {
    local issue="$1"
    cat <<EOF
Continue with GitHub issue #$issue. First read new comments on the issue (\`gh issue view $issue --comments\`) and on your pull request, if there is one (\`gh pr view --comments\`), then go on from where you stopped. The rules from the start of this session still apply.
EOF
}

start_session() {
    local prompt="$1"
    shift
    if tmux has-session -t "$SESSION" 2>/dev/null; then
        # The pane stays after Claude Code exits (tmux.conf); only a live one blocks.
        if [[ "$(tmux list-panes -t "$SESSION" -F '#{pane_dead}')" == 1 ]]; then
            tmux kill-session -t "$SESSION"
        else
            die "Claude Code is still running in tmux session '$SESSION' (attach, nudge or stop it first)"
        fi
    fi
    tmux new-session -d -s "$SESSION" -x 200 -y 50 -c "$REPO_DIR" \
        claude --dangerously-skip-permissions "$@" "$prompt"
    echo "kestic-agent: Claude Code runs in tmux session '$SESSION'"
}

case "${1:-}" in
    setup)
        setup
        ;;
    login)
        setup
        cd "$REPO_DIR"
        exec claude --dangerously-skip-permissions
        ;;
    run)
        issue="$(issue_number "${2:-}" run)"
        setup
        start_session "$(start_prompt "$issue")"
        ;;
    resume)
        issue="$(issue_number "${2:-}" resume)"
        start_session "$(resume_prompt "$issue")" --continue
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
    *)
        sed -n '5,14p' "$0" | sed 's/^# \{0,1\}//'
        exit 2
        ;;
esac
