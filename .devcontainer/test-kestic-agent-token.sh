#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause
#
# Tests the GitHub App token of kestic-agent.sh with a throwaway RSA key, a fake
# curl and a fake podman: the exact requests, a JWT that verifies with the key,
# the installation token reaching the container unchanged through stdin, neither
# key nor token in any argv, renewal only when due, and failures that store
# nothing. Also tests store-token and state of kestic-agent-inner.sh.

set -euo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
failures=0

fail() {
    echo "FAIL: $*" >&2
    failures=$((failures + 1))
}

openssl genrsa -out "$tmp/key.pem" 2048 2>/dev/null
openssl rsa -in "$tmp/key.pem" -pubout -out "$tmp/pub.pem" 2>/dev/null
TOKEN=ghs_TestToken0123456789abcdefABCDEF
export FAKE_TOKEN="$TOKEN" FAKE_KEY="$tmp/key.pem" FAKE_DIR="$tmp" KESTIC_AGENT_APP_ID=123456 \
    FAKE_INSTALLATION='{"id":4711,"account":{"login":"Swissbite","id":99}}' \
    FAKE_ACCESS="{\"token\":\"$TOKEN\",\"expires_at\":\"2026-11-22T13:00:00Z\",\"repository_selection\":\"selected\"}"

# Fake podman: records every call (one argument per line), answers what
# kestic-agent.sh asks, keeps what store-token receives on stdin.
mkdir -p "$tmp/bin"
cat > "$tmp/bin/podman" <<'FAKE'
#!/usr/bin/env bash
{ printf '%s\n' "$@"; echo ---; } >> "$FAKE_DIR/podman.log"
case "$1" in
    container) echo "${FAKE_RUNNING:-true}" ;;
    secret) cat "$FAKE_KEY" ;;
    exec)
        case "${@: -1}" in
            store-token) cat > "$FAKE_DIR/stored" ;;
            token-age) echo "$FAKE_AGE" ;;
            state) echo "idle 5 100" ;;
            show-token) echo "$FAKE_TOKEN" ;;
        esac ;;
esac
FAKE
# Fake curl: records its arguments, with the descriptor of the header file
# replaced by @FD, and the header file's content; answers by URL.
cat > "$tmp/bin/curl" <<'FAKE'
#!/usr/bin/env bash
for arg in "$@"; do
    if [[ "$arg" == @/dev/fd/* || "$arg" == @/proc/* ]]; then
        cat "${arg#@}" >> "$FAKE_DIR/headers"
        arg=@FD
    fi
    printf '%s\n' "$arg"
done >> "$FAKE_DIR/curl.log"
echo --- >> "$FAKE_DIR/curl.log"
case "${@: -1}" in
    */installation) [[ -n "$FAKE_INSTALLATION" ]] || { echo '{"message":"Not Found"}'; exit 22; }
        echo "$FAKE_INSTALLATION" ;;
    */access_tokens) [[ -n "$FAKE_ACCESS" ]] || { echo '{"message":"Forbidden"}'; exit 22; }
        echo "$FAKE_ACCESS" ;;
    */installation/token) ;;
    *) exit 22 ;;
esac
FAKE
chmod +x "$tmp/bin/podman" "$tmp/bin/curl"
export PATH="$tmp/bin:$PATH"

reset() {
    rm -f "$tmp/podman.log" "$tmp/curl.log" "$tmp/headers" "$tmp/stored"
    export FAKE_AGE=none FAKE_RUNNING=true
}

# --- a new token on request -------------------------------------------------
reset
"$here/kestic-agent.sh" token || fail "token: exit $?"

expected_curl="--silent
--show-error
--fail-with-body
--max-time
30
-X
GET
-H
Accept: application/vnd.github+json
-H
X-GitHub-Api-Version: 2022-11-28
-H
@FD
https://api.github.com/repos/Swissbite/kestic/installation
---
--silent
--show-error
--fail-with-body
--max-time
30
-X
POST
-H
Accept: application/vnd.github+json
-H
X-GitHub-Api-Version: 2022-11-28
--data
{\"repositories\":[\"kestic\"]}
-H
@FD
https://api.github.com/app/installations/4711/access_tokens
---"
[[ "$(cat "$tmp/curl.log")" == "$expected_curl" ]] ||
    fail "token: curl arguments differ:"$'\n'"$(diff <(echo "$expected_curl") "$tmp/curl.log")"

[[ "$(cat "$tmp/stored" 2>/dev/null)" == "$TOKEN" ]] || fail "token: the container did not receive the token unchanged"
tr '\n' ' ' < "$tmp/podman.log" | grep -qF -- \
    "--- exec -i kestic-agent /usr/local/lib/kestic-agent/entrypoint.sh --drop kestic-agent store-token ---" ||
    fail "token: store-token not run with stdin as kesticci in container kestic-agent"
key_line="$(sed -n 2p "$tmp/key.pem")"
for log in podman.log curl.log; do
    ! grep -qF -- "$TOKEN" "$tmp/$log" || fail "token: the installation token appears in the arguments ($log)"
    ! grep -qF -- "$key_line" "$tmp/$log" || fail "token: the private key appears in the arguments ($log)"
done

# The JWT: same bearer for both requests, RS256, issuer = App ID, at most 10
# minutes, and a signature that verifies with the public key.
mapfile -t bearers < <(sed -n 's/^Authorization: Bearer //p' "$tmp/headers")
[[ ${#bearers[@]} == 2 && "${bearers[0]}" == "${bearers[1]}" ]] || fail "token: expected one bearer for two requests"
jwt="${bearers[0]:-x.x.x}"
! grep -qF -- "$jwt" "$tmp/curl.log" || fail "token: the JWT appears in curl's arguments"
python3 -I - "$jwt" "$tmp" <<'PY' || fail "token: JWT header or claims wrong"
import base64, json, sys, time
jwt, out = sys.argv[1], sys.argv[2]
part = lambda p: base64.urlsafe_b64decode(p + "=" * (-len(p) % 4))
head, claims, sig = jwt.split(".")
assert json.loads(part(head)) == {"typ": "JWT", "alg": "RS256"}, part(head)
c = json.loads(part(claims))
now = int(time.time())
assert c["iss"] == "123456", c
assert c["exp"] - c["iat"] == 600 and now - 120 <= c["iat"] <= now - 59 and c["exp"] <= now + 600, c
open(f"{out}/signed", "w").write(f"{head}.{claims}")
open(f"{out}/sig", "wb").write(part(sig))
PY
openssl dgst -sha256 -verify "$tmp/pub.pem" -signature "$tmp/sig" "$tmp/signed" >/dev/null 2>&1 ||
    fail "token: the JWT signature does not verify with the App's key"

# --- renewal by agent-state ---------------------------------------------------
reset
export FAKE_AGE=100
[[ "$("$here/kestic-agent.sh" agent-state 2>/dev/null)" == "idle 5 100 ok" ]] || fail "agent-state, fresh token: state line"
[[ ! -e "$tmp/curl.log" ]] || fail "agent-state, fresh token: renewed although not due"
reset
export FAKE_AGE=2401
[[ "$("$here/kestic-agent.sh" agent-state 2>/dev/null)" == "idle 5 100 ok" ]] || fail "agent-state, old token: state line"
[[ "$(cat "$tmp/stored" 2>/dev/null)" == "$TOKEN" ]] || fail "agent-state, old token: not renewed"
reset
[[ "$("$here/kestic-agent.sh" agent-state 2>/dev/null)" == "idle 5 100 ok" ]] || fail "agent-state, no token: state line"
[[ "$(cat "$tmp/stored" 2>/dev/null)" == "$TOKEN" ]] || fail "agent-state, no token: not renewed"
reset
export FAKE_RUNNING=false
[[ "$("$here/kestic-agent.sh" agent-state 2>/dev/null)" == "off 0 0 ok" ]] || fail "agent-state, container off: state line"
[[ ! -e "$tmp/curl.log" ]] || fail "agent-state, container off: asked GitHub for a token"

# --- failures store nothing -----------------------------------------------------
reset
if KESTIC_AGENT_APP_ID=abc "$here/kestic-agent.sh" token 2>/dev/null; then fail "token without App ID: succeeded"; fi
[[ ! -e "$tmp/curl.log" ]] || fail "token without App ID: called GitHub"
reset
if FAKE_INSTALLATION= "$here/kestic-agent.sh" token 2>/dev/null; then fail "token, App not installed: succeeded"; fi
[[ ! -e "$tmp/stored" ]] || fail "token, App not installed: stored something"
[[ "$(FAKE_INSTALLATION= "$here/kestic-agent.sh" agent-state 2>/dev/null)" == "idle 5 100 failed" ]] ||
    fail "agent-state, renewal fails: state line missing or not marked failed"
reset
if FAKE_ACCESS= "$here/kestic-agent.sh" token 2>/dev/null; then fail "token refused by GitHub: succeeded"; fi
[[ ! -e "$tmp/stored" ]] || fail "token refused by GitHub: stored something"
reset
KESTIC_AGENT_NAME=other-box "$here/kestic-agent.sh" token || fail "token, other container: exit $?"
tr '\n' ' ' < "$tmp/podman.log" | grep -qF -- "--- exec -i other-box " ||
    fail "token, other container: KESTIC_AGENT_NAME not used"

# --- stop revokes the token with itself ----------------------------------------
reset
"$here/kestic-agent.sh" stop > /dev/null || fail "stop: exit $?"
expected_revoke="--silent
--show-error
--fail-with-body
--max-time
30
-X
DELETE
-H
Accept: application/vnd.github+json
-H
X-GitHub-Api-Version: 2022-11-28
-H
@FD
https://api.github.com/installation/token
---"
[[ "$(cat "$tmp/curl.log" 2>/dev/null)" == "$expected_revoke" ]] || fail "stop: revoke request differs"
[[ "$(cat "$tmp/headers" 2>/dev/null)" == "Authorization: Bearer $TOKEN" ]] || fail "stop: not revoked with the token itself"
! grep -qF -- "$TOKEN" "$tmp/curl.log" "$tmp/podman.log" || fail "stop: the token appears in the arguments"
grep -qx stop "$tmp/podman.log" || fail "stop: container not stopped"
reset
export FAKE_RUNNING=false
"$here/kestic-agent.sh" stop > /dev/null || fail "stop, container off: exit $?"
[[ ! -e "$tmp/curl.log" ]] || fail "stop, container off: called GitHub"

# --- kestic-agent-inner.sh: store-token and state -----------------------------------
inner="$here/kestic-agent-inner.sh"
export HOME="$tmp/home" KESTIC_AGENT_TOKEN_FILE="$tmp/home/gh-token"
mkdir -p "$HOME"
printf '%s\n' "$TOKEN" | "$inner" store-token || fail "store-token: exit $?"
[[ "$(cat "$KESTIC_AGENT_TOKEN_FILE")" == "$TOKEN" ]] || fail "store-token: content"
[[ "$(stat -c %a "$KESTIC_AGENT_TOKEN_FILE")" == 400 ]] || fail "store-token: mode is not 0400"
printf '%s\n' ghs_Second | "$inner" store-token || fail "store-token, replace: exit $?"
[[ "$(cat "$KESTIC_AGENT_TOKEN_FILE")" == ghs_Second ]] || fail "store-token: not replaced"
if printf 'not a token\n' | "$inner" store-token 2>/dev/null; then fail "store-token: accepted garbage"; fi
if printf 'ghs_ok; rm -rf x\n' | "$inner" store-token 2>/dev/null; then fail "store-token: accepted garbage"; fi
[[ "$(cat "$KESTIC_AGENT_TOKEN_FILE")" == ghs_Second ]] || fail "store-token: garbage changed the file"
[[ "$("$inner" token-age)" =~ ^[0-9]+$ ]] || fail "token-age: not a number"
mkdir -p "$HOME/.kestic-agent"
: > "$HOME/.kestic-agent/issue"
printf '12x$(id)\n' > "$HOME/.kestic-agent/turn"
[[ "$("$inner" state)" == "none 0 12" ]] || fail "state: empty or odd files not sanitised: '$("$inner" state)'"

if ((failures > 0)); then
    echo "test-kestic-agent-token: $failures failure(s)" >&2
    exit 1
fi
echo "test-kestic-agent-token: ok"
