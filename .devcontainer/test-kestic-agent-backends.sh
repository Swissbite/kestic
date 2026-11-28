#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause
#
# Tests the pluggable agent backend architecture (.devcontainer/agents/):
# backend discovery, build/run/volume/env wiring for claude, antigravity and
# an out-of-tree custom backend, running-container backend mismatch guard,
# strict validation, driver launch commands, Antigravity hooks, stale-idle
# nudge handling, real init-firewall.sh domain merging/validation, and AI
# attribution rejection in commit-msg.

set -euo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
failures=0

# Isolate all host state from the caller's real environment.
mkdir -p "$tmp/xdg"/{home,cfg,data,cache,state,run}
export HOME="$tmp/xdg/home" \
    XDG_CONFIG_HOME="$tmp/xdg/cfg" \
    XDG_DATA_HOME="$tmp/xdg/data" \
    XDG_CACHE_HOME="$tmp/xdg/cache" \
    XDG_STATE_HOME="$tmp/xdg/state" \
    XDG_RUNTIME_DIR="$tmp/xdg/run"

fail() {
    echo "FAIL: $*" >&2
    failures=$((failures + 1))
}

mkdir -p "$tmp/bin"
openssl genrsa -out "$tmp/key.pem" 2048 2>/dev/null
cat > "$tmp/bin/podman" <<'FAKE'
#!/usr/bin/env bash
{ printf '%s\n' "$@"; echo ---; } >> "$FAKE_DIR/podman.log"
case "$1" in
    image)
        if [[ "$2" == exists ]]; then
            [[ "${FAKE_IMAGE_EXISTS:-true}" == true ]]
            exit $?
        fi ;;
    secret)
        [[ "$2" == inspect && "$3" == --showsecret ]] && cat "$FAKE_DIR/key.pem"
        exit 0 ;;
    container)
        fmt="$4"
        case "$fmt" in
            *State.Running*)
                if [[ -f "$FAKE_DIR/started" ]]; then echo true; else echo false; fi ;;
            *kestic.agent.backend*)
                printf '%s\n' "${FAKE_LABEL_BACKEND:-}" ;;
            *kestic.agent.volume*)
                printf '%s\n' "${FAKE_LABEL_VOLUME:-}" ;;
        esac ;;
    run)
        touch "$FAKE_DIR/started"
        echo "fake-cid" ;;
    exec)
        case "${@: -1}" in
            /run/kestic-agent/ready) exit 0 ;;
            token-age) echo 10 ;;
            store-token) cat >/dev/null ;;
            show-token) echo "ghs_FakeBackendTestToken" ;;
        esac ;;
    volume)
        exit 0 ;;
    build | tag | rm | stop | rmi) exit 0 ;;
esac
FAKE
cat > "$tmp/bin/curl" <<'FAKE'
#!/usr/bin/env bash
case "${@: -1}" in
    */installation) echo '{"id":4711}' ;;
    */access_tokens) echo '{"token":"ghs_FakeBackendTestToken"}' ;;
    */installation/token) ;;
esac
FAKE
chmod +x "$tmp/bin/podman" "$tmp/bin/curl"
export PATH="$tmp/bin:$PATH" FAKE_DIR="$tmp" KESTIC_AGENT_APP_ID=123456

# --- 1. Built-in backends list ------------------------------------------------
list_out="$("$here/kestic-agent.sh" backends)"
grep -q '^claude ' <<< "$list_out" || fail "backends: missing claude"
grep -q '^antigravity ' <<< "$list_out" || fail "backends: missing antigravity"
grep -q 'volume=-gemini' <<< "$list_out" || fail "backends: antigravity missing volume=-gemini"

# --- 2. Build & run wiring for claude and antigravity -------------------------
rm -f "$tmp/podman.log"
KESTIC_AGENT_BACKEND=antigravity AGY_VERSION=1.5.19 "$here/kestic-agent.sh" build >/dev/null \
    || fail "antigravity build failed"
grep -qx 'AGENT_BACKEND=antigravity' "$tmp/podman.log" || fail "antigravity build: AGENT_BACKEND arg missing"
grep -qx 'AGENT_VERSION=1.5.19' "$tmp/podman.log" || fail "antigravity build: AGY_VERSION not mapped to AGENT_VERSION"
grep -qx 'localhost/kestic-agent:antigravity' "$tmp/podman.log" || fail "antigravity build: wrong image tag"
! grep -qx 'localhost/kestic-agent:latest' "$tmp/podman.log" || fail "antigravity build: must not overwrite :latest"

rm -f "$tmp/podman.log"
KESTIC_AGENT_BACKEND=claude "$here/kestic-agent.sh" build >/dev/null || fail "claude build failed"
grep -qx 'AGENT_BACKEND=claude' "$tmp/podman.log" || fail "claude build: AGENT_BACKEND arg missing"
grep -qx 'localhost/kestic-agent:claude' "$tmp/podman.log" || fail "claude build: wrong image tag"
grep -qx 'localhost/kestic-agent:latest' "$tmp/podman.log" || fail "claude build: :latest alias missing"

# Start container with antigravity + model/effort/name overrides
rm -f "$tmp/started" "$tmp/podman.log"
KESTIC_AGENT_NAME=kestic-agent-agy KESTIC_AGENT_BACKEND=antigravity \
    KESTIC_AGENT_MODEL=gemini-4-carbon-high KESTIC_AGENT_EFFORT=high \
    "$here/kestic-agent.sh" start 99 m6 >/dev/null || fail "antigravity start failed"
grep -qx 'kestic-agent-agy-gemini:/home/kesticci/.gemini' "$tmp/podman.log" \
    || fail "antigravity start: wrong agent volume mount"
grep -qx 'kestic.agent.backend=antigravity' "$tmp/podman.log" \
    || fail "antigravity start: missing backend label"
grep -qx 'KESTIC_AGENT_MODEL=gemini-4-carbon-high' "$tmp/podman.log" \
    || fail "antigravity start: KESTIC_AGENT_MODEL not forwarded"
grep -qx 'KESTIC_AGENT_EFFORT=high' "$tmp/podman.log" \
    || fail "antigravity start: KESTIC_AGENT_EFFORT not forwarded"
grep -qx 'localhost/kestic-agent:antigravity' "$tmp/podman.log" \
    || fail "antigravity start: wrong image"

# Running-container backend mismatch must fail clearly
touch "$tmp/started"
if FAKE_LABEL_BACKEND=claude KESTIC_AGENT_BACKEND=antigravity "$here/kestic-agent.sh" start 99 m6 2>/dev/null; then
    fail "backend mismatch: allowed antigravity start on running claude container"
fi
if FAKE_LABEL_BACKEND="" KESTIC_AGENT_BACKEND=antigravity "$here/kestic-agent.sh" start 99 m6 2>/dev/null; then
    fail "backend mismatch: allowed antigravity start on unlabelled legacy container"
fi
FAKE_LABEL_BACKEND=antigravity KESTIC_AGENT_BACKEND=antigravity "$here/kestic-agent.sh" start 99 m6 >/dev/null \
    || fail "backend match: rejected matching running antigravity container"
rm -f "$tmp/started"

# Stop and clean on exited container honour container labels even without KESTIC_AGENT_BACKEND
rm -f "$tmp/podman.log"
stop_out="$(FAKE_LABEL_VOLUME=kestic-agent-2-gemini KESTIC_AGENT_NAME=kestic-agent-2 "$here/kestic-agent.sh" stop)"
grep -q 'kestic-agent-2-gemini' <<< "$stop_out" || fail "stop: ignored volume label on exited container: $stop_out"
rm -f "$tmp/podman.log"
FAKE_LABEL_BACKEND=antigravity FAKE_LABEL_VOLUME=kestic-agent-2-gemini \
    KESTIC_AGENT_NAME=kestic-agent-2 "$here/kestic-agent.sh" clean --yes >/dev/null || fail "clean exited antigravity failed"
grep -qx 'localhost/kestic-agent:antigravity' "$tmp/podman.log" || fail "clean: did not target labelled backend image"

rm -f "$tmp/podman.log"
KESTIC_AGENT_BACKEND=claude "$here/kestic-agent.sh" clean --yes >/dev/null || fail "clean claude failed"
grep -qx 'localhost/kestic-agent:latest' "$tmp/podman.log" || fail "clean claude: did not remove :latest alias"

# --- 3. External/custom backend (relative path + symlinked dir) ---------------
custom_real="$tmp/custom-real/custom-bot"
custom_dir="$tmp/custom-backends"
mkdir -p "$custom_real" "$custom_dir"
cat > "$custom_real/backend.conf" <<'EOF'
description=Custom Bot
volume_suffix=custom
volume_mount=/home/kesticci/.custom
version_arg=CUSTOM_BOT_VERSION
EOF
printf '#!/usr/bin/env bash\nexit 0\n' > "$custom_real/install.sh"
printf 'example.invalid\n' > "$custom_real/allowed-domains.txt"
cat > "$custom_real/driver.sh" <<'EOF'
AGENT_DISPLAY_NAME="Custom Bot"
agent_setup() { :; }
agent_login() { :; }
agent_launch_cmd() { AGENT_CMD=(custom-bot --mode "$1" --issue "$2" "$3"); }
agent_limit_reset() { echo 0; }
EOF
ln -s "$custom_real" "$custom_dir/custom-bot"

custom_list="$(cd "$tmp" && KESTIC_AGENT_BACKENDS_DIR=custom-backends "$here/kestic-agent.sh" backends)"
grep -q '^custom-bot .*volume=-custom' <<< "$custom_list" \
    || fail "custom backend: not discovered via relative KESTIC_AGENT_BACKENDS_DIR"

rm -f "$tmp/podman.log"
(cd "$tmp" && KESTIC_AGENT_BACKENDS_DIR=custom-backends KESTIC_AGENT_BACKEND=custom-bot CUSTOM_BOT_VERSION=0.9.1 \
    "$here/kestic-agent.sh" build >/dev/null) || fail "custom backend: build failed"
grep -qx 'AGENT_BACKEND=custom-bot' "$tmp/podman.log" || fail "custom backend: AGENT_BACKEND arg missing"
grep -qx 'AGENT_VERSION=0.9.1' "$tmp/podman.log" || fail "custom backend: CUSTOM_BOT_VERSION not mapped to AGENT_VERSION"
grep -qx 'localhost/kestic-agent:custom-bot' "$tmp/podman.log" || fail "custom backend: wrong image tag"

# --- 4. Validation rejects invalid backend names, container names & mounts ----
if KESTIC_AGENT_BACKEND='../escape' "$here/kestic-agent.sh" build 2>/dev/null; then
    fail "validation: accepted path traversal in KESTIC_AGENT_BACKEND"
fi
if KESTIC_AGENT_NAME='bad/name:ro' "$here/kestic-agent.sh" backends 2>/dev/null; then
    fail "validation: accepted / and : in KESTIC_AGENT_NAME"
fi
for bad_mount in "/etc/evil" "/home/kesticci/." "/home/kesticci/kestic" "/home/kesticci/.kestic-agent" "/home/kesticci/.foo/"; do
    rm -rf "$custom_dir/bad-mount"
    mkdir -p "$custom_dir/bad-mount"
    printf 'description=Bad\nvolume_suffix=bad\nvolume_mount=%s\n' "$bad_mount" > "$custom_dir/bad-mount/backend.conf"
    touch "$custom_dir/bad-mount/install.sh" "$custom_dir/bad-mount/driver.sh" "$custom_dir/bad-mount/allowed-domains.txt"
    if KESTIC_AGENT_BACKENDS_DIR="$custom_dir" KESTIC_AGENT_BACKEND=bad-mount "$here/kestic-agent.sh" build 2>/dev/null; then
        fail "validation: accepted forbidden volume_mount '$bad_mount'"
    fi
done
rm -rf "$custom_dir/bad-ver"
mkdir -p "$custom_dir/bad-ver"
printf 'description=BadVer\nvolume_suffix=bv\nvolume_mount=/home/kesticci/.bv\nversion_arg=123_BAD\n' > "$custom_dir/bad-ver/backend.conf"
touch "$custom_dir/bad-ver/install.sh" "$custom_dir/bad-ver/driver.sh" "$custom_dir/bad-ver/allowed-domains.txt"
if KESTIC_AGENT_BACKENDS_DIR="$custom_dir" KESTIC_AGENT_BACKEND=bad-ver "$here/kestic-agent.sh" build 2>/dev/null; then
    fail "validation: accepted leading-digit version_arg"
fi

# --- 5. Driver unit tests: claude and antigravity -----------------------------
(
    export HOME="$tmp/home-claude" REPO_DIR="$tmp/home-claude/kestic" HOSTNAME="test-box"
    AGENT_DIR="$here/agents/claude"
    # shellcheck source=/dev/null
    source "$AGENT_DIR/driver.sh"
    agent_setup
    [[ "$(jq -r .skipDangerousModePermissionPrompt "$HOME/.claude/settings.json")" == true ]] \
        || fail "claude driver: skipDangerousModePermissionPrompt not set"
    KESTIC_AGENT_MODEL=opus KESTIC_AGENT_EFFORT=high agent_launch_cmd start 42 "hello"
    [[ "${AGENT_CMD[0]}" == claude ]] || fail "claude driver: wrong binary"
    printf '%s\n' "${AGENT_CMD[@]}" | grep -qx -- '--remote-control' || fail "claude driver: missing --remote-control"
    printf '%s\n' "${AGENT_CMD[@]}" | grep -qx 'test-box #42' || fail "claude driver: missing remote-control label"
    printf '%s\n' "${AGENT_CMD[@]}" | grep -qx 'opus' || fail "claude driver: missing model"
    KESTIC_AGENT_REMOTE_CONTROL=0 agent_launch_cmd resume 42 "again"
    printf '%s\n' "${AGENT_CMD[@]}" | grep -qx -- '--continue' || fail "claude driver: resume missing --continue"
    ! printf '%s\n' "${AGENT_CMD[@]}" | grep -qx -- '--remote-control' || fail "claude driver: REMOTE_CONTROL=0 ignored"
)

(
    export HOME="$tmp/home-agy" REPO_DIR="$tmp/home-agy/kestic" HOSTNAME="test-box"
    mkdir -p "$HOME/.gemini/config" "$HOME/.config/antigravity"
    echo '{"other-hook":{"Stop":[]}}' > "$HOME/.gemini/config/hooks.json"
    AGENT_DIR="$here/agents/antigravity"
    # shellcheck source=/dev/null
    source "$AGENT_DIR/driver.sh"
    agent_setup
    [[ -L "$HOME/.config/antigravity" ]] || fail "antigravity driver: empty dir not replaced with auth symlink"
    [[ "$(readlink "$HOME/.config/antigravity")" == "$HOME/.gemini/antigravity-auth" ]] \
        || fail "antigravity driver: auth symlink target wrong"
    [[ "$(jq -r --arg d "$REPO_DIR" '.trustedWorkspaces | index($d) != null' "$HOME/.gemini/antigravity-cli/settings.json")" == true ]] \
        || fail "antigravity driver: workspace not pre-trusted"
    [[ "$(jq -r 'has("kestic-agent") and has("other-hook")' "$HOME/.gemini/config/hooks.json")" == true ]] \
        || fail "antigravity driver: hooks.json merge lost existing hook or omitted kestic-agent"

    KESTIC_AGENT_MODEL=gemini-4-neon-high KESTIC_AGENT_EFFORT=high agent_launch_cmd start 55 "work"
    [[ "${AGENT_CMD[0]}" == agy ]] || fail "antigravity driver: wrong binary"
    printf '%s\n' "${AGENT_CMD[@]}" | grep -qx -- '--dangerously-skip-permissions' || fail "antigravity driver: missing bypass flag"
    printf '%s\n' "${AGENT_CMD[@]}" | grep -qx -- '--remote-control' || fail "antigravity driver: missing --remote-control"
    ! printf '%s\n' "${AGENT_CMD[@]}" | grep -qx 'test-box #55' || fail "antigravity driver: --remote-control must not take title arg"
    printf '%s\n' "${AGENT_CMD[@]}" | grep -qx 'gemini-4-neon-high' || fail "antigravity driver: missing model"
    printf '%s\n' "${AGENT_CMD[@]}" | grep -qx -- '-i' || fail "antigravity driver: missing -i flag"

    KESTIC_AGENT_REMOTE_CONTROL=0 agent_launch_cmd resume 55 "continue"
    printf '%s\n' "${AGENT_CMD[@]}" | grep -qx -- '--continue' || fail "antigravity driver: resume missing --continue"
    ! printf '%s\n' "${AGENT_CMD[@]}" | grep -qx -- '--remote-control' || fail "antigravity driver: REMOTE_CONTROL=0 ignored"
    [[ "$(agent_limit_reset 100 "anything")" == 0 ]] || fail "antigravity driver: limit_reset should return 0"
)

# --- 6. Antigravity hook.sh + stale-idle nudge turn advancement ---------------
export HOME="$tmp/home-hook"
mkdir -p "$HOME/.kestic-agent"
export KESTIC_AGENT_BIN="$here/kestic-agent-inner.sh" KESTIC_AGENT_DRIVER_DIR="$here/agents/antigravity"
# First PreInvocation with no turn file yet creates turn
out="$(printf '{"invocationNum":1}\n' | "$here/agents/antigravity/hook.sh" pre-invocation)"
[[ "$out" == '{}' ]] || fail "antigravity hook pre-invocation(first): expected {}, got '$out'"
[[ -s "$HOME/.kestic-agent/turn" ]] || fail "antigravity hook pre-invocation(first): did not create turn"
echo 1000 > "$HOME/.kestic-agent/turn"
# Busy mid-turn: PreInvocation must NOT advance turn timestamp
out="$(printf '{"invocationNum":2}\n' | "$here/agents/antigravity/hook.sh" pre-invocation)"
[[ "$out" == '{}' ]] || fail "antigravity hook pre-invocation: expected {}, got '$out'"
[[ "$(cat "$HOME/.kestic-agent/turn")" == 1000 ]] || fail "antigravity hook pre-invocation: overwrote turn mid-turn"
# Stop with fullyIdle=false: must stay busy
out="$(printf '{"fullyIdle":false}\n' | "$here/agents/antigravity/hook.sh" stop)"
[[ "$out" == '{"decision":"allow"}' ]] || fail "antigravity hook stop(busy): unexpected output '$out'"
[[ ! -e "$HOME/.kestic-agent/idle" ]] || fail "antigravity hook stop(fullyIdle=false): marked idle prematurely"
# Stop with missing fullyIdle defaults to idle
out="$(printf '{}\n' | "$here/agents/antigravity/hook.sh" stop)"
[[ "$out" == '{"decision":"allow"}' ]] || fail "antigravity hook stop(default): unexpected output '$out'"
[[ -e "$HOME/.kestic-agent/idle" ]] || fail "antigravity hook stop(missing fullyIdle): did not mark idle"
# Next PreInvocation after idle: starts new turn and clears idle
out="$(printf '{"invocationNum":1}\n' | "$here/agents/antigravity/hook.sh" pre-invocation)"
[[ "$out" == '{}' ]] || fail "antigravity hook pre-invocation(after idle): unexpected output '$out'"
[[ ! -e "$HOME/.kestic-agent/idle" ]] || fail "antigravity hook pre-invocation(after idle): did not clear idle"
[[ "$(cat "$HOME/.kestic-agent/turn")" -gt 1000 ]] || fail "antigravity hook pre-invocation(after idle): did not update turn"

# Stale-idle nudge (no idle file, window_activity older than STALE_AFTER) advances turn
cat > "$tmp/bin/tmux" <<'FAKE'
#!/usr/bin/env bash
case "$1" in
    has-session) exit 0 ;;
    list-panes) echo 0 ;;
    display-message) echo 1000 ;;
    send-keys) exit 0 ;;
    *) exit 1 ;;
esac
FAKE
chmod +x "$tmp/bin/tmux"
echo 1000 > "$HOME/.kestic-agent/turn"
rm -f "$HOME/.kestic-agent/idle"
"$here/kestic-agent-inner.sh" nudge "please continue" || fail "nudge on stale-idle failed"
[[ "$(cat "$HOME/.kestic-agent/turn")" -gt 1000 ]] || fail "nudge on stale-idle: did not advance turn timestamp"

# --- 7. Real init-firewall.sh domain merging & strict validation --------------
fw_agy="$tmp/fw-agy"
fw_claude="$tmp/fw-claude"
fw_bad="$tmp/fw-bad"
mkdir -p "$fw_agy/agent" "$fw_claude/agent" "$fw_bad/agent"
cp "$here/allowed-domains.txt" "$fw_agy/"
cp "$here/agents/antigravity/allowed-domains.txt" "$fw_agy/agent/"
cp "$here/allowed-domains.txt" "$fw_claude/"
cp "$here/agents/claude/allowed-domains.txt" "$fw_claude/agent/"
cp "$here/allowed-domains.txt" "$fw_bad/"

agy_domains="$("$here/init-firewall.sh" --print-domains "$fw_agy")"
grep -qx 'github.com' <<< "$agy_domains" || fail "firewall(antigravity): missing shared github.com"
grep -qx 'antigravity.google' <<< "$agy_domains" || fail "firewall(antigravity): missing antigravity.google"
grep -qx 'daily-cloudcode-pa.googleapis.com' <<< "$agy_domains" || fail "firewall(antigravity): missing daily-cloudcode-pa.googleapis.com"
! grep -qx 'anthropic.com' <<< "$agy_domains" || fail "firewall(antigravity): must not include anthropic.com"
! grep -qx 'google.com' <<< "$agy_domains" || fail "firewall(antigravity): must not wildcard apex google.com"
! grep -qx 'googleapis.com' <<< "$agy_domains" || fail "firewall(antigravity): must not wildcard apex googleapis.com"

claude_domains="$("$here/init-firewall.sh" --print-domains "$fw_claude")"
grep -qx 'github.com' <<< "$claude_domains" || fail "firewall(claude): missing shared github.com"
grep -qx 'anthropic.com' <<< "$claude_domains" || fail "firewall(claude): missing anthropic.com"
! grep -qx 'antigravity.google' <<< "$claude_domains" || fail "firewall(claude): must not include antigravity.google"

for bad_domain in "." ".." "-bad.example" ".leading.dot" "no-tld"; do
    printf '%s\n' "$bad_domain" > "$fw_bad/agent/allowed-domains.txt"
    if "$here/init-firewall.sh" --print-domains "$fw_bad" >/dev/null 2>&1; then
        fail "firewall validation: accepted invalid domain '$bad_domain'"
    fi
done

# --- 8. commit-msg hook rejects Antigravity attribution -----------------------
msg="$tmp/commit-msg.txt"
printf 'feat: test\n\nCo-Authored-By: Antigravity <noreply@antigravity.google>\n' > "$msg"
if "$here/git-hooks/commit-msg" "$msg" 2>/dev/null; then
    fail "commit-msg: did not reject Antigravity Co-Authored-By"
fi
printf 'feat: test\n\nGenerated with Antigravity\n' > "$msg"
if "$here/git-hooks/commit-msg" "$msg" 2>/dev/null; then
    fail "commit-msg: did not reject Generated with Antigravity"
fi
printf 'feat: normal commit without attribution\n' > "$msg"
"$here/git-hooks/commit-msg" "$msg" || fail "commit-msg: rejected normal commit"

if ((failures > 0)); then
    echo "test-kestic-agent-backends: $failures failure(s)" >&2
    exit 1
fi
echo "test-kestic-agent-backends: ok"
