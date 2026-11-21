#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause

# Runs `just <recipe> [arguments…]` in the workspace as user `kesticci` (the tests must not run as
# root), with the Rust toolchain on PATH, en_US.UTF-8, no incremental builds and the nextest
# profile `ci`. Arguments reach `just` unchanged.
set -euo pipefail

KESTIC_CI_JUST_ARGS="$(printf '%q ' "$@")"
KESTIC_CI_WORKSPACE="$GITHUB_WORKSPACE"
export KESTIC_CI_JUST_ARGS KESTIC_CI_WORKSPACE

exec su - kesticci -s /bin/bash -w KESTIC_CI_JUST_ARGS,KESTIC_CI_WORKSPACE -c '
  set -euo pipefail
  cd "$KESTIC_CI_WORKSPACE"
  export PATH="$HOME/.cargo/bin:$PATH" LANG=en_US.UTF-8 CARGO_INCREMENTAL=0 NEXTEST_PROFILE=ci
  eval "just $KESTIC_CI_JUST_ARGS"
'
