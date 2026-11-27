#!/usr/bin/env bash
#
# Tests the forge events of watch.sh against a fake gh that runs the real --jq filters over raw
# GitHub JSON: question: only after the agent's comment, external: with the number of withheld
# items, and pull requests from forks or other authors never reported as ready or red.
# Usage: test-watch.sh <trust jq file>
set -euo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
trust="$(realpath "${1:?usage: test-watch.sh <trust jq file>}")"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
fix="$tmp/fixtures"
mkdir -p "$tmp/bin" "$tmp/repo" "$tmp/config/leitstand"

# Fake gh: `api [--paginate] repos/<o>/<r>/<rest>` reads fixture <rest> with / ? & = : replaced
# by _, `pr list --base <b>` reads pr-list-<b>; the call's --jq filter runs over it. A missing
# fixture is an empty list; a file <fixture>.fail makes the call fail.
cat > "$tmp/bin/gh" <<'FAKE'
#!/usr/bin/env bash
key= filter=
[[ "$1 $2" == "pr list" ]] && key=pr-list
while [[ $# -gt 0 ]]; do
    case "$1" in
        --jq) filter="$2"; shift ;;
        --base) [[ "$key" == pr-list ]] && key="pr-list-$2"; shift ;;
        repos/*) [[ -z "$key" ]] && key="$(tr '/?&=:' '_____' <<< "${1#repos/*/*/}")" ;;
    esac
    shift
done
file="$FAKE_FIXTURES/$key"
[[ -f "$file.fail" ]] && exit 1
[[ -f "$file" ]] || { echo '[]' | jq -r "$filter"; exit 0; }
jq -r "$filter" "$file"
FAKE
# The loop sleeps between polls; here one second is enough.
printf '#!/usr/bin/env bash\nexec /bin/sleep 1\n' > "$tmp/bin/sleep"
chmod +x "$tmp/bin/gh" "$tmp/bin/sleep"

cat > "$tmp/config/leitstand/test.env" <<EOF
LEITSTAND_REPO=$tmp/repo
LEITSTAND_MAINTAINER=Swissbite
LEITSTAND_AGENTS="agent-a|m6|M6 Test"
LEITSTAND_TRUST_JQ=$trust
LEITSTAND_CHECKS="verify"
LEITSTAND_DISK_MIN_GB=0
LEITSTAND_PROMPT_PATTERN=
LEITSTAND_LIMIT_PATTERN=
LEITSTAND_SUBAGENT_PATTERN=
EOF
export PATH="$tmp/bin:$PATH" FAKE_FIXTURES="$fix" XDG_CONFIG_HOME="$tmp/config" XDG_STATE_HOME="$tmp/state"
failures=0

fail() {
    echo "FAIL: $*" >&2
    failures=$((failures + 1))
}

M='{"login":"Swissbite","id":205621,"type":"User"}'
A='{"login":"kestic-agent[bot]","id":359840804,"type":"Bot"}'
O='{"login":"mallory","id":4242,"type":"User"}'

# run: one watch pass from a fresh state; its event lines in $out.
run() {
    rm -rf "$XDG_STATE_HOME"
    out="$("$here/watch.sh" test 1 | grep -v '^[0-9][0-9]:[0-9][0-9] ' | grep -v '^disk:' || true)"
}

reset() {
    rm -rf "$fix"
    mkdir -p "$fix"
    echo '[{"number":3,"title":"M6 Test"},{"number":4,"title":"M5 Old"}]' > "$fix/milestones_state_all_per_page_100"
}

# issue <n> <author> <labels> <updated>: one open issue of the milestone (JSON object)
issue() {
    printf '{"number":%s,"user":%s,"labels":%s,"updated_at":"%s"}' "$1" "$2" "$3" "$4"
}

# comments <n> "<time> <user variable>"…
comments() {
    local n="$1" items=() t u
    shift
    for entry in "$@"; do
        read -r t u <<< "$entry"
        items+=("{\"created_at\":\"$t\",\"user\":${!u}}")
    done
    (IFS=,; echo "[${items[*]}]") > "$fix/issues_${n}_comments_per_page_100"
}

# pull <n> <author> <head repository>: one open pull request into m6 (JSON object)
pull() {
    printf '{"number":%s,"user":%s,"updated_at":"2026-11-27T10:00:00Z","head":{"repo":{"full_name":"%s"}},"base":{"repo":{"full_name":"Swissbite/kestic"}}}' \
        "$1" "$2" "$3"
}

# checks <n> <conclusion>: the pr list entry (not a fork) with one required check
checks() {
    printf '{"number":%s,"isCrossRepository":%s,"isDraft":false,"headRefOid":"abcdef0123","statusCheckRollup":[{"name":"verify","conclusion":"%s"}]}' \
        "$1" "${3:-false}" "$2"
}

held='[{"name":"needs-maintainer"}]'

# question: only when the agent's comment is the last one by the maintainer or the agent
reset
echo "[$(issue 5 "$M" "$held" 2026-11-27T10:00:00Z), $(issue 6 "$M" "$held" 2026-11-27T10:00:00Z),
    $(issue 7 "$M" "$held" 2026-11-27T10:00:00Z), $(issue 8 "$M" '[]' 2026-11-27T10:00:00Z)]" \
    > "$fix/issues_milestone_3_state_open_per_page_100"
comments 5 "2026-11-27T09:00:00Z M" "2026-11-27T09:30:00Z A"
comments 6 "2026-11-27T09:00:00Z A" "2026-11-27T09:30:00Z M"
comments 7 "2026-11-27T09:00:00Z O"
comments 8 "2026-11-27T09:00:00Z A"
run
grep -qx "question:m6:#5:2026-11-27T09:30:00Z" <<< "$out" || fail "question: the agent's last comment on a held issue not reported"
! grep -q "question:m6:#6:" <<< "$out" || fail "question: reported although the maintainer answered last"
! grep -q "question:m6:#7:" <<< "$out" || fail "question: an outsider's comment counts as a question"
! grep -q "question:m6:#8:" <<< "$out" || fail "question: reported for an issue that is not held"

# An outsider's comment after the agent's question does not hide it
reset
echo "[$(issue 5 "$M" "$held" 2026-11-27T10:00:00Z)]" > "$fix/issues_milestone_3_state_open_per_page_100"
comments 5 "2026-11-27T09:00:00Z A" "2026-11-27T09:30:00Z O"
run
grep -qx "question:m6:#5:2026-11-27T09:00:00Z" <<< "$out" || fail "question: an outsider's later comment hides the agent's question"
grep -qx "external:m6:#5:1" <<< "$out" || fail "question: the outsider's comment not counted"

# external: withheld items on milestone issues, the outsider's issue body included
reset
echo "[$(issue 5 "$M" '[]' 2026-11-27T10:00:00Z), $(issue 6 "$O" '[]' 2026-11-27T10:00:00Z),
    $(issue 7 "$O" '[{"name":"adopted"}]' 2026-11-27T10:00:00Z), $(issue 8 "$M" '[]' 2026-11-27T10:00:00Z)]" \
    > "$fix/issues_milestone_3_state_open_per_page_100"
comments 5 "2026-11-27T09:00:00Z O" "2026-11-27T09:10:00Z O" "2026-11-27T09:20:00Z M"
comments 6 "2026-11-27T09:00:00Z A"
comments 8 "2026-11-27T09:00:00Z A" "2026-11-27T09:10:00Z M"
echo '[{"created_at":"2026-11-27T09:00:00Z","user":null},{"created_at":"2026-11-27T09:10:00Z","user":{"login":"Swissbite","id":1,"type":"User"}}]' \
    > "$fix/issues_7_comments_per_page_100"
run
grep -qx "external:m6:#5:2" <<< "$out" || fail "external: two outsider comments not counted"
grep -qx "external:m6:#6:1" <<< "$out" || fail "external: the outsider's issue body not counted"
grep -qx "external:m6:#7:2" <<< "$out" || fail "external: comment without user and fake maintainer login not counted"
! grep -q "external:m6:#8:" <<< "$out" || fail "external: reported for an issue without withheld items"

# Pull requests: only the agent's own (App, not a fork) are judged and counted
reset
echo "[$(pull 21 "$A" Swissbite/kestic), $(pull 22 "$A" mallory/kestic), $(pull 23 "$O" Swissbite/kestic),
    $(pull 24 "$M" Swissbite/kestic)]" > "$fix/pulls_base_m6_state_open_per_page_100"
echo "[$(checks 21 SUCCESS), $(checks 22 SUCCESS true), $(checks 23 FAILURE), $(checks 24 SUCCESS)]" > "$fix/pr-list-m6"
comments 21 "2026-11-27T09:00:00Z O"
echo '[{"submitted_at":"2026-11-27T09:00:00Z","user":'"$O"'},{"submitted_at":"2026-11-27T09:05:00Z","user":'"$M"'}]' \
    > "$fix/pulls_21_reviews_per_page_100"
echo '[{"created_at":"2026-11-27T09:00:00Z","user":'"$O"'}]' > "$fix/pulls_21_comments_per_page_100"
comments 22 "2026-11-27T09:00:00Z O"
run
grep -qx "pr:m6:ready:21:abcdef0" <<< "$out" || fail "pr: the agent's own green pull request not ready"
! grep -q "pr:m6:[a-z]*:22:" <<< "$out" || fail "pr: a fork's pull request reported"
! grep -q "pr:m6:[a-z]*:23:" <<< "$out" || fail "pr: an outsider's pull request in the repository reported"
! grep -q "pr:m6:[a-z]*:24:" <<< "$out" || fail "pr: a pull request the App did not open reported"
grep -qx "external:m6:#21:3" <<< "$out" || fail "external: withheld comment, review and review comment on the own pull request"
! grep -q "external:m6:#22:" <<< "$out" || fail "external: counted on a fork's pull request"

# A red check on the agent's own pull request is reported, on a fork's never
reset
echo "[$(pull 21 "$A" Swissbite/kestic), $(pull 22 "$A" mallory/kestic)]" > "$fix/pulls_base_m6_state_open_per_page_100"
echo "[$(checks 21 FAILURE), $(checks 22 FAILURE true)]" > "$fix/pr-list-m6"
run
grep -qx "pr:m6:red:21:abcdef0" <<< "$out" || fail "pr: the agent's own red pull request not reported"
! grep -q "pr:m6:red:22:" <<< "$out" || fail "pr: a fork's red pull request reported"

# A failing forge request is reported, not taken for «no news»
reset
echo "[$(issue 5 "$M" "$held" 2026-11-27T10:00:00Z)]" > "$fix/issues_milestone_3_state_open_per_page_100"
echo "fail" > "$fix/issues_5_comments_per_page_100.fail"
run
grep -q "^forge-error:m6:" <<< "$out" || fail "forge error: not reported"

# Without the trust file the loop does not start
sed -i "s|^LEITSTAND_TRUST_JQ=.*|LEITSTAND_TRUST_JQ=$tmp/missing.jq|" "$tmp/config/leitstand/test.env"
rc=0
"$here/watch.sh" test 1 > /dev/null 2>&1 || rc=$?
[[ "$rc" == 2 ]] || fail "missing trust file: exit $rc, expected 2"

if ((failures > 0)); then
    echo "test-watch: $failures failure(s)" >&2
    exit 1
fi
echo "test-watch: ok"
