#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause

# Fails when a tracked text file contains characters that hide or reorder text: Unicode tags
# (U+E0000–E007F), bidi controls (U+202A–202E, U+2066–2069, U+200E/200F, U+061C), zero-width and
# invisible characters (U+200B–200D, U+2060–2064, U+FEFF, U+00AD, U+180E) and line or paragraph
# separators (U+2028/2029). They can smuggle instructions to a reader or an agent that a reviewer
# does not see. Prints `<file>:<line>: U+XXXX` per finding. Usage: check-hidden-unicode.sh [<dir>]
set -euo pipefail

cd "${1:-.}"
found=0
while IFS= read -r -d '' file; do
    [[ -f "$file" ]] || continue
    # Binary files (a NUL byte in the first 8000 bytes, as git decides) are skipped.
    if head -c 8000 -- "$file" | LC_ALL=C grep -qaP '\x00'; then continue; fi
    if out="$(perl -CSD -ne '
        while (/([\x{E0000}-\x{E007F}\x{202A}-\x{202E}\x{2066}-\x{2069}\x{200B}-\x{200F}\x{061C}\x{2060}-\x{2064}\x{FEFF}\x{00AD}\x{180E}\x{2028}\x{2029}])/g) {
            printf "%s:%d: U+%04X\n", $ARGV, $., ord $1;
        }
        close ARGV if eof;
    ' -- "$file" 2>/dev/null)" && [[ -n "$out" ]]; then
        printf '%s\n' "$out"
        found=1
    fi
done < <(git ls-files -z)
if ((found)); then
    echo "hidden or reordering Unicode characters found (see above)" >&2
    exit 1
fi
echo "check-hidden-unicode: ok"
