#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause

# Tests the job selection of the Verify workflow (ci-scope.sh) against example file lists and job
# results, and checks that verify.yml and the justfile use it. Part of `just _static-checks`.
set -euo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd -- "$here/../.." && pwd)"
# shellcheck source=ci-scope.sh
source "$here/ci-scope.sh"

failures=0
check() {
    local expected="$1" actual="$2" what="$3"
    if [ "$expected" != "$actual" ]; then
        echo "FAIL: $what: expected '$expected', got '$actual'" >&2
        failures=$((failures + 1))
    fi
}

# Scope of a pull request (or of event $2) changing the files $1 (one per line).
scope_of() {
    ci_scope "${2:-pull_request}" <<< "$1"
}

# `pass` or `fail`: verdict in scope $1 over the `<job> <result>` lines $2.
verdict_of() {
    if ci_verdict "$1" <<< "$2" > /dev/null 2>&1; then echo pass; else echo fail; fi
}

# The value of output $2 in scope $1.
output_of() {
    ci_outputs "$1" | sed -n "s/^$2=//p"
}

# Job results of a run in which every job of scope $1 succeeded and the others were skipped.
results_for() {
    local job
    for job in $CI_ALWAYS_JOBS $CI_GATED_JOBS; do
        if ci_job_runs "$job" "$1"; then echo "$job success"; else echo "$job skipped"; fi
    done
}

# Like results_for, with job $2 reporting $3 instead.
results_with() {
    results_for "$1" | sed "s/^$2 .*/$2 $3/"
}

# AC-M6-14-1 (a): documentation and site only skips build, clippy, tests, smoke and .deb.
docs_lists=(
    "README.md"
    "site/src/pages/index.astro"
    "docs/releases/v0.5.0.md"
    "docs/releases/README.md
.agents/skills/kestic-milestone-autonomy/SKILL.md
site/package.json"
    ".devcontainer/README.md
docs/screenshots/notes.txt"
)
for files in "${docs_lists[@]}"; do
    check docs "$(scope_of "$files")" "AC-M6-14-1 (a): scope of ${files//$'\n'/, }"
done
for job in clippy build release test deb-check spec-tests; do
    check false "$(output_of docs "$job")" "AC-M6-14-1 (a): $job is switched off for docs"
done

# AC-M6-14-1 (b): reuse lint and the size check (`checks`) still run.
if ! ci_job_runs checks docs; then check runs skipped "AC-M6-14-1 (b): checks in scope docs"; fi
if ! ci_job_runs ci-image docs; then check runs skipped "AC-M6-14-1 (b): ci-image in scope docs"; fi
static_checks="$(sed -n '/^_static-checks:/,/^$/p' "$root/justfile")"
check true "$(grep -qx '    reuse lint' <<< "$static_checks" && echo true || echo false)" \
    "AC-M6-14-1 (b): _static-checks runs reuse lint"
check true "$(grep -qx '    just _check-file-sizes' <<< "$static_checks" && echo true || echo false)" \
    "AC-M6-14-1 (b): _static-checks runs the size check"

# AC-M6-14-1 (c): `verify` reports success when the switched-off jobs are skipped.
check pass "$(verdict_of docs "$(results_for docs)")" "AC-M6-14-1 (c): verdict of a docs run"

# AC-M6-14-2 (a): the specification alone runs the tests that read it.
check spec "$(scope_of "docs/spec/backlog.md")" "AC-M6-14-2 (a): scope of docs/spec only"
check spec "$(scope_of "docs/spec/adr/0001-tech-stack-rust-cxx-qt-kirigami.md
README.md
site/src/screenshots.ts")" "AC-M6-14-2 (a): scope of docs/spec with docs and site"
check true "$(output_of spec spec-tests)" "AC-M6-14-2 (a): spec-tests runs for the spec"
for job in clippy build release test deb-check; do
    check false "$(output_of spec "$job")" "AC-M6-14-2 (a): $job is switched off for the spec"
done
check pass "$(verdict_of spec "$(results_for spec)")" "AC-M6-14-2 (a): verdict of a spec run"
check fail "$(verdict_of spec "$(results_with spec spec-tests skipped)")" \
    "AC-M6-14-2 (a): spec-tests skipped in a spec run"

# AC-M6-14-2 (a): every Rust file that names docs/spec/ (or a path part "spec") in a string is a
# spec test, one test binary in <crate>/tests/, and every spec test exists in its package.
while IFS= read -r file; do
    binary="$(basename "$file" .rs)"
    check true "$([[ "$file" == */tests/"$binary".rs ]] && grep -q " $binary\$" <<< "$CI_SPEC_TESTS" &&
        echo true || echo false)" "AC-M6-14-2 (a): $file reads docs/spec/ and is in CI_SPEC_TESTS"
done < <(cd "$root" && git grep -lE '"[^"]*docs/spec|"spec"' -- '*.rs')
while read -r package binary; do
    file="$(cd "$root" && git ls-files -- "*/tests/$binary.rs")"
    crate="${file%%/*}"
    check true "$([ -n "$file" ] && grep -qx "name = \"$package\"" "$root/$crate/Cargo.toml" &&
        echo true || echo false)" "AC-M6-14-2 (a): spec test $package $binary exists"
done <<< "$CI_SPEC_TESTS"
check 3 "$(grep -c . <<< "$CI_SPEC_TESTS")" "AC-M6-14-2 (a): number of spec tests"

# AC-M6-14-2 (b): translations, code or anything else runs everything.
full_lists=(
    "po/de/kestic.po"
    "core/src/lib.rs"
    "README.md
core/src/lib.rs"
    "docs/spec/glossary.md
po/de/kestic.po"
    ".github/workflows/verify.yml"
    ".github/scripts/ci-scope.sh"
    "justfile"
    "Cargo.lock"
    "gui/src/qml/Main.qml"
    "packaging/net.daester.Kestic.desktop"
    "docs.rs"
    "site"
    ""
)
for files in "${full_lists[@]}"; do
    check full "$(scope_of "$files")" "AC-M6-14-2 (b): scope of '${files//$'\n'/, }'"
done
for job in clippy build release test deb-check; do
    check true "$(output_of full "$job")" "AC-M6-14-2 (b): $job runs in a full run"
done
check false "$(output_of full spec-tests)" "AC-M6-14-2 (b): the unit tests cover the spec tests"

# AC-M6-14-2 (b): a file moved out of the code into docs/ still counts as a code change.
# The temporary repository uses neither the user's nor the system's git configuration.
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
repo="$tmp/repo"
mkdir -p "$repo" "$tmp/home"
in_repo() {
    (cd "$repo" && HOME="$tmp/home" XDG_CONFIG_HOME="$tmp/home/.config" GIT_CONFIG_GLOBAL=/dev/null \
        GIT_CONFIG_NOSYSTEM=1 GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL=t@example.invalid \
        GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t@example.invalid "$@")
}
in_repo git init -q
in_repo git commit -q --allow-empty -m base
mkdir -p "$repo/core/src"
printf 'pub fn a() {}\n' > "$repo/core/src/a.rs"
in_repo git add core/src/a.rs
in_repo git commit -q -m code
base="$(in_repo git rev-parse HEAD)"
mkdir -p "$repo/docs"
in_repo git mv core/src/a.rs docs/a.md
in_repo git commit -q -m move
check full "$(in_repo changed_files "$base" HEAD | ci_scope pull_request)" \
    "AC-M6-14-2 (b): code renamed into docs/"
check "scope=full" "$(in_repo ci_scope_main outputs pull_request "$base" HEAD 2> /dev/null |
    head -n 1)" "AC-M6-14-2 (b): outputs of the rename"

# AC-M6-14-1 (a), AC-M6-14-2 (a): names outside ASCII keep their scope.
mkdir -p "$repo/docs/spec"
printf 'x\n' > "$repo/docs/spec/übersicht.md"
printf 'x\n' > "$repo/docs/änderungen.md"
in_repo git add docs
in_repo git commit -q -m umlauts
check spec "$(in_repo changed_files HEAD^ HEAD | ci_scope pull_request)" \
    "AC-M6-14-2 (a): docs/spec file with an umlaut"
check docs "$(in_repo changed_files HEAD^ HEAD | grep -v '^docs/spec/' | ci_scope pull_request)" \
    "AC-M6-14-1 (a): docs file with an umlaut"

# AC-M6-14-3 (a), (b): pushes and other events run everything, whatever changed.
for event in push workflow_dispatch workflow_run schedule; do
    check full "$(scope_of "README.md" "$event")" "AC-M6-14-3 (a): scope of $event with README.md"
    check full "$(scope_of "docs/spec/backlog.md" "$event")" "AC-M6-14-3 (a): scope of $event with docs/spec"
done
check "scope=full" "$(ci_scope_main outputs push 2> /dev/null | head -n 1)" \
    "AC-M6-14-3 (a): outputs of a push compare no files"
for job in test release; do
    check true "$(output_of "$(scope_of "README.md" push)" "$job")" \
        "AC-M6-14-3 (b): $job (screenshots, .deb) runs on a push"
done

# AC-M6-14-4 (a): success, and skipped for switched-off jobs, count as success.
check pass "$(verdict_of full "$(results_for full)")" "AC-M6-14-4 (a): all succeeded"
check pass "$(verdict_of docs "$(results_with docs clippy success)")" \
    "AC-M6-14-4 (a): a switched-off job that ran anyway"

# AC-M6-14-4 (b): failure or cancelled fail, so does a needed job that was skipped or is missing.
for scope in full spec docs; do
    for job in ci-image scope checks clippy test deb-check spec-tests; do
        check fail "$(verdict_of "$scope" "$(results_with "$scope" "$job" failure)")" \
            "AC-M6-14-4 (b): $job failed in scope $scope"
        check fail "$(verdict_of "$scope" "$(results_with "$scope" "$job" cancelled)")" \
            "AC-M6-14-4 (b): $job cancelled in scope $scope"
    done
    check fail "$(verdict_of "$scope" "$(results_with "$scope" checks skipped)")" \
        "AC-M6-14-4 (b): checks skipped in scope $scope"
    check fail "$(verdict_of "$scope" "$(results_for "$scope" | grep -v '^checks ')")" \
        "AC-M6-14-4 (b): checks missing in scope $scope"
done
check fail "$(verdict_of full "$(results_with full build skipped)")" "AC-M6-14-4 (b): build skipped in a full run"
check fail "$(verdict_of full "$(results_for full | grep -v '^test ')")" "AC-M6-14-4 (b): test missing in a full run"
check fail "$(verdict_of "" "$(results_for full)")" "AC-M6-14-4 (b): no scope (the scope job failed)"
check fail "$(verdict_of docs "$(results_for docs)
new-job skipped")" "AC-M6-14-4 (b): an unknown job counts as needed"

# AC-M6-14-4 (c): verify.yml filters no paths on its triggers, gates each job by the scope and lets
# `verify` decide with ci-scope.sh.
verify_yml="$root/.github/workflows/verify.yml"
triggers="$(sed -n '/^on:/,/^[a-z]/p' "$verify_yml")"
check false "$(grep -qE '^\s*(paths|paths-ignore):' <<< "$triggers" && echo true || echo false)" \
    "AC-M6-14-4 (c): no paths filter on the triggers of verify.yml"
check false "$(grep -qE '^\s*paths-ignore:' "$verify_yml" && echo true || echo false)" \
    "AC-M6-14-4 (c): no paths-ignore in verify.yml"
for job in $CI_GATED_JOBS; do
    block="$(sed -n "/^  $job:/,/^  [a-z]/p" "$verify_yml")"
    check true "$(grep -qF "if: needs.scope.outputs.$job == 'true'" <<< "$block" && echo true || echo false)" \
        "AC-M6-14-4 (c): job $job runs only when its scope needs it"
done
for job in $CI_ALWAYS_JOBS; do
    block="$(sed -n "/^  $job:/,/^  [a-z]/p" "$verify_yml")"
    check false "$(grep -qF 'needs.scope.outputs' <<< "$block" && echo true || echo false)" \
        "AC-M6-14-4 (c): job $job does not depend on the scope"
done
verify_job="$(sed -n '/^  verify:/,$p' "$verify_yml")"
verify_needs="$(sed -n 's/^    needs: \[\(.*\)\]$/\1/p' <<< "$verify_job" | tr -d ' ' | tr ',' '\n' | sort)"
check "$(printf '%s\n' $CI_ALWAYS_JOBS $CI_GATED_JOBS | sort)" "$verify_needs" \
    "AC-M6-14-4 (c): verify needs every job"
check true "$(grep -qx '    if: always()' <<< "$verify_job" && echo true || echo false)" \
    "AC-M6-14-4 (b): verify reports even when a job failed or was skipped"
check true "$(grep -qF '.github/scripts/ci-scope.sh verdict "$SCOPE"' <<< "$verify_job" && echo true || echo false)" \
    "AC-M6-14-4 (c): verify decides with ci-scope.sh"
check true "$(grep -qF 'ci-scope.sh outputs "$EVENT_NAME"' "$verify_yml" && echo true || echo false)" \
    "AC-M6-14-4 (c): the scope job runs ci-scope.sh"
check true "$(grep -qF '.github/scripts/ci-just.sh _spec-tests' "$verify_yml" && echo true || echo false)" \
    "AC-M6-14-2 (a): the spec-tests job runs _spec-tests"

if [ "$failures" -ne 0 ]; then
    echo "test-ci-scope.sh: $failures failure(s)" >&2
    exit 1
fi
echo "test-ci-scope.sh: all checks passed"
