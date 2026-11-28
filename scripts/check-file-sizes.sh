#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause

# Checks the size limit of NFR-12: no `.rs`, `.qml` or `.cpp` file below the `src/` directory of
# a crate has more than 1500 lines. Checks the workspace at $1 (default: the workspace of this
# script). Part of `just _static-checks`.
set -euo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
root="${1:-"$here/.."}"
limit=1500

cd -- "$root"
oversized=0
while IFS= read -r -d '' f; do
    lines=$(wc -l < "$f")
    if [ "$lines" -gt "$limit" ]; then
        echo "ERROR: $f has $lines lines (limit $limit, NFR-12)" >&2
        oversized=1
    fi
done < <(find build-stamp/src core/src i18n/src daemon/src gui/src test-support/src -type f \
    \( -name '*.rs' -o -name '*.qml' -o -name '*.cpp' \) -print0 | sort -z)
exit "$oversized"
