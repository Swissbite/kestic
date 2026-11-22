#!/bin/bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause
#
# Container entrypoint (as root): sets up the egress firewall, then runs the
# command as `kesticci`.
#
#   entrypoint.sh <command…>       firewall, then <command> as kesticci
#   entrypoint.sh --drop <command…> only drop to kesticci (for `podman exec`)
#
# Dropping clears all capabilities including the bounding set and sets
# no_new_privs, so neither the agent nor anything it starts can change the
# firewall or become root again.

set -euo pipefail

# Root must not run anything from the home volume, which the agent can write
# (the image PATH starts with ~/.cargo/bin).
export PATH=/usr/sbin:/usr/bin:/sbin:/bin

if [[ "$(id -u)" != 0 ]]; then
    echo "entrypoint: must start as root (it sets up the firewall, then drops to kesticci)" >&2
    exit 1
fi

if [[ "${1:-}" == --drop ]]; then
    shift
else
    /usr/local/lib/kestic-agent/init-firewall.sh
fi

# Optionally lower the priority of everything the agent runs (raising nice needs
# no capability). Off by default: Kestic's tests check the niceness of restic.
exec nice -n "${KESTIC_AGENT_NICE:-0}" setpriv --reuid=kesticci --regid=kesticci --init-groups \
    --inh-caps=-all --bounding-set=-all --no-new-privs -- \
    env HOME=/home/kesticci USER=kesticci LOGNAME=kesticci SHELL=/bin/bash \
    PATH=/home/kesticci/.cargo/bin:/usr/local/bin:/usr/bin:/bin "$@"
