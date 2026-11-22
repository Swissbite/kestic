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
#   kestic-agent.sh agent-state        `<state> <issue> <turn start>` for kestic-agent-watch.sh;
#                                      state off when the container is not running
#   kestic-agent.sh attach             attach to the agent's tmux session (detach: Ctrl-b d)
#   kestic-agent.sh exec <command…>    run a command as kesticci in the clone (e.g. just verify)
#   kestic-agent.sh shell              interactive shell as kesticci
#   kestic-agent.sh dns-log [lines]    DNS queries of the container, allowed and refused
#   kestic-agent.sh stop               stop and remove the container; volumes stay
#   kestic-agent.sh clean --yes        remove container, image and both volumes (also the login)
#
# Root inside the container only ever runs commands by absolute path or with a
# fixed PATH, because the home volume (first in the image PATH) is writable by
# the agent.
#
# Nothing from the host reaches the container except the image, the Podman
# secret with the GitHub token and two named volumes: no bind mount, no host
# D-Bus, keyring, SSH agent or home directory.

set -euo pipefail

cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."

IMAGE=localhost/kestic-agent:latest
NAME=kestic-agent
SECRET="${KESTIC_AGENT_SECRET:-kestic-agent-gh-token}"
VOL_HOME=kestic-agent-home
VOL_CLAUDE=kestic-agent-claude
DROP=(/usr/local/lib/kestic-agent/entrypoint.sh --drop)

die() {
    echo "kestic-agent.sh: $*" >&2
    exit 1
}

running() {
    [[ "$(podman container inspect --format '{{.State.Running}}' "$NAME" 2>/dev/null)" == true ]]
}

ensure_running() {
    running && return 0
    podman image exists "$IMAGE" || die "no image $IMAGE; run: $0 build"
    podman secret inspect "$SECRET" >/dev/null 2>&1 \
        || die "no Podman secret $SECRET; see .devcontainer/README.md"
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
        --secret "$SECRET,type=mount,target=kestic-agent-gh-token,uid=1001,gid=1001,mode=0400" \
        --volume "$VOL_HOME:/home/kesticci" \
        --volume "$VOL_CLAUDE:/home/kesticci/.claude" \
        "$IMAGE" >/dev/null
    for _ in $(seq 60); do
        running && podman exec "$NAME" /usr/bin/test -e /run/kestic-agent/ready && return 0
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
            podman exec "$NAME" "${DROP[@]}" kestic-agent state
        else
            echo "off 0 0"
        fi
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
        echo "removed; the secret $SECRET stays (podman secret rm $SECRET)"
        ;;
    *)
        sed -n '5,21p' "$0" | sed 's/^# \{0,1\}//'
        exit 2
        ;;
esac
