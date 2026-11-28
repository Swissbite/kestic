#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause
#
# Tests the pluggable agent backend architecture (.devcontainer/agents/):
# backend discovery, build/run/volume/env wiring for claude, antigravity and
# an out-of-tree custom backend, driver launch commands, Antigravity hooks,
# firewall domain isolation, and AI attribution rejection in commit-msg.

set -euo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
failures=0

fail() {
    echo "FAIL: $*" >&2
    failures=$((failures + 1))
}

mkdir -p "$tmp/bin"
cat > "$tmp/bin/podman" <<'FAKE'
#!/usr/bin/env bash
{ printf '%s\n' "$@"; echo ---; } >> "$FAKE_DIR/podman.log"
case "$1" in
    image)
        if [[ "$2" == exists ]]; then
            [[ "${FAKE_IMAGE_EXISTS:-true}" == true ]]
            exit $?
        fi ;;
    secret) exit 0 ;;
    container) echo "${FAKE_RUNNING:-false}" ;;
    exec)
        case "${@: -1}" in
            /run/kestic-agent/ready) exit 0 ;;
            token-age) echo 10 ;;
            store-token) cat >/dev/null ;;
        esac ;;
    run) echo "fake-cid" ;;
    build | tag | rm | stop | rmi) exit 0 ;;
esac
FAKE
cat > "$tmp/bin/curl" <<'FAKE'
#!/usr/bin/env bash
case "${@: -1}" in
    */installation) echo '{"id":4711}' ;;
    */access_tokens) echo '{"token":"ghs_FakeBackendTestToken"}' ;;
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
grep -qx 'AGY_VERSION=1.5.19' "$tmp/podman.log" || fail "antigravity build: AGY_VERSION arg not forwarded"
grep -qx 'localhost/kestic-agent:antigravity' "$tmp/podman.log" || fail "antigravity build: wrong image tag"
! grep -qx 'localhost/kestic-agent:latest' "$tmp/podman.log" || fail "antigravity build: must not overwrite :latest"

rm -f "$tmp/podman.log"
KESTIC_AGENT_BACKEND=claude "$here/kestic-agent.sh" build >/dev/null || fail "claude build failed"
grep -qx 'AGENT_BACKEND=claude' "$tmp/podman.log" || fail "claude build: AGENT_BACKEND arg missing"
grep -qx 'localhost/kestic-agent:claude' "$tmp/podman.log" || fail "claude build: wrong image tag"
grep -qx 'localhost/kestic-agent:latest' "$tmp/podman.log" || fail "claude build: :latest alias missing"

# Start container with antigravity + model/effort/name overrides
rm -f "$tmp/podman.log"
openssl genrsa -out "$tmp/key.pem" 2048 2>/dev/null
cat > "$tmp/bin/podman" <<'FAKE'
#!/usr/bin/env bash
{ printf '%s\n' "$@"; echo ---; } >> "$FAKE_DIR/podman.log"
case "$1" in
    image) [[ "${FAKE_IMAGE_EXISTS:-true}" == true ]] ;;
    secret) [[ "$2" == inspect && "$3" == --showsecret ]] && cat "$FAKE_DIR/key.pem"; exit 0 ;;
    container)
        if [[ -f "$FAKE_DIR/started" ]]; then echo true; else echo false; fi ;;
    run) touch "$FAKE_DIR/started"; echo "fake-cid" ;;
    exec)
        case "${@: -1}" in
            /run/kestic-agent/ready) exit 0 ;;
            token-age) echo 10 ;;
            store-token) cat >/dev/null ;;
        esac ;;
    build | tag | rm | stop | rmi) exit 0 ;;
esac
FAKE
chmod +x "$tmp/bin/podman"

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

# --- 3. External/custom third-party backend works without touching core -------
custom_dir="$tmp/custom-backends"
mkdir -p "$custom_dir/custom-bot"
cat > "$custom_dir/custom-bot/backend.conf" <<'EOF'
description=Custom Bot
volume_suffix=custom
volume_mount=/home/kesticci/.custom
version_arg=CUSTOM_BOT_VERSION
EOF
printf '#!/usr/bin/env bash\nexit 0\n' > "$custom_dir/custom-bot/install.sh"
printf 'example.invalid\n' > "$custom_dir/custom-bot/allowed-domains.txt"
cat > "$custom_dir/custom-bot/driver.sh" <<'EOF'
AGENT_DISPLAY_NAME="Custom Bot"
agent_setup() { :; }
agent_login() { :; }
agent_launch_cmd() { AGENT_CMD=(custom-bot --mode "$1" --issue "$2" "$3"); }
agent_limit_reset() { echo 0; }
EOF

custom_list="$(KESTIC_AGENT_BACKENDS_DIR="$custom_dir" "$here/kestic-agent.sh" backends)"
grep -q '^custom-bot .*volume=-custom' <<< "$custom_list" \
    || fail "custom backend: not discovered in KESTIC_AGENT_BACKENDS_DIR"

rm -f "$tmp/podman.log"
KESTIC_AGENT_BACKENDS_DIR="$custom_dir" KESTIC_AGENT_BACKEND=custom-bot CUSTOM_BOT_VERSION=0.9.1 \
    "$here/kestic-agent.sh" build >/dev/null || fail "custom backend: build failed"
grep -qx 'AGENT_BACKEND=custom-bot' "$tmp/podman.log" || fail "custom backend: AGENT_BACKEND arg missing"
grep -qx 'CUSTOM_BOT_VERSION=0.9.1' "$tmp/podman.log" || fail "custom backend: version_arg not forwarded"
grep -qx 'localhost/kestic-agent:custom-bot' "$tmp/podman.log" || fail "custom backend: wrong image tag"

# --- 4. Validation rejects invalid backend names and mounts -------------------
if KESTIC_AGENT_BACKEND='../escape' "$here/kestic-agent.sh" build 2>/dev/null; then
    fail "validation: accepted path traversal in KESTIC_AGENT_BACKEND"
fi
mkdir -p "$custom_dir/bad-mount"
cat > "$custom_dir/bad-mount/backend.conf" <<'EOF'
description=Bad Mount
volume_suffix=bad
volume_mount=/etc/evil
EOF
touch "$custom_dir/bad-mount/install.sh" "$custom_dir/bad-mount/driver.sh" "$custom_dir/bad-mount/allowed-domains.txt"
if KESTIC_AGENT_BACKENDS_DIR="$custom_dir" KESTIC_AGENT_BACKEND=bad-mount "$here/kestic-agent.sh" build 2>/dev/null; then
    fail "validation: accepted volume_mount outside /home/kesticci"
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
    mkdir -p "$HOME/.gemini/config"
    echo '{"other-hook":{"Stop":[]}}' > "$HOME/.gemini/config/hooks.json"
    AGENT_DIR="$here/agents/antigravity"
    # shellcheck source=/dev/null
    source "$AGENT_DIR/driver.sh"
    agent_setup
    [[ -L "$HOME/.config/antigravity" ]] || fail "antigravity driver: auth symlink not created"
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

# --- 6. Antigravity hook.sh turn/idle state transitions -----------------------
export HOME="$tmp/home-hook"
mkdir -p "$HOME/.kestic-agent"
export KESTIC_AGENT_BIN="$here/kestic-agent-inner.sh" KESTIC_AGENT_DRIVER_DIR="$here/agents/antigravity"
echo 1000 > "$HOME/.kestic-agent/turn"
# Busy mid-turn: PreInvocation must NOT advance turn timestamp
out="$(printf '{"invocationNum":2}\n' | "$here/agents/antigravity/hook.sh" pre-invocation)"
[[ "$out" == '{}' ]] || fail "antigravity hook pre-invocation: expected {}, got '$out'"
[[ "$(cat "$HOME/.kestic-agent/turn")" == 1000 ]] || fail "antigravity hook pre-invocation: overwrote turn mid-turn"
# Stop with fullyIdle=false: must stay busy
out="$(printf '{"fullyIdle":false}\n' | "$here/agents/antigravity/hook.sh" stop)"
[[ "$out" == '{"decision":"allow"}' ]] || fail "antigravity hook stop(busy): unexpected output '$out'"
[[ ! -e "$HOME/.kestic-agent/idle" ]] || fail "antigravity hook stop(fullyIdle=false): marked idle prematurely"
# Stop with fullyIdle=true: marks idle
out="$(printf '{"fullyIdle":true}\n' | "$here/agents/antigravity/hook.sh" stop)"
[[ "$out" == '{"decision":"allow"}' ]] || fail "antigravity hook stop(idle): unexpected output '$out'"
[[ -e "$HOME/.kestic-agent/idle" ]] || fail "antigravity hook stop(fullyIdle=true): did not mark idle"
# Next PreInvocation after idle: starts new turn and clears idle
out="$(printf '{"invocationNum":1}\n' | "$here/agents/antigravity/hook.sh" pre-invocation)"
[[ "$out" == '{}' ]] || fail "antigravity hook pre-invocation(after idle): unexpected output '$out'"
[[ ! -e "$HOME/.kestic-agent/idle" ]] || fail "antigravity hook pre-invocation(after idle): did not clear idle"
[[ "$(cat "$HOME/.kestic-agent/turn")" -gt 1000 ]] || fail "antigravity hook pre-invocation(after idle): did not update turn"

# --- 7. Firewall domain isolation per backend ---------------------------------
! grep -qx 'anthropic.com' "$here/allowed-domains.txt" || fail "shared allowed-domains.txt must not contain anthropic.com"
! grep -qx 'googleapis.com' "$here/allowed-domains.txt" || fail "shared allowed-domains.txt must not contain googleapis.com"
grep -qx 'anthropic.com' "$here/agents/claude/allowed-domains.txt" || fail "claude allowed-domains missing anthropic.com"
! grep -qx 'googleapis.com' "$here/agents/claude/allowed-domains.txt" || fail "claude allowed-domains must not contain googleapis.com"
grep -qx 'antigravity.google' "$here/agents/antigravity/allowed-domains.txt" || fail "antigravity allowed-domains missing antigravity.google"
grep -qx 'googleapis.com' "$here/agents/antigravity/allowed-domains.txt" || fail "antigravity allowed-domains missing googleapis.com"
! grep -qx 'anthropic.com' "$here/agents/antigravity/allowed-domains.txt" || fail "antigravity allowed-domains must not contain anthropic.com"

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
