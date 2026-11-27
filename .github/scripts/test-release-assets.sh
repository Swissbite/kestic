#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause

# Tests the asset names and SHA256SUMS of the release workflow (release-assets.sh) against temp
# folders, and checks that release.yml uses it. Part of `just _static-checks`.
set -euo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd -- "$here/../.." && pwd)"
# shellcheck source=release-assets.sh
source "$here/release-assets.sh"

failures=0
check() {
    local expected="$1" actual="$2" what="$3"
    if [ "$expected" != "$actual" ]; then
        echo "FAIL: $what: expected '$expected', got '$actual'" >&2
        failures=$((failures + 1))
    fi
}

# `pass` or `fail`: the exit status of a command, its output dropped.
outcome() {
    if ("$@") > /dev/null 2>&1; then echo pass; else echo fail; fi
}

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# A release folder $1 as the jobs `build` and `screenshots` leave it for version $2.
make_assets() {
    mkdir -p "$1"
    printf 'deb %s\n' "$2" > "$1/kestic_$2-1_amd64.deb"
    printf 'zip %s\n' "$2" > "$1/kestic_$2_screenshots.zip"
}

# What GitHub serves after the upload of folder $1, downloaded into $2: the same files, with `~`
# replaced by `.` as GitHub does (independent of upload_name).
github_upload() {
    local file
    mkdir -p "$2"
    for file in "$1"/*; do
        file="${file##*/}"
        cp -- "$1/$file" "$2/${file//\~/.}"
    done
}

# The file names SHA256SUMS in folder $1 lists, sorted.
listed_names() {
    sed -E 's/^[0-9a-f]{64} [ *]//' "$1/SHA256SUMS" | LC_ALL=C sort
}

# AC-M6-15-1 (a): the upload name replaces `~` (and every other special character) by `.`.
check kestic_0.6.0.m6.3-1_amd64.deb "$(upload_name 'kestic_0.6.0~m6.3-1_amd64.deb')" \
    "AC-M6-15-1 (a): pre-release package"
check kestic_1.0.0.rc.1_screenshots.zip "$(upload_name 'kestic_1.0.0~rc.1_screenshots.zip')" \
    "AC-M6-15-1 (a): release candidate screenshots"
check a.b.c.d "$(upload_name 'a b+c~d')" "AC-M6-15-1 (a): space, plus and tilde"
for name in '~kestic.deb' 'kestic.deb~' 'kestic~.deb' '' '~'; do
    check fail "$(outcome upload_name "$name")" "AC-M6-15-1 (a): '$name' has no unambiguous upload name"
done

# AC-M6-15-1 (a): prepare renames the assets and SHA256SUMS names them as uploaded.
pre="$tmp/pre"
make_assets "$pre" '0.6.0~m6.3'
check pass "$(outcome release_assets_prepare "$pre")" "AC-M6-15-1 (a): prepare a pre-release"
check "$(printf '%s\n' kestic_0.6.0.m6.3-1_amd64.deb kestic_0.6.0.m6.3_screenshots.zip)" \
    "$(listed_names "$pre")" "AC-M6-15-1 (a): SHA256SUMS lists the upload names"
check "$(printf '%s\n' SHA256SUMS kestic_0.6.0.m6.3-1_amd64.deb kestic_0.6.0.m6.3_screenshots.zip)" \
    "$(cd "$pre" && ls -A | LC_ALL=C sort)" "AC-M6-15-1 (a): the files carry the upload names"
check 'deb 0.6.0~m6.3' "$(cat "$pre/kestic_0.6.0.m6.3-1_amd64.deb")" \
    "AC-M6-15-1 (a): renaming keeps the content"

collide="$tmp/collide"
mkdir -p "$collide"
echo one > "$collide/kestic_0.6.0~m6.3.deb"
echo two > "$collide/kestic_0.6.0.m6.3.deb"
check fail "$(outcome release_assets_prepare "$collide")" "AC-M6-15-1 (a): two assets with one upload name"
check one "$(cat "$collide/kestic_0.6.0~m6.3.deb")" "AC-M6-15-1 (a): a collision overwrites nothing"
check two "$(cat "$collide/kestic_0.6.0.m6.3.deb")" "AC-M6-15-1 (a): a collision overwrites nothing (other file)"

# AC-M6-15-1 (b): `sha256sum -c` passes in the folder of the downloaded assets.
github_upload "$pre" "$tmp/pre-downloaded"
check pass "$(outcome bash -c "cd '$tmp/pre-downloaded' && sha256sum -c SHA256SUMS")" \
    "AC-M6-15-1 (b): sha256sum -c next to the downloaded assets"
check pass "$(outcome release_assets_check "$tmp/pre-downloaded")" \
    "AC-M6-15-1 (b): check of the downloaded assets"

# The naming before this fix: SHA256SUMS of the local names fails after the upload.
old="$tmp/old"
make_assets "$old" '0.6.0~m6.3'
(cd "$old" && sha256sum -- * > "$tmp/SHA256SUMS" && mv "$tmp/SHA256SUMS" .)
github_upload "$old" "$tmp/old-downloaded"
check fail "$(outcome bash -c "cd '$tmp/old-downloaded' && sha256sum -c SHA256SUMS")" \
    "AC-M6-15-1 (b): local names in SHA256SUMS fail after the upload"
check fail "$(outcome release_assets_check "$tmp/old-downloaded")" \
    "AC-M6-15-1 (b): check finds the local names"

# AC-M6-15-1 (c): the check fails on any difference between SHA256SUMS and the folder.
variant() {
    rm -rf "$tmp/variant"
    cp -a "$tmp/pre-downloaded" "$tmp/variant"
    echo "$tmp/variant"
}
dir="$(variant)"
echo tampered > "$dir/kestic_0.6.0.m6.3-1_amd64.deb"
check fail "$(outcome release_assets_check "$dir")" "AC-M6-15-1 (c): a changed asset"
dir="$(variant)"
rm -f "$dir/kestic_0.6.0.m6.3_screenshots.zip"
check fail "$(outcome release_assets_check "$dir")" "AC-M6-15-1 (c): a missing asset"
dir="$(variant)"
echo extra > "$dir/kestic_0.6.0.m6.3-1_arm64.deb"
check fail "$(outcome release_assets_check "$dir")" "AC-M6-15-1 (c): an asset SHA256SUMS does not list"
dir="$(variant)"
rm -f "$dir/SHA256SUMS"
check fail "$(outcome release_assets_check "$dir")" "AC-M6-15-1 (c): no SHA256SUMS"
dir="$(variant)"
: > "$dir/SHA256SUMS"
check fail "$(outcome release_assets_check "$dir")" "AC-M6-15-1 (c): an empty SHA256SUMS"

# AC-M6-15-2 (a): a release without `~` keeps its names.
check kestic_0.6.0-1_amd64.deb "$(upload_name kestic_0.6.0-1_amd64.deb)" "AC-M6-15-2 (a): release package"
check kestic_0.6.0_screenshots.zip "$(upload_name kestic_0.6.0_screenshots.zip)" \
    "AC-M6-15-2 (a): release screenshots"
final="$tmp/final"
make_assets "$final" 0.6.0
check pass "$(outcome release_assets_prepare "$final")" "AC-M6-15-2 (a): prepare a release"
check "$(printf '%s\n' SHA256SUMS kestic_0.6.0-1_amd64.deb kestic_0.6.0_screenshots.zip)" \
    "$(cd "$final" && ls -A | LC_ALL=C sort)" "AC-M6-15-2 (a): the files keep their names"
check "$(printf '%s\n' kestic_0.6.0-1_amd64.deb kestic_0.6.0_screenshots.zip)" \
    "$(listed_names "$final")" "AC-M6-15-2 (a): SHA256SUMS lists the names unchanged"
github_upload "$final" "$tmp/final-downloaded"
check pass "$(outcome bash -c "cd '$tmp/final-downloaded' && sha256sum -c SHA256SUMS")" \
    "AC-M6-15-2 (a): sha256sum -c next to the downloaded release assets"

# AC-M6-15-1 (a), (c): release.yml names, hashes and checks the assets with this script, and only
# the job `sums` runs code from the checkout.
release_yml="$root/.github/workflows/release.yml"
job_block() {
    sed -n "/^  $1:/,/^  [a-z][a-z-]*:\$/p" "$release_yml"
}
has() {
    if grep -qF -- "$1" <<< "$2"; then echo true; else echo false; fi
}
sums="$(job_block sums)"
check true "$(has '.github/scripts/release-assets.sh prepare assets' "$sums")" \
    "AC-M6-15-1 (a): job sums prepares the assets"
check true "$(has 'name: release-assets' "$sums")" "AC-M6-15-1 (a): job sums uploads release-assets"
check "    permissions:
      contents: read" "$(sed -n '/^    permissions:/,/^    [a-z-]*:\(\s\|$\)/p' <<< "$sums" | sed '$d')" \
    "AC-M6-15-1 (a): job sums may only read the contents"
for job in attest publish; do
    check false "$(has 'actions/checkout' "$(job_block "$job")")" \
        "AC-M6-15-1 (c): job $job runs no code from the checkout"
done
check false "$(has 'sha256sum -- * >' "$(job_block attest)$(job_block publish)")" \
    "AC-M6-15-1 (a): attest and publish write no SHA256SUMS of the local names"

# Line number of the first line of the publish job that contains $1, or 0.
line_of() {
    local n
    n="$(job_block publish | grep -nF -- "$1" | head -1 | cut -d: -f1)"
    echo "${n:-0}"
}
create="$(line_of 'gh release create "$TAG" assets/*')"
download="$(line_of 'gh release download "$TAG"')"
compare="$(line_of 'cmp -s assets/SHA256SUMS')"
names="$(line_of 'diff <(ls -A assets)')"
verify="$(line_of 'sha256sum --check --strict SHA256SUMS')"
release="$(line_of 'gh release edit "$TAG" --repo "$GITHUB_REPOSITORY" "${publish[@]}"')"
for step in create download compare names verify release; do
    check true "$([ "${!step}" -gt 0 ] && echo true || echo false)" "AC-M6-15-1 (c): publish has the $step line"
done
check true "$([ "$create" -lt "$download" ] && echo true || echo false)" \
    "AC-M6-15-1 (c): download after the upload"
check true "$([ "$download" -lt "$compare" ] && [ "$download" -lt "$names" ] && [ "$download" -lt "$verify" ] && echo true || echo false)" \
    "AC-M6-15-1 (c): checks after the download"
check true "$([ "$compare" -lt "$release" ] && [ "$names" -lt "$release" ] && [ "$verify" -lt "$release" ] && echo true || echo false)" \
    "AC-M6-15-1 (c): checks before the draft is published"

# AC-M6-15-1 (c): run the check of job publish, from the download to the publishing of the draft,
# with a fake `gh` that serves folder $1 as the uploaded assets and logs `release edit`. Prints
# `<pass|fail> <published|draft>`.
check_block="$(job_block publish |
    sed -n '/^ *uploaded="\$RUNNER_TEMP\/uploaded"$/,/^ *gh release edit "\$TAG"/p' | sed 's/^          //')"
mkdir -p "$tmp/bin"
cat > "$tmp/bin/gh" << 'EOF'
#!/usr/bin/env bash
set -euo pipefail
case "$1 $2" in
    "release download")
        shift 2
        while [ "$#" -gt 0 ]; do
            if [ "$1" = --dir ]; then dir="$2"; fi
            shift
        done
        cp -a "$FAKE_UPLOADED/." "$dir/"
        ;;
    "release edit") echo edit >> "$FAKE_GH_LOG" ;;
    *) exit 2 ;;
esac
EOF
chmod +x "$tmp/bin/gh"
run_publish_check() {
    local work="$tmp/publish" result state=draft
    rm -rf "$work"
    mkdir -p "$work/runner"
    cp -a "$pre" "$work/assets"
    if (cd "$work" && PATH="$tmp/bin:$PATH" RUNNER_TEMP="$work/runner" TAG=v0.6.0-m6.3 \
        GITHUB_REPOSITORY=owner/kestic FAKE_UPLOADED="$1" FAKE_GH_LOG="$work/gh.log" \
        bash -c "set -euo pipefail; publish=(--draft=false); $check_block") > /dev/null 2>&1; then
        result=pass
    else
        result=fail
    fi
    if [ -s "$work/gh.log" ]; then state=published; fi
    echo "$result $state"
}
check true "$([ -n "$check_block" ] && echo true || echo false)" "AC-M6-15-1 (c): publish has the check block"
check "pass published" "$(run_publish_check "$pre")" "AC-M6-15-1 (c): matching upload is published"
dir="$(variant)"
echo tampered > "$dir/kestic_0.6.0.m6.3-1_amd64.deb"
check "fail draft" "$(run_publish_check "$dir")" "AC-M6-15-1 (c): a changed asset stays a draft"
dir="$(variant)"
echo extra > "$dir/kestic_0.6.0.m6.3-1_arm64.deb"
check "fail draft" "$(run_publish_check "$dir")" "AC-M6-15-1 (c): an unlisted asset stays a draft"
dir="$(variant)"
mv "$dir/kestic_0.6.0.m6.3_screenshots.zip" "$dir/kestic_0.6.0.m6.3_screenshots.ZIP"
check "fail draft" "$(run_publish_check "$dir")" "AC-M6-15-1 (c): an asset GitHub renamed stays a draft"
dir="$(variant)"
sed -i "s/^\([0-9a-f]\{64\}\)  /\1 */" "$dir/SHA256SUMS"
check "fail draft" "$(run_publish_check "$dir")" "AC-M6-15-1 (c): another SHA256SUMS stays a draft"
dir="$(variant)"
find "$dir" -mindepth 1 -delete
check "fail draft" "$(run_publish_check "$dir")" "AC-M6-15-1 (c): an empty download stays a draft"

check true "$(grep -qF '.github/scripts/test-release-assets.sh' "$root/justfile" && echo true || echo false)" \
    "this test runs in just _static-checks"

if [ "$failures" -ne 0 ]; then
    exit 1
fi
echo "release-assets.sh: all checks passed"
