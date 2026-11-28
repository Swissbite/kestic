#!/bin/bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause
#
# Egress firewall of the agent container; runs as root at every container start.
#
# dnsmasq is the only resolver. It forwards queries for the domains in
# allowed-domains.txt (and their subdomains) to the upstream resolver of the
# container network and puts every IPv4 address it returns into the nftables
# set `allowed`. Queries for other domains are refused, so nothing resolves
# them. Outgoing traffic is dropped unless it goes to loopback, to an address
# in `allowed` on TCP port 443, or (from dnsmasq only) to the upstream
# resolver on port 53. IPv6 is dropped completely.
#
# Unlike a list of IP addresses resolved once at start, this follows CDNs that
# rotate addresses (static.crates.io, GitHub release downloads).
#
# Fails closed: any error stops the container start.

set -euo pipefail

export PATH=/usr/sbin:/usr/bin:/sbin:/bin

LIB_DIR=/usr/local/lib/kestic-agent
STATE_DIR=/var/lib/kestic-agent
RUN_DIR=/run/kestic-agent
LOG_DIR=/var/log/kestic-agent
DNSMASQ_USER=nobody

mkdir -p "$STATE_DIR" "$RUN_DIR" "$LOG_DIR"
rm -f "$RUN_DIR/ready"

# Podman writes the upstream resolver into /etc/resolv.conf. After the first
# start it points to dnsmasq; a restart may keep that file, so the upstream
# resolver is remembered.
upstream="$(awk '$1 == "nameserver" { print $2; exit }' /etc/resolv.conf)"
if [[ "$upstream" == 127.0.0.1 && -s "$STATE_DIR/upstream" ]]; then
    upstream="$(<"$STATE_DIR/upstream")"
fi
if [[ ! "$upstream" =~ ^[0-9]{1,3}(\.[0-9]{1,3}){3}$ || "$upstream" == 127.0.0.1 ]]; then
    echo "init-firewall: no usable IPv4 nameserver in /etc/resolv.conf" >&2
    exit 1
fi
printf '%s\n' "$upstream" > "$STATE_DIR/upstream"

domain_files=("$LIB_DIR/allowed-domains.txt")
if [[ -f "$LIB_DIR/agent/allowed-domains.txt" ]]; then
    domain_files+=("$LIB_DIR/agent/allowed-domains.txt")
fi
mapfile -t domains < <(grep -hEv '^[[:space:]]*(#|$)' "${domain_files[@]}" | sort -u)
if [[ ${#domains[@]} -eq 0 ]]; then
    echo "init-firewall: no allowed domains configured" >&2
    exit 1
fi
for domain in "${domains[@]}"; do
    if [[ ! "$domain" =~ ^[a-z0-9.-]+$ ]]; then
        echo "init-firewall: invalid domain '$domain' in allowed-domains.txt" >&2
        exit 1
    fi
done

dnsmasq_uid="$(id -u "$DNSMASQ_USER")"

nft -f - <<EOF
table inet kestic_agent
delete table inet kestic_agent
table inet kestic_agent {
    set allowed {
        type ipv4_addr
    }

    chain output {
        type filter hook output priority filter; policy drop;
        oifname "lo" accept
        ct state established,related accept
        meta skuid $dnsmasq_uid ip daddr $upstream udp dport 53 accept
        meta skuid $dnsmasq_uid ip daddr $upstream tcp dport 53 accept
        ip daddr { 10.0.0.0/8, 100.64.0.0/10, 127.0.0.0/8, 169.254.0.0/16, 172.16.0.0/12, 192.168.0.0/16 } reject with icmpx admin-prohibited
        ip daddr @allowed tcp dport 443 accept
        meta l4proto tcp reject with tcp reset
        reject with icmpx admin-prohibited
    }
}
EOF

{
    echo "no-resolv"
    echo "no-poll"
    echo "listen-address=127.0.0.1"
    echo "bind-interfaces"
    echo "user=$DNSMASQ_USER"
    echo "pid-file="
    echo "filter-AAAA"
    echo "log-queries"
    echo "log-facility=$LOG_DIR/dns.log"
    for domain in "${domains[@]}"; do
        echo "server=/$domain/$upstream"
        echo "nftset=/$domain/4#inet#kestic_agent#allowed"
    done
} > "$RUN_DIR/dnsmasq.conf"

pkill -x dnsmasq 2>/dev/null || true
dnsmasq --conf-file="$RUN_DIR/dnsmasq.conf"

printf 'nameserver 127.0.0.1\noptions edns0 trust-ad\n' > /etc/resolv.conf

# Self-test: a domain outside the list must neither resolve nor connect.
if getent ahostsv4 example.com >/dev/null 2>&1; then
    echo "init-firewall: example.com resolves; DNS allowlist is not active" >&2
    exit 1
fi
if curl -sS --max-time 5 -o /dev/null https://1.1.1.1 2>/dev/null; then
    echo "init-firewall: direct connection to an address outside the list succeeded" >&2
    exit 1
fi
if ! curl -fsS --max-time 10 -o /dev/null https://api.github.com/zen; then
    echo "init-firewall: api.github.com is not reachable" >&2
    exit 1
fi
if ! nft list set inet kestic_agent allowed | grep -q 'elements = {'; then
    echo "init-firewall: dnsmasq did not fill the address set" >&2
    exit 1
fi
touch "$RUN_DIR/ready"
echo "init-firewall: egress limited to $(echo "${domains[*]}" | tr ' ' ',')"
