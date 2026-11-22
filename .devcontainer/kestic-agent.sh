#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause
#
# Runs the Kestic agent container with rootless Podman (no devcontainer CLI needed).
#
#   kestic-agent.sh build              build the image on the CI image of this checkout
#   kestic-agent.sh login              one-time interactive Claude Code login
#   kestic-agent.sh start <issue> [<base>]  start the container and the agent on a GitHub
#                                      issue; branch and pull request on <base> (default main)
#   kestic-agent.sh resume <issue>     the agent continues its last session (after stop)
#   kestic-agent.sh nudge <text…>      type a message into the agent's Claude session
#   kestic-agent.sh status [lines]     container state and the agent's last terminal lines
#   kestic-agent.sh agent-state        `<state> <issue> <turn start> <ok|failed>` for
#                                      kestic-agent-watch.sh; state off when the container is
#                                      not running; renews the GitHub token when it is due
#   kestic-agent.sh token              a new GitHub App installation token for the agent now
#   kestic-agent.sh attach             attach to the agent's tmux session (detach: Ctrl-b d)
#   kestic-agent.sh exec <command…>    run a command as kesticci in the clone (e.g. just verify)
#   kestic-agent.sh shell              interactive shell as kesticci
#   kestic-agent.sh dns-log [lines]    DNS queries of the container, allowed and refused
#   kestic-agent.sh stop               revoke the GitHub token, stop and remove the container;
#                                      volumes stay
#   kestic-agent.sh clean --yes        remove container, image and both volumes (also the login)
#
# Root inside the container only ever runs commands by absolute path or with a
# fixed PATH, because the home volume (first in the image PATH) is writable by
# the agent.
#
# KESTIC_AGENT_NAME (default kestic-agent) names the container and its two
# volumes <name>-home and <name>-claude; every command acts on that instance.
#
# Nothing from the host reaches the container except the image, a GitHub App
# installation token (written through stdin, renewed every 40 minutes; the
# App's private key stays on the host), the git author from agent.conf and two
# named volumes: no bind mount, no host D-Bus, keyring, SSH agent or home
# directory.

set -euo pipefail

cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."

IMAGE=localhost/kestic-agent:latest
NAME="${KESTIC_AGENT_NAME:-kestic-agent}"
APP_KEY_SECRET="${KESTIC_AGENT_APP_KEY_SECRET:-kestic-agent-app-key}"
REPO="${KESTIC_AGENT_REPO:-Swissbite/kestic}"
CONF=.devcontainer/agent.conf
# Installation tokens live 60 minutes; renew well before.
TOKEN_MAX_AGE=2400
# Each instance (KESTIC_AGENT_NAME) has its own clone and its own Claude login,
# so several agents can work on different milestones side by side.
VOL_HOME="$NAME-home"
VOL_CLAUDE="$NAME-claude"
DROP=(/usr/local/lib/kestic-agent/entrypoint.sh --drop)

die() {
    echo "kestic-agent.sh: $*" >&2
    exit 1
}

running() {
    [[ "$(podman container inspect --format '{{.State.Running}}' "$NAME" 2>/dev/null)" == true ]]
}

# conf <key>: KESTIC_AGENT_<KEY> from the environment, else <key>= from agent.conf.
conf() {
    local env="KESTIC_AGENT_${1^^}"
    if [[ -n "${!env:-}" ]]; then
        printf '%s' "${!env}"
    else
        sed -n "s/^$1=//p" "$CONF" | tail -n 1
    fi
}

base64url() {
    openssl base64 -A | tr '+/' '-_' | tr -d '='
}

# A JSON Web Token for the GitHub App, valid for 9 minutes (GitHub allows 10).
app_jwt() {
    local app_id="$1" key="$2" now unsigned signature
    now="$(date +%s)"
    unsigned="$(printf '{"typ":"JWT","alg":"RS256"}' | base64url).$(printf '{"iat":%d,"exp":%d,"iss":"%s"}' \
        $((now - 60)) $((now + 540)) "$app_id" | base64url)"
    signature="$(printf '%s' "$unsigned" | openssl dgst -sha256 -binary -sign <(printf '%s\n' "$key") | base64url)"
    printf '%s.%s' "$unsigned" "$signature"
}

# github_api <method> <path> <bearer> [<json body>]: the response body. The
# bearer goes to curl through a file descriptor, never through argv.
github_api() {
    local args=(--silent --show-error --fail-with-body --max-time 30 -X "$1"
        -H 'Accept: application/vnd.github+json' -H 'X-GitHub-Api-Version: 2022-11-28')
    [[ $# -ge 4 ]] && args+=(--data "$4")
    curl "${args[@]}" -H @<(printf 'Authorization: Bearer %s\n' "$3") "https://api.github.com$2"
}

json_field() {
    python3 -I -c 'import json, sys; print(json.load(sys.stdin)[sys.argv[1]])' "$1"
}

# Asks GitHub for an installation token limited to this repository and hands it
# to the agent through stdin.
renew_token() {
    local app_id key jwt installation token
    app_id="$(conf app_id)"
    [[ "$app_id" =~ ^[0-9]+$ ]] || die "no GitHub App ID: set app_id in $CONF"
    key="$(podman secret inspect --showsecret --format '{{.SecretData}}' "$APP_KEY_SECRET")" \
        || die "no Podman secret $APP_KEY_SECRET with the App's private key; see .devcontainer/README.md"
    jwt="$(app_jwt "$app_id" "$key")"
    installation="$(github_api GET "/repos/$REPO/installation" "$jwt" | json_field id)" \
        || die "the GitHub App is not installed on $REPO or the key does not match"
    [[ "$installation" =~ ^[0-9]+$ ]] || die "unexpected installation id from GitHub"
    token="$(github_api POST "/app/installations/$installation/access_tokens" "$jwt" \
        "{\"repositories\":[\"${REPO#*/}\"]}" | json_field token)" || die "GitHub refused an installation token"
    printf '%s\n' "$token" | podman exec -i "$NAME" "${DROP[@]}" kestic-agent store-token
}

ensure_token() {
    local age
    age="$(podman exec "$NAME" "${DROP[@]}" kestic-agent token-age)"
    if [[ ! "$age" =~ ^[0-9]+$ ]] || ((age > TOKEN_MAX_AGE)); then
        renew_token
    fi
}

ensure_running() {
    if running; then
        ensure_token
        return 0
    fi
    podman image exists "$IMAGE" || die "no image $IMAGE; run: $0 build"
    podman secret inspect "$APP_KEY_SECRET" >/dev/null 2>&1 \
        || die "no Podman secret $APP_KEY_SECRET; see .devcontainer/README.md"
    [[ "$(conf app_id)" =~ ^[0-9]+$ ]] || die "no GitHub App ID: set app_id in $CONF"
    local git_name git_email
    git_name="$(conf git_name)"
    git_email="$(conf git_email)"
    [[ -n "$git_name" && -n "$git_email" ]] || die "git_name and git_email missing in $CONF"
    podman rm --ignore "$NAME" >/dev/null
    # A low CPU weight keeps the desktop responsive without slowing the agent
    # down when the host is idle. KESTIC_AGENT_CPUS additionally caps the cores;
    # cargo and nextest then use that many jobs.
    local limits=()
    if [[ -n "${KESTIC_AGENT_CPUS:-}" ]]; then
        limits=(--cpus="$KESTIC_AGENT_CPUS"
            --env CARGO_BUILD_JOBS="$KESTIC_AGENT_CPUS" --env NEXTEST_TEST_THREADS="$KESTIC_AGENT_CPUS")
    fi
    # Capabilities are only for the root entrypoint (firewall, dnsmasq, dropping
    # to kesticci); the agent itself runs without any.
    podman run -d --name "$NAME" --hostname "$NAME" --init \
        --network=slirp4netns:allow_host_loopback=false,enable_ipv6=false \
        --cap-drop=all --cap-add=NET_ADMIN,NET_BIND_SERVICE,SETUID,SETGID,SETPCAP,KILL \
        --security-opt=no-new-privileges \
        --cpu-shares=256 --env KESTIC_AGENT_NICE="${KESTIC_AGENT_NICE:-0}" "${limits[@]}" \
        --memory="${KESTIC_AGENT_MEMORY:-24g}" --pids-limit=16384 \
        --tmpfs /var/log/kestic-agent:size=16m,mode=0755 \
        --tmpfs /run/kestic-agent-token:size=64k,mode=1777 \
        --env KESTIC_AGENT_GIT_NAME="$git_name" --env KESTIC_AGENT_GIT_EMAIL="$git_email" \
        --volume "$VOL_HOME:/home/kesticci" \
        --volume "$VOL_CLAUDE:/home/kesticci/.claude" \
        "$IMAGE" >/dev/null
    for _ in $(seq 60); do
        if running && podman exec "$NAME" /usr/bin/test -e /run/kestic-agent/ready; then
            renew_token
            return 0
        fi
        running || break
        sleep 1
    done
    podman logs "$NAME" >&2 || true
    die "container did not come up (firewall setup failed?)"
}

cmd="${1:-}"
[[ $# -gt 0 ]] && shift
case "$cmd" in
    build)
        tag="$(.github/ci-image/tag.sh)"
        default_tag="$(sed -n 's/^ARG CI_IMAGE=ghcr.io\/swissbite\/kestic-ci://p' .devcontainer/Containerfile)"
        if [[ "$tag" != "$default_tag" ]]; then
            echo "kestic-agent.sh: note: Containerfile default CI image tag $default_tag differs from $tag (used here)" >&2
        fi
        podman build --pull=newer \
            --build-arg "CI_IMAGE=ghcr.io/swissbite/kestic-ci:$tag" \
            --build-arg "CLAUDE_CODE_VERSION=${CLAUDE_CODE_VERSION:-stable}" \
            --file .devcontainer/Containerfile --tag "$IMAGE" .devcontainer
        ;;
    login)
        ensure_running
        podman exec -it "$NAME" "${DROP[@]}" kestic-agent login
        ;;
    start)
        [[ "${1:-}" =~ ^[0-9]+$ ]] || die "usage: $0 start <issue number> [<base branch>]"
        ensure_running
        podman exec "$NAME" "${DROP[@]}" kestic-agent run "$1" "${2:-main}"
        ;;
    resume)
        [[ "${1:-}" =~ ^[0-9]+$ ]] || die "usage: $0 resume <issue number>"
        ensure_running
        podman exec "$NAME" "${DROP[@]}" kestic-agent setup
        podman exec "$NAME" "${DROP[@]}" kestic-agent resume "$1"
        ;;
    nudge)
        running || die "container is not running"
        podman exec "$NAME" "${DROP[@]}" kestic-agent nudge "$@"
        ;;
    status)
        podman ps -a --filter "name=^$NAME\$" --format '{{.Names}}  {{.Status}}  {{.Image}}'
        if running; then
            podman stats --no-stream --format '{{.Name}}  CPU {{.CPUPerc}}  MEM {{.MemUsage}}  PIDs {{.PIDs}}' "$NAME"
            podman exec "$NAME" "${DROP[@]}" kestic-agent status "${1:-40}" || true
        fi
        ;;
    agent-state)
        if running; then
            # A failed renewal must not hide the state line (die exits only the subshell).
            token_status=ok
            (ensure_token) >&2 || token_status=failed
            echo "$(podman exec "$NAME" "${DROP[@]}" kestic-agent state) $token_status"
        else
            echo "off 0 0 ok"
        fi
        ;;
    token)
        running || die "container is not running"
        renew_token
        ;;
    attach)
        running || die "container is not running"
        podman exec -it "$NAME" "${DROP[@]}" tmux attach -t agent
        ;;
    exec)
        [[ $# -gt 0 ]] || die "usage: $0 exec <command…>"
        ensure_running
        podman exec "$NAME" "${DROP[@]}" bash -c 'cd ~/kestic 2>/dev/null || cd ~; exec "$@"' bash "$@"
        ;;
    shell)
        ensure_running
        podman exec -it "$NAME" "${DROP[@]}" bash -c 'cd ~ && exec bash'
        ;;
    dns-log)
        running || die "container is not running"
        # The log holds names the agent chose; cat -v keeps escape sequences out of the terminal.
        podman exec "$NAME" /usr/bin/tail -n "${1:-50}" /var/log/kestic-agent/dns.log | cat -v
        ;;
    stop)
        if running; then
            # The token would stay valid for up to an hour; revoke it with itself.
            token="$(podman exec "$NAME" "${DROP[@]}" kestic-agent show-token 2>/dev/null || true)"
            if [[ -n "$token" ]]; then
                github_api DELETE /installation/token "$token" >/dev/null ||
                    echo "kestic-agent.sh: revoking the GitHub token failed" >&2
            fi
        fi
        podman stop --ignore --time 20 "$NAME" >/dev/null
        podman rm --ignore "$NAME" >/dev/null
        echo "stopped; volumes $VOL_HOME and $VOL_CLAUDE are kept"
        ;;
    clean)
        [[ "${1:-}" == --yes ]] || die "removes container, image and volumes incl. the Claude login; confirm with: $0 clean --yes"
        podman rm --force --ignore "$NAME" >/dev/null
        for volume in "$VOL_HOME" "$VOL_CLAUDE"; do
            if podman volume exists "$volume"; then
                podman volume rm --force "$volume" >/dev/null
            fi
        done
        podman rmi --ignore "$IMAGE" >/dev/null
        echo "removed; the secret $APP_KEY_SECRET stays (podman secret rm $APP_KEY_SECRET)"
        ;;
    *)
        sed -n '5,24p' "$0" | sed 's/^# \{0,1\}//'
        exit 2
        ;;
esac
