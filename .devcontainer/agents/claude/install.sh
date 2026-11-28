#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause
#
# Installs Claude Code inside the agent container image (runs as root at build
# time) and prepares its config mount point.

set -euo pipefail

base=https://downloads.claude.ai/claude-code-releases
case "$(uname -m)" in
    x86_64) platform=linux-x64 ;;
    aarch64) platform=linux-arm64 ;;
    *) echo "claude/install.sh: unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac

version="${AGENT_VERSION:-${CLAUDE_CODE_VERSION:-stable}}"
case "$version" in
    stable | latest) version="$(curl -fsSL "$base/$version")" ;;
esac
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] \
    || { echo "claude/install.sh: invalid version '$version'" >&2; exit 1; }

checksum="$(curl -fsSL "$base/$version/manifest.json" | jq -r --arg p "$platform" '.platforms[$p].checksum')"
[[ "$checksum" =~ ^[0-9a-f]{64}$ ]] \
    || { echo "claude/install.sh: invalid sha256 checksum" >&2; exit 1; }

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

curl -fsSL -o "$tmp/claude" "$base/$version/$platform/claude"
echo "$checksum  $tmp/claude" | sha256sum -c -
install -m 0755 "$tmp/claude" /usr/local/bin/claude

mkdir -p /home/kesticci/.claude
chown -R kesticci:kesticci /home/kesticci/.claude
