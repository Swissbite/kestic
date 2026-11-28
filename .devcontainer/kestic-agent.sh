#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause
#
# Runs the Kestic agent container with rootless Podman (no devcontainer CLI needed).
#
#   kestic-agent.sh backends           list available agent variations (agents/<name>/)
#   kestic-agent.sh build              build the image for KESTIC_AGENT_BACKEND (default claude)
#   kestic-agent.sh login              one-time interactive agent login in the container
#   kestic-agent.sh start <issue> [<base>]  start the container and the agent on a GitHub
#                                      issue; branch and pull request on <base> (default main)
#   kestic-agent.sh resume <issue>     the agent continues its last session (after stop)
#   kestic-agent.sh nudge <text…>      type a message into the agent's session
#   kestic-agent.sh status [lines]     container state and the agent's last terminal lines
#   kestic-agent.sh agent-state        `<state> <issue> <turn start> <ok|failed> <limit reset>`
#                                      for kestic-agent-watch.sh; state off when the container
#                                      is not running; renews the GitHub token when it is due
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
# volumes <name>-home and <name>-<volume_suffix>; every command acts on that
# instance. KESTIC_AGENT_BACKEND (default claude, or `backend=` in agent.conf)
# selects the agent variation under KESTIC_AGENT_BACKENDS_DIR (default
# .devcontainer/agents). Optional runtime overrides: KESTIC_AGENT_MODEL,
# KESTIC_AGENT_EFFORT, KESTIC_AGENT_REMOTE_CONTROL (0 to disable).
#
# Nothing from the host reaches the container except the image, a GitHub App
# installation token (written through stdin, renewed every 40 minutes; the
# App's private key stays on the host), the git author from agent.conf and two
# named volumes: no bind mount, no host D-Bus, keyring, SSH agent or home
# directory.

set -euo pipefail

# Resolve a relative KESTIC_AGENT_BACKENDS_DIR against the caller's cwd before
# switching to the repository root.
CALLER_PWD="$PWD"
if [[ -n "${KESTIC_AGENT_BACKENDS_DIR:-}" && "${KESTIC_AGENT_BACKENDS_DIR}" != /* ]]; then
    BACKENDS_DIR="$(realpath -m "$CALLER_PWD/$KESTIC_AGENT_BACKENDS_DIR")"
else
    BACKENDS_DIR="${KESTIC_AGENT_BACKENDS_DIR:-}"
fi

cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
BACKENDS_DIR="${BACKENDS_DIR:-.devcontainer/agents}"

NAME="${KESTIC_AGENT_NAME:-kestic-agent}"
APP_KEY_SECRET="${KESTIC_AGENT_APP_KEY_SECRET:-kestic-agent-app-key}"
REPO="${KESTIC_AGENT_REPO:-Swissbite/kestic}"
CONF=.devcontainer/agent.conf
# Installation tokens live 60 minutes; renew well before.
TOKEN_MAX_AGE=2400
DROP=(/usr/local/lib/kestic-agent/entrypoint.sh --drop)

die() {
    echo "kestic-agent.sh: $*" >&2
    exit 1
}

[[ "$NAME" =~ ^[A-Za-z0-9][A-Za-z0-9._-]*$ ]] || die "invalid container name: $NAME"
VOL_HOME="$NAME-home"

running() {
    [[ "$(podman container inspect --format '{{.State.Running}}' "$NAME" 2>/dev/null)" == true ]]
}

container_label() {
    podman container inspect --format "{{index .Config.Labels \"$1\"}}" "$NAME" 2>/dev/null || true
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

# backend_conf <file> <key>: reads <key>=value from a backend.conf (never sourced).
backend_conf() {
    sed -n "s/^$2=//p" "$1" | tail -n 1
}

BACKEND="$(conf backend)"
BACKEND="${BACKEND:-claude}"
[[ "$BACKEND" =~ ^[a-z0-9][a-z0-9._-]*$ ]] || die "invalid backend name: $BACKEND"
BACKEND_DIR="$BACKENDS_DIR/$BACKEND"
IMAGE="localhost/kestic-agent:$BACKEND"
VOL_SUFFIX=""
VOL_MOUNT=""
VOL_AGENT=""
VERSION_ARG=""

load_backend() {
    local bconf="$BACKEND_DIR/backend.conf"
    [[ -d "$BACKEND_DIR" ]] || die "unknown backend '$BACKEND' (not found in $BACKENDS_DIR; run: $0 backends)"
    for req in backend.conf install.sh driver.sh allowed-domains.txt; do
        [[ -r "$BACKEND_DIR/$req" ]] || die "backend '$BACKEND' is missing $req in $BACKEND_DIR"
    done
    VOL_SUFFIX="$(backend_conf "$bconf" volume_suffix)"
    VOL_MOUNT="$(backend_conf "$bconf" volume_mount)"
    VERSION_ARG="$(backend_conf "$bconf" version_arg)"
    [[ "$VOL_SUFFIX" =~ ^[a-z0-9][a-z0-9._-]*$ ]] || die "backend '$BACKEND': invalid volume_suffix '$VOL_SUFFIX'"
    [[ "$VOL_MOUNT" =~ ^/home/kesticci/[A-Za-z0-9._-]+(/[A-Za-z0-9._-]+)*$ ]] \
        && [[ "$VOL_MOUNT" != */. && "$VOL_MOUNT" != */.. && "$VOL_MOUNT" != */./* && "$VOL_MOUNT" != */../* ]] \
        && [[ "$VOL_MOUNT" != /home/kesticci/kestic && "$VOL_MOUNT" != /home/kesticci/kestic/* ]] \
        && [[ "$VOL_MOUNT" != /home/kesticci/.kestic-agent && "$VOL_MOUNT" != /home/kesticci/.kestic-agent/* ]] \
        || die "backend '$BACKEND': invalid volume_mount '$VOL_MOUNT'"
    [[ -z "$VERSION_ARG" || "$VERSION_ARG" =~ ^[A-Z_][A-Z0-9_]*$ ]] \
        || die "backend '$BACKEND': invalid version_arg '$VERSION_ARG'"
    VOL_AGENT="$NAME-$VOL_SUFFIX"
}

# Optional runtime settings forwarded into the container on run/exec.
exec_env_args() {
    EXEC_ENV_ARGS=()
    if [[ -n "${KESTIC_AGENT_MODEL:-}" ]]; then
        EXEC_ENV_ARGS+=(--env "KESTIC_AGENT_MODEL=$KESTIC_AGENT_MODEL")
    fi
    if [[ -n "${KESTIC_AGENT_EFFORT:-}" ]]; then
        EXEC_ENV_ARGS+=(--env "KESTIC_AGENT_EFFORT=$KESTIC_AGENT_EFFORT")
    fi
    if [[ -n "${KESTIC_AGENT_REMOTE_CONTROL:-}" ]]; then
        EXEC_ENV_ARGS+=(--env "KESTIC_AGENT_REMOTE_CONTROL=$KESTIC_AGENT_REMOTE_CONTROL")
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
        local running_backend
        running_backend="$(container_label kestic.agent.backend)"
        if [[ -n "$running_backend" && "$running_backend" != "$BACKEND" ]]; then
            die "container $NAME is running backend '$running_backend', not '$BACKEND' (stop it first or set KESTIC_AGENT_BACKEND=$running_backend)"
        fi
        if [[ -z "$running_backend" && "$BACKEND" != claude ]]; then
            die "container $NAME has no backend label (legacy claude container); stop it first before running '$BACKEND'"
        fi
        ensure_token
        return 0
    fi
    load_backend
    local run_image="$IMAGE"
    if [[ "$BACKEND" == claude ]] && ! podman image exists "$run_image" && podman image exists localhost/kestic-agent:latest; then
        run_image=localhost/kestic-agent:latest
    fi
    podman image exists "$run_image" || die "no image $IMAGE; run: KESTIC_AGENT_BACKEND=$BACKEND $0 build"
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
    exec_env_args
    # Capabilities are only for the root entrypoint (firewall, dnsmasq, dropping
    # to kestic-infra and kesticci); the agent itself runs without any.
    # on-failure: a container that ends by itself (a crash, a killed main
    # process) comes back, five times at most; `stop` ends it for good. The
    # watcher then continues the agent's session.
    podman run -d --name "$NAME" --hostname "$NAME" --init --restart=on-failure:5 \
        --label "kestic.agent.backend=$BACKEND" \
        --label "kestic.agent.volume=$VOL_AGENT" \
        --network=slirp4netns:allow_host_loopback=false,enable_ipv6=false \
        --cap-drop=all --cap-add=NET_ADMIN,NET_BIND_SERVICE,SETUID,SETGID,SETPCAP,KILL \
        --security-opt=no-new-privileges \
        --cpu-shares=256 --env KESTIC_AGENT_NICE="${KESTIC_AGENT_NICE:-0}" "${limits[@]}" \
        --memory="${KESTIC_AGENT_MEMORY:-24g}" --pids-limit=16384 \
        --tmpfs /var/log/kestic-agent:size=16m,mode=0755 \
        --tmpfs /run/kestic-agent-token:size=64k,mode=1777 \
        --env KESTIC_AGENT_GIT_NAME="$git_name" --env KESTIC_AGENT_GIT_EMAIL="$git_email" \
        "${EXEC_ENV_ARGS[@]}" \
        --volume "$VOL_HOME:/home/kesticci" \
        --volume "$VOL_AGENT:$VOL_MOUNT" \
        "$run_image" >/dev/null
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
    backends)
        [[ -d "$BACKENDS_DIR" ]] || die "no backends directory at $BACKENDS_DIR"
        for dir in "$BACKENDS_DIR"/*/; do
            [[ -f "${dir}backend.conf" ]] || continue
            bname="$(basename "$dir")"
            bdesc="$(backend_conf "${dir}backend.conf" description)"
            bvol="$(backend_conf "${dir}backend.conf" volume_suffix)"
            bimg="localhost/kestic-agent:$bname"
            bstate="not built"
            if podman image exists "$bimg" 2>/dev/null; then
                bstate="built"
            fi
            printf '%-14s  %-10s  volume=-%-10s  %s\n' "$bname" "$bstate" "$bvol" "$bdesc"
        done
        ;;
    build)
        load_backend
        tag="$(.github/ci-image/tag.sh)"
        default_tag="$(sed -n 's/^ARG CI_IMAGE=ghcr.io\/swissbite\/kestic-ci://p' .devcontainer/Containerfile)"
        if [[ "$tag" != "$default_tag" ]]; then
            echo "kestic-agent.sh: note: Containerfile default CI image tag $default_tag differs from $tag (used here)" >&2
        fi
        build_args=(
            --build-arg "CI_IMAGE=ghcr.io/swissbite/kestic-ci:$tag"
            --build-arg "AGENT_BACKEND=$BACKEND"
        )
        # Map any backend-specific version env var (version_arg in backend.conf)
        # onto the generic AGENT_VERSION build-arg so third-party backends never
        # need to declare a new ARG in Containerfile.
        effective_version="${AGENT_VERSION:-}"
        if [[ -z "$effective_version" && -n "$VERSION_ARG" && -n "${!VERSION_ARG:-}" ]]; then
            effective_version="${!VERSION_ARG}"
        fi
        if [[ -n "$effective_version" ]]; then
            build_args+=(--build-arg "AGENT_VERSION=$effective_version")
        fi
        context=.devcontainer
        stage=""
        if [[ "$(realpath -m "$BACKENDS_DIR")" != "$(realpath -m .devcontainer/agents)" ]]; then
            stage="$(mktemp -d)"
            trap 'rm -rf "$stage"' EXIT
            cp -a .devcontainer/. "$stage/"
            rm -rf "$stage/agents/$BACKEND"
            mkdir -p "$stage/agents/$BACKEND"
            # Dereference symlinks so out-of-tree backends with symlinked files build cleanly.
            cp -RL "$BACKEND_DIR/." "$stage/agents/$BACKEND/"
            context="$stage"
        fi
        podman build --pull=newer "${build_args[@]}" \
            --file "$context/Containerfile" --tag "$IMAGE" "$context"
        if [[ "$BACKEND" == claude ]]; then
            podman tag "$IMAGE" localhost/kestic-agent:latest
        fi
        ;;
    login)
        ensure_running
        exec_env_args
        podman exec -it "${EXEC_ENV_ARGS[@]}" "$NAME" "${DROP[@]}" kestic-agent login
        ;;
    start)
        [[ "${1:-}" =~ ^[0-9]+$ ]] || die "usage: $0 start <issue number> [<base branch>]"
        ensure_running
        exec_env_args
        podman exec "${EXEC_ENV_ARGS[@]}" "$NAME" "${DROP[@]}" kestic-agent run "$1" "${2:-main}"
        ;;
    resume)
        [[ "${1:-}" =~ ^[0-9]+$ ]] || die "usage: $0 resume <issue number>"
        ensure_running
        exec_env_args
        podman exec "${EXEC_ENV_ARGS[@]}" "$NAME" "${DROP[@]}" kestic-agent setup
        podman exec "${EXEC_ENV_ARGS[@]}" "$NAME" "${DROP[@]}" kestic-agent resume "$1"
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
            reset="$(podman exec "$NAME" "${DROP[@]}" kestic-agent limit-reset 2>/dev/null || echo 0)"
            echo "$(podman exec "$NAME" "${DROP[@]}" kestic-agent state) $token_status ${reset:-0}"
        else
            echo "off 0 0 ok 0"
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
        kept_vol="$(container_label kestic.agent.volume)"
        if running; then
            # The token would stay valid for up to an hour; revoke it with itself.
            token="$(podman exec "$NAME" "${DROP[@]}" kestic-agent show-token 2>/dev/null || true)"
            if [[ -n "$token" ]]; then
                github_api DELETE /installation/token "$token" >/dev/null ||
                    echo "kestic-agent.sh: revoking the GitHub token failed" >&2
            fi
        fi
        if [[ -z "$kept_vol" ]]; then
            load_backend
            kept_vol="$VOL_AGENT"
        fi
        podman stop --ignore --time 20 "$NAME" >/dev/null
        podman rm --ignore "$NAME" >/dev/null
        echo "stopped; volumes $VOL_HOME and $kept_vol are kept"
        ;;
    clean)
        load_backend
        labelled_vol="$(container_label kestic.agent.volume)"
        labelled_backend="$(container_label kestic.agent.backend)"
        target_image="$IMAGE"
        if [[ -n "$labelled_backend" && -z "${KESTIC_AGENT_BACKEND:-}" ]]; then
            target_image="localhost/kestic-agent:$labelled_backend"
        fi
        [[ "${1:-}" == --yes ]] || die "removes container, image ($target_image) and volumes incl. the login; confirm with: $0 clean --yes"
        podman rm --force --ignore "$NAME" >/dev/null
        volumes=("$VOL_HOME" "$VOL_AGENT")
        if [[ -n "$labelled_vol" && "$labelled_vol" != "$VOL_AGENT" ]]; then
            volumes+=("$labelled_vol")
        fi
        for volume in "${volumes[@]}"; do
            if podman volume exists "$volume"; then
                podman volume rm --force "$volume" >/dev/null
            fi
        done
        podman rmi --ignore "$target_image" >/dev/null
        if [[ "$target_image" == localhost/kestic-agent:claude ]]; then
            podman rmi --ignore localhost/kestic-agent:latest >/dev/null
        fi
        echo "removed; the secret $APP_KEY_SECRET stays (podman secret rm $APP_KEY_SECRET)"
        ;;
    *)
        sed -n '5,27p' "$0" | sed 's/^# \{0,1\}//'
        exit 2
        ;;
esac
