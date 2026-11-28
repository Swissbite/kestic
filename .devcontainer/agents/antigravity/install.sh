#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause
#
# Installs Antigravity CLI (`agy`) inside the agent container image (runs as
# root at build time) and prepares its config mount point.

set -euo pipefail

base=https://antigravity-cli-auto-updater-974169037036.us-central1.run.app
case "$(uname -m)" in
    x86_64) arch=amd64 ;;
    aarch64) arch=arm64 ;;
    *) echo "antigravity/install.sh: unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac

manifest="$(curl -fsSL "$base/manifests/linux_${arch}.json")"
version="$(jq -r '.version // empty' <<< "$manifest")"
url="$(jq -r '.url // empty' <<< "$manifest")"
sha512="$(jq -r '.sha512 // empty' <<< "$manifest")"

wanted="${AGENT_VERSION:-${AGY_VERSION:-latest}}"
if [[ -n "$wanted" && "$wanted" != latest && "$version" != "$wanted" ]]; then
    echo "antigravity/install.sh: manifest version $version does not match requested $wanted" >&2
    exit 1
fi
[[ "$url" =~ ^https:// ]] || { echo "antigravity/install.sh: unexpected download URL: $url" >&2; exit 1; }
[[ "$sha512" =~ ^[0-9a-f]{128}$ ]] || { echo "antigravity/install.sh: invalid sha512 in manifest" >&2; exit 1; }

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

payload="$tmp/payload"
curl -fsSL -o "$payload" "$url"
echo "$sha512  $payload" | sha512sum -c -

case "$url" in
    *.tar.gz*)
        tar -xzf "$payload" -C "$tmp"
        if [[ -f "$tmp/antigravity" ]]; then
            install -m 0755 "$tmp/antigravity" /usr/local/bin/agy
        elif [[ -f "$tmp/agy" ]]; then
            install -m 0755 "$tmp/agy" /usr/local/bin/agy
        else
            echo "antigravity/install.sh: no binary found in archive" >&2
            exit 1
        fi
        ;;
    *)
        install -m 0755 "$payload" /usr/local/bin/agy
        ;;
esac

AGY_CLI_DISABLE_AUTO_UPDATE=true /usr/local/bin/agy --version

mkdir -p /home/kesticci/.gemini /home/kesticci/.config
chown -R kesticci:kesticci /home/kesticci/.gemini /home/kesticci/.config
