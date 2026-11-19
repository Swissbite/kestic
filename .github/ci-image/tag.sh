#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause
#
# Prints the CI image tag: the first 16 hex chars of the SHA-256 over the
# concatenated image inputs (Dockerfile + scripts/bootstrap.sh).
#
#   tag.sh          tag for the working tree
#   tag.sh <rev>    tag for a git revision (used by ci-image-cleanup.yml)
#
# Used by ci-image.yml (build/push) and ci-image-cleanup.yml (which tags are
# still in use), so both always agree on the tag of a given revision.

set -euo pipefail

cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.."

INPUTS=(.github/ci-image/Dockerfile scripts/bootstrap.sh)

if [[ $# -gt 0 ]]; then
    for f in "${INPUTS[@]}"; do
        if ! git cat-file -e "$1:$f" 2>/dev/null; then
            echo "tag.sh: $f does not exist in $1" >&2
            exit 1
        fi
    done
fi

if [[ $# -eq 0 ]]; then
    cat -- "${INPUTS[@]}"
else
    for f in "${INPUTS[@]}"; do
        git show "$1:$f"
    done
fi | sha256sum | cut -c1-16
