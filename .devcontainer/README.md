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
pull request. A maintainer reviews and merges into `main`; the agent never
does. In a milestone run the agent merges its own story pull requests into the
milestone's integration branch under that milestone's rules
([Milestone runs](#milestone-runs)).

## Isolation

| Host resource | In the container |
| --- | --- |
| Repository checkout | not mounted; the agent clones `Swissbite/kestic` into the volume `kestic-agent-home` |
| GitHub credentials | only a fine-grained token for this repository, as the Podman secret `kestic-agent-gh-token` (file `/run/secrets/kestic-agent-gh-token`, read per call by `gh` and git, never in argv, environment, image or config files) |
| Home directory, keyring, D-Bus, SSH agent | not mounted, no sockets, no environment variables passed |
| Network | IPv4 HTTPS to the domains in [allowed-domains.txt](allowed-domains.txt) only; no host loopback, no LAN, no IPv6 |
| User | the agent runs as `kesticci` (uid 1001 in the container, an unprivileged sub-uid on the host) without capabilities and with `no_new_privs`; `sudo` does not work |
| Claude Code login | the volume `kestic-agent-claude` (`CLAUDE_CONFIG_DIR`) |

The entrypoint starts as container root with six capabilities
(`NET_ADMIN` for nftables, `NET_BIND_SERVICE` for dnsmasq on port 53, `SETUID`
and `SETGID` to change users, `SETPCAP` to empty the bounding set, `KILL` so
that `podman stop` reaches the agent's processes), sets up the firewall and
drops to `kesticci` with an empty bounding set. Container root is
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

Private address ranges are rejected even when an allowed name resolves to one.
`kestic-agent.sh dns-log` shows every query, refused ones included (kept in a
16 MB tmpfs).

Limits, by design of an IP filter:

- **Shared CDN edges:** an address that serves an allowed domain (Cloudflare
  for claude.ai, Fastly/CloudFront for crates.io and rustup) also serves other
  sites. An agent that wants to can reach such a site with
  `curl --resolve other.example:443:<allowed address>`. The allowlist therefore
  stops accidental and casual traffic, not a determined exfiltration. Closing
  this needs an HTTPS proxy that checks the server name (SNI); not done here.
- Subdomains of allowed domains resolve too (e.g. any `*.githubusercontent.com`).
- The agent can always send data to GitHub and Anthropic, which it must reach.
- dnsmasq keeps `NET_ADMIN` to fill the address set; a remote-code-execution bug
  in dnsmasq reachable from the agent would lift the firewall.

### What the token can and cannot do

The fine-grained token is limited to this repository: contents, pull requests
and issues read/write; checks, commit statuses, actions and metadata read. It
has no administration and no workflow permission, so the agent cannot change
branch protection, secrets or `.github/workflows/`. It belongs to the
maintainer's account, so GitHub shows the agent's commits, comments and pull
requests under that account. The ruleset on `main` applies to it: no direct
push or force push, changes only through a pull request with a green `verify`.
The ruleset needs no approving review, so the token **can** merge a pull
request once `verify` is green.

The container has one rule against that: merging into `main` is refused.
[gh-wrapper.sh](gh-wrapper.sh) rejects `gh pr merge` and a PUT on the REST
endpoint `…/pulls/<n>/merge` when the pull request targets `main` (tested by
[test-gh-wrapper.sh](test-gh-wrapper.sh) in `just verify`). Merges into other
branches, such as story pull requests against a milestone branch, and all
other commands go through; texts that mention merge or main never block
anything. A `pre-push` hook rejects pushes to `main`. Both guard against
mistakes only. Known gaps: GraphQL merge mutations, the branch merge API,
gh aliases, the token with `curl`, `--no-verify`, and a pull request with
`--auto` that GitHub later retargets to `main`.

The binding lock belongs in the ruleset on `main`. Since the token is the
maintainer's own, a required review would also block the maintainer, who
cannot approve their own pull requests; it needs a second identity (a bot
account for the agent, or a second reviewer) or a bypass for the maintainer
that the agent's token does not have.

Podman keeps secrets unencrypted in its storage on the host
(`~/.local/share/containers/storage/secrets/`), readable by the host user only.

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
   bash -c 'read -rsp "Token: " t && printf %s "$t" | podman secret create --replace kestic-agent-gh-token -; echo'
   ```

   Run it as shown, through `bash`: zsh's `read -p` means something else and
   would store a wrong value. `--replace` also rotates an existing token; then
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
.devcontainer/kestic-agent.sh start 51 m4   # same, branch and pull request on m4 instead of main
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

## Milestone runs

For a milestone (M4–M6) the agent works through the issues of a GitHub
milestone on the integration branch `m<n>`, one issue after another, each in a
fresh session. The rules (when it merges its own story pull request into
`m<n>`, protected paths, questions to the maintainer) are in
`.agents/skills/kestic-milestone-autonomy/SKILL.md` on that branch. The
maintainer asks for a decision by the label `needs-maintainer` and answers in a
comment.

[kestic-agent-watch.sh](kestic-agent-watch.sh) runs on the host and makes one
pass per call:

```bash
.devcontainer/kestic-agent-watch.sh "M4 Restore" m4
```

- The container is off or the agent is busy: nothing. The watcher never starts
  the container; stopping it pauses the milestone.
- The agent's current issue is open and not labelled `needs-maintainer`: it
  stays the only one. A new comment on the issue or on its pull request
  continues the idle session; a session that is gone (Claude Code exited, the
  container restarted) is resumed.
- Otherwise it starts the open issue of the milestone with the lowest number
  that is not labelled `needs-maintainer` and has no open «blocked by» issue.
  New issues therefore queue at the end; a «blocked by» link changes the order.

Busy or idle comes from two Claude Code hooks in
[managed-settings.json](managed-settings.json): a prompt starts a turn, the end
of a turn marks the session idle (`kestic-agent.sh agent-state`). The agent
writes with the maintainer's token, so it ends every comment with
`<!-- kestic-agent -->`; only comments without that mark and newer than the
agent's last turn count as new.

Run it every five minutes with a systemd user timer (`<checkout>` is the
host checkout; `gh` on the host only reads issues and pull requests):

```ini
# ~/.config/systemd/user/kestic-agent-watch.service
[Service]
Type=oneshot
ExecStart=<checkout>/.devcontainer/kestic-agent-watch.sh "M4 Restore" m4

# ~/.config/systemd/user/kestic-agent-watch.timer
[Timer]
OnActiveSec=1min
OnUnitInactiveSec=5min

[Install]
WantedBy=timers.target
```

```bash
systemctl --user daemon-reload && systemctl --user enable --now kestic-agent-watch.timer
journalctl --user -u kestic-agent-watch      # what it started or continued
```

No ready-made tool fits: reacting to issue events with GitHub Actions needs
either a self-hosted runner on this machine, which GitHub advises against for
public repositories because pull requests from forks can run code on it, or the
agent on GitHub's runners with an API key instead of this container; webhooks
need an endpoint reachable from GitHub. A timer and `gh` need neither.

## Dev Container

[devcontainer.json](devcontainer.json) describes the same container for tools
that support the Dev Container standard (VS Code, JetBrains, the devcontainer
CLI) with Podman as the container engine. The Podman secret and the firewall
come along. The default CI image tag in it and in the
[Containerfile](Containerfile) must follow `.github/ci-image/tag.sh`;
`kestic-agent.sh build` always uses the current one and warns when the default
is stale.

Run the agent with `kestic-agent.sh`, not through an IDE: Dev Container tools
add host integration that this container is meant to keep out. VS Code, for
example, copies the host's `~/.gitconfig` and forwards the SSH agent, git
credentials and GPG into the container unless switched off in its settings
(`dev.containers.copyGitConfig`, `dev.containers.gitCredentialHelperConfigLocation`
set to `none`, no SSH agent forwarding). Processes started by the tool run as
`kesticci` without capabilities, but outside the entrypoint, so without its
optional `nice` and with the container's bounding set (`no_new_privs` still applies).

## Resources

Measured on a 20-thread laptop (2026-11): `just verify` in the container took
6:56 min from a fresh clone (toolchain already installed) and 2:57 min warm, with
the same 381 tests as on the host; the container's memory peak (including page
cache) was 9–11 GB. Bootstrap of toolchain and test tools in a new home volume:
1:35 min. Disk: image 2.4 GB (including the 2.0 GB CI image), home volume about
6 GB after one build, login volume under 1 MB.

`just verify` compiles with all host cores. The container
runs with a low CPU weight (`--cpu-shares=256`), which keeps the desktop
responsive without slowing the agent on an idle host. Optional limits:
`KESTIC_AGENT_CPUS` (cores; cargo and nextest then use as many jobs),
`KESTIC_AGENT_MEMORY` (default `24g`), `KESTIC_AGENT_NICE` (default 0; with
another value the test that checks restic's niceness fails, because nice adds up).

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
