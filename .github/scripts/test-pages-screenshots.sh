#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause

# Tests the screenshot source of the Pages site (pages-screenshots.sh) against a fake gh that
# answers from a scenario directory: an artifact that is still there, one that has expired, no
# release with screenshots. Also checks that pages.yml and the site use the script's outputs. Part
# of `just _static-checks`.
set -euo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd -- "$here/../.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# Fake gh. Answers (already in the form of the --jq filter of the call) come from $FAKE_DIR:
#   run                    `run list`: "<id> <sha>" of the newest successful Verify run
#   artifacts-<id>         `api …/runs/<id>/artifacts`: "<name> <expired>" lines
#   releases               `release list`: tags, newest first
#   assets-<tag>           `release view <tag>`: asset names
#   commit-<tag>           `api …/commits/<tag>`: the commit
#   <asset>                `release download --pattern <asset>`: the file to download
# `run download` writes from-artifact.png. Every call is logged to $FAKE_DIR/log.
mkdir -p "$tmp/bin"
cat > "$tmp/bin/gh" <<'FAKE'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$FAKE_DIR/log"
answer() { if [ -e "$FAKE_DIR/$1" ]; then cat "$FAKE_DIR/$1"; fi; }
option() {
    local name="$1"
    shift
    while [ $# -gt 1 ]; do
        if [ "$1" = "$name" ]; then echo "$2"; return; fi
        shift
    done
}
case "$1 $2" in
    "run list") answer run ;;
    "run download") printf 'PNG' > "$(option --dir "$@")/from-artifact.png" ;;
    "release list") answer releases ;;
    "release view") answer "assets-$3" ;;
    "release download") cp "$FAKE_DIR/$(option --pattern "$@")" "$(option --dir "$@")/" ;;
    "api "*)
        case "$2" in
            */artifacts) answer "artifacts-$(basename "$(dirname "$2")")" ;;
            */commits/*) answer "commit-$(basename "$2")" ;;
            *) exit 1 ;;
        esac
        ;;
    *) exit 1 ;;
esac
FAKE
chmod +x "$tmp/bin/gh"

failures=0
check() {
    local expected="$1" actual="$2" what="$3"
    if [ "$expected" != "$actual" ]; then
        echo "FAIL: $what: expected '$expected', got '$actual'" >&2
        failures=$((failures + 1))
    fi
}

# A screenshot bundle as release.yml builds it: PNGs in a directory, plus a file that is no PNG and
# a link named like a PNG that points at a file of the runner.
make_bundle() {
    python3 - "$1" <<'PY'
import stat, sys, zipfile
with zipfile.ZipFile(sys.argv[1], "w") as z:
    z.writestr("screens/m4-s01-snapshots-wide-en.png", b"PNG")
    z.writestr("README.txt", b"not a screenshot")
    link = zipfile.ZipInfo("screens/link.png")
    link.create_system = 3
    link.external_attr = (stat.S_IFLNK | 0o777) << 16
    z.writestr(link, "/etc/hostname")
PY
}

# A bundle without any PNG.
make_empty_bundle() {
    python3 - "$1" <<'PY'
import sys, zipfile
with zipfile.ZipFile(sys.argv[1], "w") as z:
    z.writestr("README.txt", b"not a screenshot")
PY
}

# Runs the script for scenario $1 (event $2) into $tmp/$1/out; sets `status`, `outputs`, `errors`.
run_scenario() {
    local name="$1"
    export FAKE_DIR="$tmp/$name"
    status=0
    outputs="$(PATH="$tmp/bin:$PATH" GITHUB_REPOSITORY=Swissbite/kestic EVENT_NAME="$2" \
        RUN_ID="${RUN_ID:-}" RUN_SHA="${RUN_SHA:-}" \
        "$here/pages-screenshots.sh" "$FAKE_DIR/out" 2> "$FAKE_DIR/stderr")" || status=$?
    errors="$(cat "$FAKE_DIR/stderr")"
}

output() {
    sed -n "s/^$1=//p" <<< "$outputs"
}

# AC-M6-14-5: the artifact is still there: screenshots of the Verify run, no release lookup.
mkdir -p "$tmp/present"
echo "101 aaaa111" > "$tmp/present/run"
printf '%s\n' "nextest-archive true" "smoke-artifacts false" > "$tmp/present/artifacts-101"
run_scenario present pull_request
check 0 "$status" "AC-M6-14-5: artifact present: exit status"
check aaaa111 "$(output sha)" "AC-M6-14-5: artifact present: commit of the Verify run"
check "" "$(output version)" "AC-M6-14-5: artifact present: no release version"
check true "$([ -f "$tmp/present/out/from-artifact.png" ] && echo true || echo false)" \
    "AC-M6-14-5: artifact present: screenshots downloaded"
check false "$(grep -q '^release' "$tmp/present/log" && echo true || echo false)" \
    "AC-M6-14-5: artifact present: no release asked"

# AC-M6-14-5 (a), (b), (c): the artifact has expired: screenshots of the newest release with a
# bundle (the branch pre-release has none), its version and commit, and the build goes on.
mkdir -p "$tmp/expired"
echo "101 aaaa111" > "$tmp/expired/run"
printf '%s\n' "smoke-artifacts true" > "$tmp/expired/artifacts-101"
printf '%s\n' v0.6.0-m6.1 v0.5.0 v0.4.0 > "$tmp/expired/releases"
printf '%s\n' kestic_0.6.0.m6.1-1_amd64.deb SHA256SUMS > "$tmp/expired/assets-v0.6.0-m6.1"
printf '%s\n' kestic_0.5.0-1_amd64.deb kestic_0.5.0_screenshots.zip SHA256SUMS > "$tmp/expired/assets-v0.5.0"
printf '%s\n' kestic_0.4.0-1_amd64.deb kestic_0.4.0_screenshots.zip > "$tmp/expired/assets-v0.4.0"
echo "bbbb222" > "$tmp/expired/commit-v0.5.0"
make_bundle "$tmp/expired/kestic_0.5.0_screenshots.zip"
run_scenario expired pull_request
check true "$([ -f "$tmp/expired/out/m4-s01-snapshots-wide-en.png" ] && echo true || echo false)" \
    "AC-M6-14-5 (a): expired: screenshots of the release bundle"
check false "$([ -e "$tmp/expired/out/README.txt" ] || [ -e "$tmp/expired/out/screens" ] &&
    echo true || echo false)" "AC-M6-14-5 (a): expired: only the PNGs, without directories"
check false "$(grep -q 'release view v0.4.0' "$tmp/expired/log" && echo true || echo false)" \
    "AC-M6-14-5 (a): expired: the newest release with a bundle wins"
check v0.5.0 "$(output version)" "AC-M6-14-5 (b): expired: version of the release"
check bbbb222 "$(output sha)" "AC-M6-14-5 (b): expired: commit of the release"
check 0 "$status" "AC-M6-14-5 (c): expired: exit status"
check true "$(grep -q '::warning::.*v0.5.0.*expired' <<< "$errors" && echo true || echo false)" \
    "AC-M6-14-5 (c): expired: warning names the release and the reason"

check false "$([ -L "$tmp/expired/out/link.png" ] || [ -e "$tmp/expired/out/link.png" ] &&
    echo true || echo false)" "AC-M6-14-5 (a): expired: no link from the bundle"

# AC-M6-14-5 (a), (c): a newest bundle without screenshots is passed over for an older one.
mkdir -p "$tmp/unusable"
echo "101 aaaa111" > "$tmp/unusable/run"
printf '%s\n' "smoke-artifacts true" > "$tmp/unusable/artifacts-101"
printf '%s\n' v0.6.0 v0.5.0 > "$tmp/unusable/releases"
printf '%s\n' kestic_0.6.0_screenshots.zip > "$tmp/unusable/assets-v0.6.0"
cp "$tmp/expired/assets-v0.5.0" "$tmp/expired/commit-v0.5.0" "$tmp/expired/kestic_0.5.0_screenshots.zip" \
    "$tmp/unusable/"
make_empty_bundle "$tmp/unusable/kestic_0.6.0_screenshots.zip"
run_scenario unusable workflow_dispatch
check 0 "$status" "AC-M6-14-5 (c): unusable newest bundle: exit status"
check v0.5.0 "$(output version)" "AC-M6-14-5 (a): unusable newest bundle: the older release is taken"
check true "$(grep -q '::warning::kestic_0.6.0_screenshots.zip of release v0.6.0 has no usable screenshots' <<< "$errors" &&
    echo true || echo false)" "AC-M6-14-5 (a): unusable newest bundle: warning"

# AC-M6-14-5 (a): without any successful Verify run on main the release is taken as well.
mkdir -p "$tmp/no-run"
cp "$tmp/expired/releases" "$tmp/expired"/assets-* "$tmp/expired/commit-v0.5.0" \
    "$tmp/expired/kestic_0.5.0_screenshots.zip" "$tmp/no-run/"
run_scenario no-run workflow_dispatch
check 0 "$status" "AC-M6-14-5 (a): no Verify run: exit status"
check v0.5.0 "$(output version)" "AC-M6-14-5 (a): no Verify run: version of the release"

# A Verify run that started the workflow is taken without looking for another one.
mkdir -p "$tmp/workflow-run"
printf '%s\n' "smoke-artifacts false" > "$tmp/workflow-run/artifacts-202"
RUN_ID=202 RUN_SHA=cccc333 run_scenario workflow-run workflow_run
check 0 "$status" "AC-M6-14-5: workflow_run: exit status"
check cccc333 "$(output sha)" "AC-M6-14-5: workflow_run: commit of the triggering run"
check false "$(grep -q '^run list' "$tmp/workflow-run/log" && echo true || echo false)" \
    "AC-M6-14-5: workflow_run: no other run looked up"

# AC-M6-14-5 (d): expired artifact and no release with a bundle: a clear error, no outputs.
mkdir -p "$tmp/nothing"
echo "101 aaaa111" > "$tmp/nothing/run"
printf '%s\n' "smoke-artifacts true" > "$tmp/nothing/artifacts-101"
printf '%s\n' v0.6.0-m6.1 > "$tmp/nothing/releases"
printf '%s\n' kestic_0.6.0.m6.1-1_amd64.deb > "$tmp/nothing/assets-v0.6.0-m6.1"
run_scenario nothing pull_request
check 1 "$status" "AC-M6-14-5 (d): nothing: exit status"
check "" "$outputs" "AC-M6-14-5 (d): nothing: no outputs"
check true "$(grep -q '::error::No screenshots for the site: the smoke-artifacts artifact of Verify run 101 has expired, and no release has' <<< "$errors" &&
    echo true || echo false)" "AC-M6-14-5 (d): nothing: the error names both sources"

# AC-M6-14-5 (b), (c): pages.yml takes the screenshots from the script and hands version and
# commit to the site, which shows them.
pages_yml="$root/.github/workflows/pages.yml"
check true "$(grep -qF '.github/scripts/pages-screenshots.sh "$RUNNER_TEMP/smoke-artifacts" >> "$GITHUB_OUTPUT"' "$pages_yml" &&
    echo true || echo false)" "AC-M6-14-5 (c): pages.yml runs pages-screenshots.sh"
check true "$(grep -qF 'KESTIC_SCREENSHOTS_VERSION: ${{ steps.source.outputs.version }}' "$pages_yml" &&
    echo true || echo false)" "AC-M6-14-5 (b): pages.yml hands the release version to the site"
check true "$(grep -qF "KESTIC_SCREENSHOTS_ALLOW_MISSING: \${{ github.event_name == 'pull_request' || steps.source.outputs.version != '' }}" "$pages_yml" &&
    echo true || echo false)" "AC-M6-14-5 (c): a release bundle older than the manifest does not stop the deploy"
check true "$(grep -qF 'KESTIC_BUILD_SHA: ${{ steps.source.outputs.sha }}' "$pages_yml" &&
    echo true || echo false)" "AC-M6-14-5 (b): pages.yml hands the commit to the site"
check true "$(grep -qF 'process.env["KESTIC_SCREENSHOTS_VERSION"]' "$root/site/src/build-info.ts" &&
    echo true || echo false)" "AC-M6-14-5 (b): the site reads the release version"
index_astro="$root/site/src/pages/index.astro"
check true "$(grep -qF 'Development preview · {screenshotsVersion ?? "main"} @ <a href={commitUrl}>{buildShaShort}</a>' "$index_astro" &&
    echo true || echo false)" "AC-M6-14-5 (b): the preview bar shows the release and its commit"
check true "$(grep -qF 'Rendered by the test suite at {screenshotsVersion ? `${screenshotsVersion} @ ` : ""}<a' "$index_astro" &&
    echo true || echo false)" "AC-M6-14-5 (b): the screenshot note shows the release"

if [ "$failures" -ne 0 ]; then
    echo "test-pages-screenshots.sh: $failures failure(s)" >&2
    exit 1
fi
echo "test-pages-screenshots.sh: all checks passed"
