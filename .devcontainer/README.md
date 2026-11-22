<!--
SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Agent container

A rootless Podman container in which a coding agent (Claude Code) works on
Kestic on its own, from a GitHub issue to a pull request. It is based on the CI
image (`.github/ci-image/Dockerfile`), so `just verify` runs with the same
system packages as the `verify` job.

The agent talks to the outside world only through GitHub: it gets its
assignment from an issue, reports progress as issue comments and delivers a
pull request. A maintainer reviews and merges; the agent never does.

## Isolation

| Host resource | In the container |
| --- | --- |
| Repository checkout | not mounted; the agent clones `Swissbite/kestic` into the volume `kestic-agent-home` |
| GitHub credentials | only a fine-grained token for this repository, as the Podman secret `kestic-agent-gh-token` (file `/run/secrets/kestic-agent-gh-token`, read per call by `gh` and git, never in argv, environment, image or config files) |
| Home directory, keyring, D-Bus, SSH agent | not mounted, no sockets, no environment variables passed |
| Network | IPv4 HTTPS to the domains in [allowed-domains.txt](allowed-domains.txt) only; no host loopback, no LAN, no IPv6 |
| User | the agent runs as `kesticci` (uid 1001 in the container, an unprivileged sub-uid on the host) without capabilities and with `no_new_privs`; `sudo` does not work |
| Claude Code login | the volume `kestic-agent-claude` (`CLAUDE_CONFIG_DIR`) |

The entrypoint starts as container root with five capabilities
(`NET_ADMIN`, `NET_BIND_SERVICE`, `SETUID`, `SETGID`, `SETPCAP`), sets up the
firewall and drops to `kesticci` with an empty bounding set. Container root is
the host user only inside the user namespace; it has no capabilities on the host.

### Firewall

[init-firewall.sh](init-firewall.sh) follows the idea of the
[Claude Code reference dev container](https://github.com/anthropics/claude-code/tree/main/.devcontainer)
(default deny, allowlist, self-test at start) with these changes:

- **dnsmasq instead of addresses resolved once:** dnsmasq resolves only the
  allowlisted domains and their subdomains and adds each returned address to an
  nftables set. CDNs with rotating addresses (crates.io, GitHub release
  downloads) keep working, and other names do not resolve at all, which also
  closes DNS as a channel to arbitrary servers.
- **nftables** directly (works rootless; modules load on demand) instead of
  iptables + ipset; no GitHub meta ranges needed.
- **No host network, no port 22:** the reference allows the host's /24 network
  and outgoing SSH. Here only port 443 to the set and port 53 from dnsmasq to the
  upstream resolver are open; git uses HTTPS.
- **The agent cannot change the rules:** the reference gives the agent user
  `sudo` for the firewall script. Here the firewall is set up before the drop
  to `kesticci`, and nothing in the container can become root again.

Limits: the filter works on IP addresses. A CDN address that serves an allowed
domain also serves other sites on that address. Subdomains of allowed domains
resolve too (e.g. any `*.githubusercontent.com`). The allowlist does not stop
the agent from sending data to GitHub or Anthropic, which it must reach anyway.
`kestic-agent.sh dns-log` shows every query, refused ones included.

### What the token can and cannot do

The fine-grained token is limited to this repository: contents, pull requests
and issues read/write; checks, commit statuses, actions and metadata read. It
has no administration and no workflow permission, so the agent cannot change
branch protection, secrets or `.github/workflows/`. It belongs to the
maintainer's account, so GitHub shows the agent's commits, comments and pull
requests under that account. Branch protection on `main` (required `verify`)
applies to it like to any push. It **can** merge a pull request once `verify` is
green; Claude Code settings ([managed-settings.json](managed-settings.json)), a
`pre-push` hook and the assignment forbid it, but these are rules for the
agent, not a hard limit.

No AI attribution: the managed settings switch off Claude Code's
`Co-Authored-By` and "Generated with" lines, and a `commit-msg` hook rejects
such trailers.

## One-time setup (maintainer)

1. Build the image (about 1 min plus the CI image download):

   ```bash
   .devcontainer/kestic-agent.sh build
   ```

2. Create a fine-grained token on GitHub (Settings → Developer settings →
   Fine-grained tokens): resource owner `Swissbite`, only the repository
   `Swissbite/kestic`, expiry about 90 days; permissions as above. Store it as a
   Podman secret without it appearing in argv or the shell history:

   ```bash
   read -rsp 'Token: ' t && printf '%s' "$t" | podman secret create kestic-agent-gh-token - ; unset t; echo
   ```

   To rotate it: `podman secret rm kestic-agent-gh-token`, create it again, then
   `kestic-agent.sh stop` and start again.

3. Log in to Claude Code once (subscription, no API key):

   ```bash
   .devcontainer/kestic-agent.sh login
   ```

   The first start clones the repository and installs toolchain and test tools
   (several minutes). Claude Code then asks for the login method: choose the
   Claude account with subscription, open the shown URL in the browser on the
   host, sign in, paste the code back, accept the questions and leave with
   `/exit`. The login stays in the volume `kestic-agent-claude`.

## Working on an issue

```bash
.devcontainer/kestic-agent.sh start 42      # container + agent on issue #42
.devcontainer/kestic-agent.sh status        # resources and the agent's last terminal lines
.devcontainer/kestic-agent.sh attach        # watch or step in (detach: Ctrl-b d)
.devcontainer/kestic-agent.sh stop          # stop; volumes stay
.devcontainer/kestic-agent.sh resume 42     # continue the last session after a stop
.devcontainer/kestic-agent.sh nudge 'Please read the new comment on the pull request.'
```

The agent runs `claude --dangerously-skip-permissions` in the tmux session
`agent`, with a fixed prompt that points it to the issue (see
[kestic-agent-inner.sh](kestic-agent-inner.sh)). Its sessions are stored in
`kestic-agent-claude`, so `resume` works after a container restart.

From the host, without entering the container:

```bash
gh issue view 42 --repo Swissbite/kestic --comments
gh pr list --repo Swissbite/kestic --search 'linked:issue 42'
gh pr checks <pr> --repo Swissbite/kestic
```

`kestic-agent.sh exec just verify` runs any command in the clone, e.g. to check
the container itself.

## Dev Container

[devcontainer.json](devcontainer.json) describes the same container for tools
that support the Dev Container standard (VS Code, JetBrains, the devcontainer
CLI) with Podman as the container engine. The Podman secret and the firewall
come along. The default CI image tag in it and in the
[Containerfile](Containerfile) must follow `.github/ci-image/tag.sh`;
`kestic-agent.sh build` always uses the current one and warns when the default
is stale.

## Other agents

The container does not depend on Claude Code except for these parts:

- **Codex CLI:** install it in the Containerfile instead of (or next to)
  Claude Code; add its API host to `allowed-domains.txt`; log in with
  `codex login --device-auth` into a volume for `~/.codex`; run it in the tmux
  session with `codex exec --full-auto` (or the interactive TUI). It reads
  `AGENTS.md` directly. The attribution settings do not apply, the git hook does.
- **Antigravity:** an IDE agent. It would attach to the container as a Dev
  Container (devcontainer.json) instead of running headless in tmux; its model
  endpoints go into `allowed-domains.txt`. Long unattended sessions are not its
  normal mode of operation.

## Cleaning up

```bash
.devcontainer/kestic-agent.sh clean --yes       # container, image, both volumes (also the login)
podman secret rm kestic-agent-gh-token
podman rmi ghcr.io/swissbite/kestic-ci:<tag>    # the base image
```
