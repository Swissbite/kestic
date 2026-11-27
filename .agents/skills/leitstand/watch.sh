#!/usr/bin/env bash
#
# Leitstand watch loop: polls every 120 s and exits with the new events as soon as something
# needs the Leitstand. Seen events are kept in a state file, so a restarted loop does not report
# them again. Usage: watch.sh <project> [seconds]
#
# Settings come from ${XDG_CONFIG_HOME:-~/.config}/leitstand/<project>.env (see
# instance.env.example). Commands in it may contain {agent}, which is replaced by the agent name.
#
# Issue and pull request text is never read here. Whose comments count decides the jq file
# LEITSTAND_TRUST_JQ (functions trust_class and agent_pull, by numeric user ID); only the
# agent's own pull requests (opened by its App in this repository, never from a fork) are judged.
set -u

project=${1:?usage: watch.sh <project> [seconds]}
env_file="${XDG_CONFIG_HOME:-$HOME/.config}/leitstand/$project.env"
[[ -r "$env_file" ]] || { echo "missing $env_file" >&2; exit 2; }
# shellcheck source=/dev/null
. "$env_file"

: "${LEITSTAND_REPO:?}" "${LEITSTAND_AGENTS:?}" "${LEITSTAND_MAINTAINER:?}" "${LEITSTAND_TRUST_JQ:?}"
: "${LEITSTAND_AGENT_STATE:=}" "${LEITSTAND_AGENT_PANE:=}"
: "${LEITSTAND_CHECKS:=verify CodeQL}" "${LEITSTAND_MAIN_PRS:=}"
: "${LEITSTAND_SUBAGENT_MAX_MIN:=25}" "${LEITSTAND_DISK_MIN_GB:=25}" "${LEITSTAND_IDLE_MAX_MIN:=30}"
# Console patterns; the defaults match Claude Code. An empty pattern disables its check.
: "${LEITSTAND_PROMPT_PATTERN=Do you want to proceed}"
: "${LEITSTAND_LIMIT_PATTERN=hit your .*limit}"
: "${LEITSTAND_LIMIT_RESET_PATTERN=resets [0-9:apm]* ([^)]*)}"
: "${LEITSTAND_SUBAGENT_PATTERN=◯ .* [0-9]+m [0-9]+s}"

cd "$LEITSTAND_REPO" || exit 1
[[ -r "$LEITSTAND_TRUST_JQ" ]] || { echo "cannot read LEITSTAND_TRUST_JQ=$LEITSTAND_TRUST_JQ" >&2; exit 2; }
TRUST=$(<"$LEITSTAND_TRUST_JQ")
state_dir="${XDG_STATE_HOME:-$HOME/.local/state}/leitstand/$project"
mkdir -p "$state_dir"
# One loop per project: two loops share the seen file, and the second would swallow events the
# first never reports. The lock is released when this process exits.
exec 9>>"$state_dir/lock"
if ! flock -n 9; then
    echo "another watch loop runs for $project (pid $(<"$state_dir/pid")); stop it or ask its session first" >&2
    exit 3
fi
echo $$ >"$state_dir/pid"
SEEN="$state_dir/seen"
touch "$SEEN"
end=$(( $(date +%s) + ${2:-36000} ))
checks_json=$(printf '%s\n' $LEITSTAND_CHECKS | jq -R . | jq -sc .)

# Runs command template $1 for agent $2.
for_agent() {
    [[ -n "$1" ]] || return 0
    bash -c "${1//\{agent\}/$2}" 2>/dev/null 9>&-
}

# A pull request as `red:<n>:<sha>` or `ready:<n>:<sha>`, judged by the required checks only.
pr_filter="$checks_json as \$checks | "'. as $p | ($p.statusCheckRollup | map(select(.name as $n | $checks | index($n)))) as $c
    | if ($c | any(.conclusion=="FAILURE")) then "red:\($p.number):\($p.headRefOid[0:7])"
      elif ($p.isDraft|not) and ($c|length) == ($checks|length) and ($c|all(.conclusion=="SUCCESS"))
      then "ready:\($p.number):\($p.headRefOid[0:7])"
      else empty end'

# Remembers since when agent $1 has been idle ($2 is its state) and reports `idle:<agent>:<since>`
# once it stays idle longer than LEITSTAND_IDLE_MAX_MIN. An idle agent with open work is stuck;
# an idle agent with nothing left to do is reported once per idle period.
idle_since() {
    local file="$state_dir/idle-$1" now since
    if [[ "$2" != idle ]]; then
        rm -f -- "$file"
        return 0
    fi
    now=$(date +%s)
    [[ -s "$file" ]] || echo "$now" >"$file"
    since=$(<"$file")
    (( now - since >= LEITSTAND_IDLE_MAX_MIN * 60 )) && echo "idle:$1:$(date -d "@$since" +%Y%m%d%H%M)"
    return 0
}

# Counts per issue or pull request, recomputed only when its updated_at changes:
# `<updated_at> <withheld> <class of the last comment> <time of the last comment>`.
# withheld = items by anyone the trust file does not name (the body of an issue opened by
# someone else counts unless it carries the label adopted; the thread reader decides whether
# that adoption holds).
cached() {
    local file="$state_dir/count-$1" upd="$2"
    shift 2
    if [[ -s "$file" ]] && [[ "$(cut -d' ' -f1 "$file")" == "$upd" ]]; then
        cat "$file"
        return 0
    fi
    local lines
    lines=$("$@") || return 1
    printf '%s %s\n' "$upd" "$lines" | tee "$file"
}

# `<withheld> <last class> <last time>` from lines `<time> <class>` on stdin, plus $1 more withheld.
summarise() {
    sort | awk -v extra="$1" 'NF >= 2 { n += ($2 == "withheld"); last = $2; t = $1 }
        END { printf "%d %s %s\n", n + extra, (last == "" ? "none" : last), (t == "" ? "-" : t) }'
}

issue_counts() {
    local n="$1" body="$2" lines
    lines=$(gh api --paginate "repos/{owner}/{repo}/issues/$n/comments?per_page=100" \
        --jq "$TRUST .[] | \"\(.created_at) \(.user | trust_class)\"" 2>/dev/null) || return 1
    summarise "$body" <<<"$lines"
}

pr_counts() {
    local n="$1" lines more
    lines=$(gh api --paginate "repos/{owner}/{repo}/issues/$n/comments?per_page=100" \
        --jq "$TRUST .[] | \"\(.created_at) \(.user | trust_class)\"" 2>/dev/null) || return 1
    for kind in reviews comments; do
        more=$(gh api --paginate "repos/{owner}/{repo}/pulls/$n/$kind?per_page=100" \
            --jq "$TRUST .[] | \"\(.submitted_at // .created_at) \(.user | trust_class)\"" 2>/dev/null) || return 1
        lines+=$'\n'"$more"
    done
    summarise 0 <<<"$lines"
}

# question: and external: events of the open issues of milestone $2 (branch $1).
milestone_events() {
    local branch="$1" number n upd author held adopted body withheld last t
    number=$(gh api --paginate "repos/{owner}/{repo}/milestones?state=all&per_page=100" \
        --jq '.[] | "\(.number)\t\(.title)"' 2>/dev/null | awk -F'\t' -v t="$2" '$2 == t { print $1; exit }')
    [[ "$number" =~ ^[0-9]+$ ]] || return 0
    gh api --paginate "repos/{owner}/{repo}/issues?milestone=$number&state=open&per_page=100" \
        --jq "$TRUST"' .[] | select(has("pull_request") | not)
            | "\(.number) \(.updated_at) \(.user | trust_class) \(any(.labels[]; .name == "needs-maintainer")) \(any(.labels[]; .name == "adopted"))"' \
        2>/dev/null | while read -r n upd author held adopted; do
        [[ "$n" =~ ^[0-9]+$ ]] || continue
        body=0
        [[ "$author" == withheld && "$adopted" != true ]] && body=1
        read -r _ withheld last t < <(cached "issue-$n" "$upd" issue_counts "$n" "$body") || continue
        # Only a question the agent asked last waits for the maintainer.
        [[ "$held" == true && "$last" == agent ]] && echo "question:$branch:#$n:$t"
        (( withheld > 0 )) && echo "external:$branch:#$n:$withheld"
    done
}

# pr: and external: events of the agent's own open pull requests into branch $1.
pr_events() {
    local branch="$1" own n upd withheld
    own=$(gh api --paginate "repos/{owner}/{repo}/pulls?base=$branch&state=open&per_page=100" \
        --jq "$TRUST .[] | select(agent_pull) | \"\(.number) \(.updated_at)\"" 2>/dev/null) || return 0
    [[ -n "$own" ]] || return 0
    while read -r n upd; do
        [[ "$n" =~ ^[0-9]+$ ]] || continue
        read -r _ withheld _ < <(cached "pr-$n" "$upd" pr_counts "$n") || continue
        (( withheld > 0 )) && echo "external:$branch:#$n:$withheld"
    done <<<"$own"
    gh pr list --base "$branch" --state open --json number,isCrossRepository,isDraft,headRefOid,statusCheckRollup \
        --jq ".[] | select(.isCrossRepository == false) | $pr_filter" 2>/dev/null |
        awk -F: -v own="$(cut -d' ' -f1 <<<"$own" | tr '\n' ' ')" \
            'BEGIN { split(own, a, " "); for (i in a) mine[a[i]] = 1 } ($2 in mine)' | sed "s/^/pr:$branch:/"
}

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
            idle_since "$agent" "${st%% *}"
        fi
        pane=$(for_agent "$LEITSTAND_AGENT_PANE" "$agent" | grep -v '^[[:space:]]*$' | tail -n 25)
        [[ -n "$LEITSTAND_PROMPT_PATTERN" ]] && grep -q -- "$LEITSTAND_PROMPT_PATTERN" <<<"$pane" \
            && echo "prompt:$agent:$(date +%H)"
        [[ -n "$LEITSTAND_LIMIT_PATTERN" ]] && grep -q -- "$LEITSTAND_LIMIT_PATTERN" <<<"$pane" \
            && echo "limit:$agent:$(grep -o -- "${LEITSTAND_LIMIT_RESET_PATTERN:-.^}" <<<"$pane" | tail -1 | tr ' ' _)"
        [[ -n "$LEITSTAND_SUBAGENT_PATTERN" ]] && grep -E -- "$LEITSTAND_SUBAGENT_PATTERN" <<<"$pane" | while read -r line; do
            m=$(grep -oE '[0-9]+m [0-9]+s' <<<"$line" | head -1 | cut -dm -f1)
            (( m >= LEITSTAND_SUBAGENT_MAX_MIN )) \
                && echo "subagent:$agent:over-${LEITSTAND_SUBAGENT_MAX_MIN}min:$(date +%Y%m%d%H)"
        done
        milestone_events "$branch" "$milestone"
        pr_events "$branch"
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
    # Children never inherit the lock, so a stopped loop does not leave it held by a straggler.
    new=$(events 9>&- | sort -u | grep -vxF -f "$SEEN" || true)
    if [[ -n "$new" ]]; then
        printf '%s\n' "$new" >>"$SEEN"
        echo "$(date +%H:%M) events:"
        printf '%s\n' "$new"
        exit 0
    fi
    sleep 120 9>&-
done
echo "$(date +%H:%M) no events"
