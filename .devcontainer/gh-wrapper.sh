#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause
#
# `gh` in the agent container: reads the token from the Podman secret for this
# call only, so it is neither in the general environment nor in a gh config file.

set -euo pipefail

token_file=/run/secrets/kestic-agent-gh-token
if [[ -r "$token_file" ]]; then
    GH_TOKEN="$(<"$token_file")"
    export GH_TOKEN
fi
exec /usr/bin/gh "$@"
