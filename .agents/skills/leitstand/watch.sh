#!/usr/bin/env bash
#
# Leitstand watch loop: polls every 120 s and exits with the new events as soon as something
# needs the Leitstand. Seen events are kept in a state file, so a restarted loop does not report
# them again. Usage: watch.sh <project> [seconds]
#
# Settings come from ${XDG_CONFIG_HOME:-~/.config}/leitstand/<project>.env (see
# instance.env.example). Commands in it may contain {agent}, which is replaced by the agent name.
set -u

project=${1:?usage: watch.sh <project> [seconds]}
env_file="${XDG_CONFIG_HOME:-$HOME/.config}/leitstand/$project.env"
[[ -r "$env_file" ]] || { echo "missing $env_file" >&2; exit 2; }
# shellcheck source=/dev/null
. "$env_file"

: "${LEITSTAND_REPO:?}" "${LEITSTAND_AGENTS:?}" "${LEITSTAND_MAINTAINER:?}"
: "${LEITSTAND_AGENT_STATE:=}" "${LEITSTAND_AGENT_PANE:=}"
: "${LEITSTAND_CHECKS:=verify CodeQL}" "${LEITSTAND_MAIN_PRS:=}"
: "${LEITSTAND_SUBAGENT_MAX_MIN:=25}" "${LEITSTAND_DISK_MIN_GB:=25}"

cd "$LEITSTAND_REPO" || exit 1
state_dir="${XDG_STATE_HOME:-$HOME/.local/state}/leitstand/$project"
mkdir -p "$state_dir"
SEEN="$state_dir/seen"
touch "$SEEN"
end=$(( $(date +%s) + ${2:-36000} ))
checks_json=$(printf '%s\n' $LEITSTAND_CHECKS | jq -R . | jq -sc .)

# Runs command template $1 for agent $2.
for_agent() {
    [[ -n "$1" ]] || return 0
    bash -c "${1//\{agent\}/$2}" 2>/dev/null
}

# A pull request as `red:<n>:<sha>` or `ready:<n>:<sha>`, judged by the required checks only.
pr_filter="$checks_json as \$checks | "'. as $p | ($p.statusCheckRollup | map(select(.name as $n | $checks | index($n)))) as $c
    | if ($c | any(.conclusion=="FAILURE")) then "red:\($p.number):\($p.headRefOid[0:7])"
      elif ($p.isDraft|not) and ($c|length) == ($checks|length) and ($c|all(.conclusion=="SUCCESS"))
      then "ready:\($p.number):\($p.headRefOid[0:7])"
      else empty end'

events() {
    local entry agent branch milestone st pane
    IFS=';' read -ra entries <<<"$LEITSTAND_AGENTS"
    for entry in "${entries[@]}"; do
        IFS='|' read -r agent branch milestone <<<"$entry"
        st=$(for_agent "$LEITSTAND_AGENT_STATE" "$agent" | head -1)
        if [[ -n "$LEITSTAND_AGENT_STATE" ]]; then
            case "${st%% *}" in
                busy|idle) ;;
                *) echo "state:$agent:${st// /_}" ;;
            esac
        fi
        pane=$(for_agent "$LEITSTAND_AGENT_PANE" "$agent" | grep -v '^[[:space:]]*$' | tail -n 25)
        grep -q 'Do you want to proceed' <<<"$pane" && echo "prompt:$agent:$(date +%H)"
        grep -q 'hit your .*limit' <<<"$pane" \
            && echo "limit:$agent:$(grep -o 'resets [0-9:apm]* ([A-Z]*)' <<<"$pane" | tail -1 | tr ' ' _)"
        grep -E '◯ .* [0-9]+m [0-9]+s' <<<"$pane" | while read -r line; do
            m=$(grep -oE '[0-9]+m [0-9]+s' <<<"$line" | head -1 | cut -dm -f1)
            (( m >= LEITSTAND_SUBAGENT_MAX_MIN )) \
                && echo "subagent:$agent:over-${LEITSTAND_SUBAGENT_MAX_MIN}min:$(date +%Y%m%d%H)"
        done
        gh issue list --milestone "$milestone" --label needs-maintainer --state open --json number \
            --jq '.[].number' 2>/dev/null | while read -r n; do
            read -r held author upd < <(gh issue view "$n" --json labels,comments \
                --jq '"\(any(.labels[]; .name=="needs-maintainer")) \(.comments[-1].author.login) \(.comments[-1].createdAt)"' 2>/dev/null)
            # The label list lags; skip an issue whose label is gone or whose last comment is the maintainer's.
            [[ "$held" == true && "$author" != "$LEITSTAND_MAINTAINER" ]] && echo "question:$branch:#$n:$upd"
        done
        gh pr list --base "$branch" --state open --json number,isDraft,headRefOid,statusCheckRollup \
            --jq ".[] | $pr_filter" 2>/dev/null | sed "s/^/pr:$branch:/"
    done
    for n in $LEITSTAND_MAIN_PRS; do
        gh pr view "$n" --json number,state,isDraft,headRefOid,statusCheckRollup \
            --jq "select(.state==\"OPEN\") | $pr_filter" 2>/dev/null | sed "s/^/pr:main:/"
    done
    free=$(df -BG --output=avail / | tail -1 | tr -dc 0-9)
    for level in "$LEITSTAND_DISK_MIN_GB" 15 8; do
        (( free < level )) && echo "disk:below-${level}G"
    done
    true
}

while (( $(date +%s) < end )); do
    new=$(events | sort -u | grep -vxF -f "$SEEN" || true)
    if [[ -n "$new" ]]; then
        printf '%s\n' "$new" >>"$SEEN"
        echo "$(date +%H:%M) events:"
        printf '%s\n' "$new"
        exit 0
    fi
    sleep 120
done
echo "$(date +%H:%M) no events"
