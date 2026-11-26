#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause

# Which jobs of the Verify workflow (verify.yml) a run needs, and whether their results add up to a
# green `verify`. Pull requests that change only documentation or the site skip the jobs that build
# and test; pull requests that change only the specification run just the tests that read it.
# Every other event (a push to main or an integration branch, a manual or scheduled run) runs
# everything, so screenshots and the .deb always come from a full run.
#
#   ci-scope.sh outputs <event> [<base> <head>]
#       Lines `scope=…` and `<job>=true|false` for $GITHUB_OUTPUT. For a pull request the changed
#       files are `git diff <base> <head>`; the default HEAD^1 HEAD is the merge commit that
#       actions/checkout checks out for a pull request (needs fetch-depth 2).
#   ci-scope.sh verdict <scope>
#       Reads `<job> <result>` lines (the `needs` of the `verify` job) and fails unless every job
#       of the scope succeeded and every other job succeeded or was skipped.
#   ci-scope.sh spec-tests
#       `<package> <test binary>` of each test that reads files under docs/spec/.
#
# Sourced by test-ci-scope.sh, which tests the functions below.

set -euo pipefail

# Jobs that run in every scope: `checks` covers formatting, file sizes, licenses and REUSE. Any
# other job of `verify` that is not gated below counts as one of them.
readonly CI_ALWAYS_JOBS="ci-image scope checks"
# Jobs the scope switches on or off.
readonly CI_GATED_JOBS="clippy build release test deb-check spec-tests"

# Tests that read the specification. test-ci-scope.sh fails when a test names a path under
# docs/spec/ and is missing here.
readonly CI_SPEC_TESTS="kestic-core m0_s03_cxx_qt_pin
kestic-core m4_s00_code_structure"

# Scope of a run: `full`, `spec` or `docs`. $1 is the event name; the changed files come one per
# line on stdin and count only for a pull request. An empty list counts as a full run.
ci_scope() {
    local event="$1" file any=false spec=false
    if [ "$event" != pull_request ]; then
        echo full
        return
    fi
    while IFS= read -r file; do
        [ -n "$file" ] || continue
        any=true
        case "$file" in
            docs/spec/*) spec=true ;;
            docs/* | site/* | *.md) ;;
            *)
                echo full
                return
                ;;
        esac
    done
    if [ "$any" = false ]; then
        echo full
    elif [ "$spec" = true ]; then
        echo spec
    else
        echo docs
    fi
}

# The gated jobs that run in scope $1.
ci_jobs() {
    case "$1" in
        full) echo "clippy build release test deb-check" ;;
        spec) echo "spec-tests" ;;
        docs) echo "" ;;
        *)
            echo "unknown scope '$1'" >&2
            return 1
            ;;
    esac
}

# Whether job $1 runs in scope $2: ungated jobs always do.
ci_job_runs() {
    local job="$1" scope="$2" runs
    runs="$(ci_jobs "$scope")" || return 2
    if [[ " $CI_GATED_JOBS " != *" $job "* ]]; then
        return 0
    fi
    [[ " $runs " == *" $job "* ]]
}

# Verdict of the `verify` job in scope $1 over `<job> <result>` lines on stdin. A job of the scope
# must report `success`; a job the scope switched off may report `success` or `skipped`; `failure`
# and `cancelled` always fail, and so does a job of the scope that is missing.
ci_verdict() {
    local scope="$1" job result failed=false seen=" "
    ci_jobs "$scope" > /dev/null || return 1
    while read -r job result; do
        [ -n "$job" ] || continue
        seen+="$job "
        if ci_job_runs "$job" "$scope"; then
            if [ "$result" != success ]; then
                echo "$job: $result (needed in scope $scope)" >&2
                failed=true
                continue
            fi
        elif [ "$result" != success ] && [ "$result" != skipped ]; then
            echo "$job: $result" >&2
            failed=true
            continue
        fi
        echo "$job: $result"
    done
    for job in $CI_ALWAYS_JOBS $(ci_jobs "$scope"); do
        if [[ "$seen" != *" $job "* ]]; then
            echo "$job: missing (needed in scope $scope)" >&2
            failed=true
        fi
    done
    [ "$failed" = false ]
}

# Files changed between $1 and $2, one per line. Renames count as a deletion and an addition, so a
# file moved out of the code into docs/ still counts as a code change.
changed_files() {
    git diff --name-only --no-renames "$1" "$2" --
}

# The $GITHUB_OUTPUT lines for scope $1.
ci_outputs() {
    local scope="$1" job
    ci_jobs "$scope" > /dev/null || return 1
    echo "scope=$scope"
    for job in $CI_GATED_JOBS; do
        if ci_job_runs "$job" "$scope"; then
            echo "$job=true"
        else
            echo "$job=false"
        fi
    done
}

ci_scope_main() {
    local command="${1:-}" files scope
    case "$command" in
        outputs)
            local event="${2:?event name missing}"
            if [ "$event" = pull_request ]; then
                files="$(changed_files "${3:-HEAD^1}" "${4:-HEAD}")"
            else
                files=""
            fi
            scope="$(ci_scope "$event" <<< "$files")"
            echo "Scope $scope for $event; changed files:" >&2
            printf '%s\n' "${files:-(not compared)}" >&2
            ci_outputs "$scope"
            ;;
        verdict) ci_verdict "${2:?scope missing}" ;;
        spec-tests) echo "$CI_SPEC_TESTS" ;;
        *)
            echo "usage: $0 outputs <event> [<base> <head>] | verdict <scope> | spec-tests" >&2
            return 2
            ;;
    esac
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    ci_scope_main "$@"
fi
