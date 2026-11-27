#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause

# Tests the GitHub flags of a release: runs the plan step of release.yml with the planning scripts
# in a temp Git repository and a fake `gh`, then its job summary and the step that creates and
# publishes the release, and the release lookup of pages.yml. Checks that the release
# documentation states the same rule. Part of `just _static-checks`.
set -euo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd -- "$here/../.." && pwd)"
release_yml="$root/.github/workflows/release.yml"
pages_yml="$root/.github/workflows/pages.yml"

failures=0
check() {
    local expected="$1" actual="$2" what="$3"
    if [ "$expected" != "$actual" ]; then
        echo "FAIL: $what: expected '$expected', got '$actual'" >&2
        failures=$((failures + 1))
    fi
}

# `true` when text $2 contains the fixed string $1.
has() {
    if grep -qF -- "$1" <<< "$2"; then echo true; else echo false; fi
}

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
export HOME="$tmp/home" XDG_CONFIG_HOME="$tmp/home/.config" XDG_DATA_HOME="$tmp/home/.local/share" \
    XDG_CACHE_HOME="$tmp/home/.cache" XDG_STATE_HOME="$tmp/home/.local/state" XDG_RUNTIME_DIR="$tmp/runtime"
mkdir -p "$HOME" "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"

# The script of the `run: |` block of the step in workflow $1 that contains the line $2, without
# its indentation.
step_run() {
    awk -v pat="$2" '
        !found && index($0, pat) { found = 1 }
        found && !inside && /^ *run: \|$/ { inside = 1; indent = match($0, /[^ ]/) + 1; next }
        inside {
            if ($0 ~ /^ *$/) { print ""; next }
            if (match($0, /[^ ]/) <= indent) exit
            print substr($0, indent + 1)
        }
    ' "$1"
}

# Value of key $1 in the step output file $2.
output() {
    sed -n "s/^$1=//p" "$2"
}

# `<name> <expression>` for each entry of the `env:` mapping of the step in workflow $1 that
# contains the line $2.
step_env_mapping() {
    awk -v pat="$2" '
        !found && index($0, pat) { found = 1 }
        found && !inside && /^ *env:$/ { inside = 1; indent = match($0, /[^ ]/); next }
        inside {
            if (match($0, /[^ ]/) <= indent) exit
            line = substr($0, match($0, /[^ ]/))
            name = line; sub(/:.*/, "", name)
            value = line; sub(/^[^:]*: */, "", value)
            print name " " value
        }
    ' "$1"
}

# The output `<name>` of job prepare in release.yml: the key of the plan step it is mapped to.
prepare_output_key() {
    awk -v name="$1" '
        /^  prepare:$/ { job = 1; next }
        job && /^  [a-z]/ { exit }
        job && /^    outputs:$/ { inside = 1; next }
        inside && /^    [a-z]/ { exit }
        inside && $1 == name ":" && $2 == "${{" && $3 ~ /^steps\.plan\.outputs\./ && $4 == "}}" {
            key = $3; sub(/^steps\.plan\.outputs\./, "", key); print key; exit
        }
    ' "$release_yml"
}

# `NAME=value` lines for the env of the step in release.yml that contains the line $1, as GitHub
# would set it from the outputs of the last plan; tokens become `fake`. Fails on any other
# expression.
step_env() {
    local name value field key
    while read -r name value; do
        case "$value" in
            '${{ needs.prepare.outputs.'*' }}')
                field="${value#'${{ needs.prepare.outputs.'}"
                field="${field%' }}'}"
                key="$(prepare_output_key "$field")"
                [ -n "$key" ] || { echo "no output $field in job prepare" >&2; return 1; }
                echo "$name=$(output "$key" "$tmp/plan/out")"
                ;;
            '${{ github.token }}' | '${{ steps.app-token.outputs.token }}') echo "$name=fake" ;;
            *) echo "unknown env expression $value" >&2; return 1 ;;
        esac
    done < <(step_env_mapping "$release_yml" "$1")
}

# A fake `gh` for the planning script, the job publish and pages.yml. Every call is logged to
# $FAKE_GH_LOG; releases are kept in $FAKE_SERVER.
mkdir -p "$tmp/bin"
cat > "$tmp/bin/gh" << 'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$FAKE_GH_LOG"
jq_expr=""
args=()
while [ "$#" -gt 0 ]; do
    case "$1" in
        --jq) jq_expr="$2"; shift 2 ;;
        *) args+=("$1"); shift ;;
    esac
done
reply() {
    if [ -n "$jq_expr" ]; then jq -r "$jq_expr" <<< "$1"; else printf '%s\n' "$1"; fi
}
case "${args[0]} ${args[1]}" in
    "api repos/owner/kestic/milestones?state=all&per_page=100")
        reply '[{"title":"M6 Onboarding and security","number":6,"html_url":"https://example.invalid/milestone/6"}]' ;;
    "api repos/owner/kestic/issues?milestone=6&state=all&per_page=100") reply '[]' ;;
    "api repos/owner/kestic/commits/"*/pulls) reply '[]' ;;
    "api repos/owner/kestic/git/ref/tags/"*) exit 1 ;;
    "api -X")
        [ "${args[2]} ${args[3]}" = "POST repos/owner/kestic/git/refs" ] || exit 2 ;;
    "api repos/owner/kestic/releases?per_page=100") reply '[]' ;;
    "release create")
        dir="$FAKE_SERVER/${args[2]}"
        mkdir -p "$dir"
        # The files come right after the tag, before the first option.
        for arg in "${args[@]:3}"; do
            [[ "$arg" != -* ]] || break
            cp -- "$arg" "$dir/"
        done
        ;;
    "release download")
        for ((i = 0; i < ${#args[@]}; i++)); do
            if [ "${args[i]}" = --dir ]; then cp -a "$FAKE_SERVER/${args[2]}/." "${args[i + 1]}/"; fi
        done
        ;;
    "release edit") ;;
    "release view")
        if [ -n "$jq_expr" ] && [ "$jq_expr" = .url ]; then
            echo "https://example.invalid/releases/${args[2]}"
        else
            reply "$(cat "$FAKE_SERVER/view-${args[2]}.json")"
        fi
        ;;
    "release list") reply "$(cat "$FAKE_SERVER/list.json")" ;;
    *) echo "fake gh: unexpected call: ${args[*]}" >&2; exit 2 ;;
esac
EOF
chmod +x "$tmp/bin/gh"
export PATH="$tmp/bin:$PATH" GITHUB_REPOSITORY=owner/kestic

# Temp repository: v0.5.0, m6-start, the branch build v0.6.0-m6.1, then the head with the
# highlights of v0.6.0 and v1.0.0. origin/main holds the planning scripts as they are now.
repo="$tmp/repo"
git_t() {
    (cd "$repo" && GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 GIT_AUTHOR_NAME=t \
        GIT_AUTHOR_EMAIL=t@example.invalid GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t@example.invalid \
        git "$@")
}
mkdir -p "$repo"
git_t init -q
git_t commit -q --allow-empty -m one
git_t tag v0.5.0
git_t commit -q --allow-empty -m two
git_t tag m6-start
git_t commit -q --allow-empty -m three
git_t tag v0.6.0-m6.1
mkdir -p "$repo/.github/scripts" "$repo/docs/releases"
cp "$root/.github/scripts/release-notes.sh" "$root/.github/scripts/release-version.sh" "$repo/.github/scripts/"
for tag in v0.6.0 v1.0.0; do
    printf -- '---\nmilestones:\n  - M6 Onboarding and security\n---\n\nHighlights of %s.\n' "$tag" \
        > "$repo/docs/releases/$tag.md"
done
git_t add -A
git_t commit -q -m head
git_t update-ref refs/remotes/origin/main HEAD
head="$(git_t rev-parse HEAD)"

plan_script="$(step_run "$release_yml" 'id: plan')"
check true "$([ -n "$plan_script" ] && echo true || echo false)" "release.yml has the plan step"

# Runs the plan step for event $1 (push or workflow_dispatch) with ref name $2 and bump $3 in a
# dry run. Leaves the step outputs in $tmp/plan/out and its log in $tmp/plan/log; prints `pass`
# or `fail`.
run_plan() {
    local event="$1" ref_name="$2" bump="$3" ref="refs/heads/$2"
    rm -rf "$tmp/plan" "$tmp/runner"
    mkdir -p "$tmp/plan" "$tmp/runner"
    : > "$tmp/plan/out"
    if (cd "$repo" && GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 GH_TOKEN=fake \
        EVENT_NAME="$event" REF="$ref" REF_NAME="$ref_name" BUMP="$bump" DRY_RUN=true \
        RUNNER_TEMP="$tmp/runner" GITHUB_SHA="$head" GITHUB_OUTPUT="$tmp/plan/out" \
        FAKE_GH_LOG="$tmp/plan/gh.log" FAKE_SERVER="$tmp/server" \
        bash -c "set -euo pipefail; $plan_script") > "$tmp/plan/log" 2>&1; then
        echo pass
    else
        echo fail
    fi
}

# Plan, job summary and publishing for one row of the table: event, ref name, bump, expected tag,
# expected pre-release flag, expected latest flag, AC.
summary_script="$(step_run "$release_yml" '- name: Job summary')"
create_script="$(step_run "$release_yml" '- name: Create tag and release')"
check true "$([ -n "$summary_script" ] && [ -n "$create_script" ] && echo true || echo false)" \
    "release.yml has the steps Job summary and Create tag and release"

# Runs the step Job summary with the outputs of the last plan; prints the summary.
run_summary() {
    local work="$tmp/summary"
    rm -rf "$work"
    mkdir -p "$work/notes" "$work/assets"
    echo "Notes." > "$work/notes/notes.md"
    echo deb > "$work/assets/kestic.deb"
    (cd "$work/assets" && sha256sum kestic.deb > SHA256SUMS)
    local -a vars
    mapfile -t vars < <(step_env '- name: Job summary')
    (cd "$work" && env "${vars[@]}" GITHUB_STEP_SUMMARY="$work/summary.md" \
        bash -c "set -euo pipefail; $summary_script") > /dev/null 2>&1 || true
    cat "$work/summary.md" 2> /dev/null || true
}

# Runs the step Create tag and release with the outputs of the last plan and a fake `gh`; prints
# the `gh release create` and `gh release edit` calls, one per line.
run_create() {
    local work="$tmp/create"
    local -a vars
    rm -rf "$work" "$tmp/server"
    mkdir -p "$work/notes" "$work/assets" "$work/runner" "$tmp/server"
    echo "Notes." > "$work/notes/notes.md"
    echo deb > "$work/assets/kestic.deb"
    (cd "$work/assets" && sha256sum kestic.deb > SHA256SUMS)
    mapfile -t vars < <(step_env '- name: Create tag and release')
    (cd "$work" && env "${vars[@]}" RUNNER_TEMP="$work/runner" FAKE_GH_LOG="$work/gh.log" \
        FAKE_SERVER="$tmp/server" bash -c "set -euo pipefail; $create_script") > /dev/null 2>&1 ||
        echo "step failed"
    grep -E '^release (create|edit) ' "$work/gh.log" 2> /dev/null || true
}

# `true` when the call line $2 has the whole argument $1.
has_arg() {
    if grep -qE -- "(^| )$1( |$)" <<< "$2"; then echo true; else echo false; fi
}

plan_row() {
    local event="$1" ref_name="$2" bump="$3" tag="$4" prerelease="$5" latest="$6" ac="$7"
    local what="$ac: $tag" summary calls create edit
    check pass "$(run_plan "$event" "$ref_name" "$bump")" "$what: the plan step succeeds"
    check "$tag" "$(output tag "$tmp/plan/out")" "$what: planned tag"
    if [ "$prerelease" = false ]; then
        check false "$(output prerelease "$tmp/plan/out")" "$ac (a): $tag is no pre-release"
        check true "$(output latest "$tmp/plan/out")" "$ac (b): $tag is marked latest"
    else
        check true "$(output prerelease "$tmp/plan/out")" "$ac (a): $tag is a pre-release"
        check false "$(output latest "$tmp/plan/out")" "$ac (b): $tag is never marked latest"
    fi
    check false "$(has 'release-notes.sh on main plans' "$(cat "$tmp/plan/log")")" \
        "$what: the planning script agrees with the tag"

    summary="$(run_summary)"
    if [ "$event" = workflow_dispatch ]; then
        check true "$(has '# Dry run:' "$summary")" "AC-M6-16-3 (a): $tag: the summary is a dry run's"
    fi
    check true "$(has "- Pre-release: $prerelease" "$summary")" \
        "AC-M6-16-3 (a): $tag: the summary shows the pre-release flag"
    check true "$(has "- Latest: $latest" "$summary")" "AC-M6-16-3 (a): $tag: the summary shows the latest flag"

    calls="$(run_create)"
    check false "$(has 'step failed' "$calls")" "$what: the step Create tag and release succeeds"
    create="$(grep '^release create ' <<< "$calls" || true)"
    edit="$(grep '^release edit ' <<< "$calls" || true)"
    check true "$(has_arg --draft "$create")" "$what: created as a draft"
    check "$prerelease" "$(has_arg --prerelease "$create")" "$ac (a): $tag: --prerelease on create"
    check false "$(grep -qE -- '(^| )--latest' <<< "$create" && echo true || echo false)" \
        "$ac (b): $tag: the draft is not marked latest"
    check true "$(has_arg --draft=false "$edit")" "$what: the draft is published"
    check "$latest" "$(has_arg --latest "$edit")" "$ac (b): $tag: --latest on publishing"
    check "$latest" "$(grep -qE -- '(^| )--latest' <<< "$edit" && echo true || echo false)" \
        "$ac (b): $tag: make_latest is passed only for a normal release"
}

# AC-M6-16-1, AC-M6-16-2: the table of tags.
plan_row workflow_dispatch main minor v0.6.0 false true AC-M6-16-1
plan_row workflow_dispatch main major v1.0.0 false true AC-M6-16-1
plan_row workflow_dispatch main prerelease v0.6.0-rc.1 true false AC-M6-16-2
plan_row workflow_dispatch main preminor v0.7.0-rc.1 true false AC-M6-16-2
plan_row push m6 "" v0.6.0-m6.2 true false AC-M6-16-2

# AC-M6-16-1, AC-M6-16-2: the flags come from the tag even when the planning script on main
# still marks every 0.x version as a pre-release; it only gets a warning.
sed -i 's/^if tag_is_prerelease "\$tag"; then prerelease=true; fi$/if [ "$(tag_major "$tag")" -eq 0 ] || tag_is_prerelease "$tag"; then prerelease=true; fi/' \
    "$repo/.github/scripts/release-notes.sh"
check true "$(has '"$(tag_major "$tag")" -eq 0 ] || tag_is_prerelease' "$(cat "$repo/.github/scripts/release-notes.sh")")" \
    "the old planning script is set up"
git_t commit -q -am "old planning script"
git_t update-ref refs/remotes/origin/main HEAD
head="$(git_t rev-parse HEAD)"
check pass "$(run_plan workflow_dispatch main minor)" "AC-M6-16-1: plan with the old script"
check v0.6.0 "$(output tag "$tmp/plan/out")" "AC-M6-16-1: plan with the old script: tag"
check false "$(output prerelease "$tmp/plan/out")" "AC-M6-16-1 (a): the old script cannot make v0.6.0 a pre-release"
check true "$(output latest "$tmp/plan/out")" "AC-M6-16-1 (b): v0.6.0 is latest with the old script"
check true "$(has '::warning::release-notes.sh on main plans prerelease=true for v0.6.0' "$(cat "$tmp/plan/log")")" \
    "AC-M6-16-1: a differing planning script gets a warning"
git_t reset -q --hard HEAD~1
git_t update-ref refs/remotes/origin/main HEAD
head="$(git_t rev-parse HEAD)"

# A tag the rule does not know stops the plan.
sed -i 's/^echo "tag=\$tag"$/echo "tag=${tag}-beta.1"/' "$repo/.github/scripts/release-notes.sh"
git_t commit -q -am "unknown suffix"
git_t update-ref refs/remotes/origin/main HEAD
head="$(git_t rev-parse HEAD)"
check fail "$(run_plan workflow_dispatch main minor)" "AC-M6-16-2: a tag with an unknown suffix stops the plan"
check "" "$(output tag "$tmp/plan/out")" "AC-M6-16-2: a tag with an unknown suffix gives no output"
git_t reset -q --hard HEAD~1
git_t update-ref refs/remotes/origin/main HEAD
head="$(git_t rev-parse HEAD)"

# AC-M6-16-3 (b): pages.yml takes the highest published tag without suffix, whichever release
# GitHub marks latest. Prints its outputs `tag=` and `deb-url=`, or `step failed`.
pages_script="$(step_run "$pages_yml" '- name: Find the newest release')"
check true "$([ -n "$pages_script" ] && echo true || echo false)" "pages.yml has the step Find the newest release"
run_pages() {
    local work="$tmp/pages" tag
    rm -rf "$work" "$tmp/server"
    mkdir -p "$work" "$tmp/server"
    printf '%s\n' "$1" > "$tmp/server/list.json"
    for tag in $(jq -r '.[].tagName' <<< "$1"); do
        printf '{"assets":[{"name":"kestic_%s-1_amd64.deb","url":"https://example.invalid/%s.deb"}]}\n' \
            "${tag#v}" "$tag" > "$tmp/server/view-$tag.json"
    done
    if (cd "$work" && GH_TOKEN=fake GITHUB_OUTPUT="$work/out" FAKE_GH_LOG="$work/gh.log" \
        FAKE_SERVER="$tmp/server" bash -c "set -euo pipefail; $pages_script") > /dev/null 2>&1; then
        cat "$work/out"
    else
        echo "step failed"
    fi
}
expected="$(printf 'tag=%s\ndeb-url=%s\n' v0.6.0 https://example.invalid/v0.6.0.deb)"
check "$expected" "$(run_pages '[
    {"tagName":"v0.7.0-m7.1","isDraft":false,"isLatest":false},
    {"tagName":"v0.6.0","isDraft":false,"isLatest":true},
    {"tagName":"v0.6.0-rc.1","isDraft":false,"isLatest":false},
    {"tagName":"v0.5.0","isDraft":false,"isLatest":false}]')" \
    "AC-M6-16-3 (b): the release marked latest is the highest without suffix"
check "$expected" "$(run_pages '[
    {"tagName":"v0.6.0","isDraft":false,"isLatest":false},
    {"tagName":"v0.5.0","isDraft":false,"isLatest":true},
    {"tagName":"v0.4.0","isDraft":false,"isLatest":false}]')" \
    "AC-M6-16-3 (b): a lower release marked latest does not hide the highest"
expected="$(printf 'tag=%s\ndeb-url=%s\n' v0.10.0 https://example.invalid/v0.10.0.deb)"
check "$expected" "$(run_pages '[
    {"tagName":"v0.11.0","isDraft":true,"isLatest":false},
    {"tagName":"v0.10.0","isDraft":false,"isLatest":true},
    {"tagName":"v0.9.0","isDraft":false,"isLatest":false}]')" \
    "AC-M6-16-3 (b): numeric order, drafts left out"

# AC-M6-16-4 (a): README and architecture.md §Release state the same rule, and nothing says that
# every 0.x version is a pre-release.
readme="$(cat "$root/docs/releases/README.md")"
architecture="$(cat "$root/docs/spec/architecture.md")"
release_section="$(awk '/^## Release$/ { inside = 1; next } /^## / { inside = 0 } inside' <<< "$architecture")"
for fact in '`v0.6.0`, later `v1.x`' '`v0.<n>.0-m<n>.<k>`' '`-rc.<k>`' 'normal GitHub release marked «Latest»' \
    'are pre-releases, never «Latest»'; do
    check true "$(has "$fact" "$readme")" "AC-M6-16-4 (a): docs/releases/README.md states: $fact"
done
for fact in '`v0.6.0`, später `v1.x`' '`v0.<n>.0-m<n>.<k>`' '`-rc.<k>`' 'normales GitHub-Release' \
    'trägt die Markierung «Latest»' 'sind Pre-Releases und nie «Latest»'; do
    check true "$(has "$fact" "$release_section")" "AC-M6-16-4 (a): architecture.md §Release states: $fact"
done
stale='(every|all|jede[s]?|alle)[^.]*0\.x[^.]*(pre-release|Pre-Release)|(Pre-Release-Flag|pre-release flag)[^.]*0\.x'
for file in docs/releases/README.md docs/spec/architecture.md .github/workflows/release.yml \
    .github/workflows/pages.yml .github/scripts/release-notes.sh; do
    check "" "$(grep -niE -- "$stale" "$root/$file" || true)" \
        "AC-M6-16-4 (a): $file does not say that every 0.x version is a pre-release"
done

check true "$(grep -qF '.github/scripts/test-release-flags.sh' "$root/justfile" && echo true || echo false)" \
    "this test runs in just _static-checks"

if [ "$failures" -ne 0 ]; then
    exit 1
fi
echo "release flags: all checks passed"
