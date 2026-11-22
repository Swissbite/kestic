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
| GitHub credentials | only a one-hour installation token of the GitHub App `kestic-agent`, limited to this repository, in `/run/kestic-agent-token/gh-token` (tmpfs, owner `kesticci`, mode 0400; read per call by `gh` and git, never in argv, environment, image or volume). The App's private key stays on the host. |
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

### Identity and token

The agent acts as the GitHub App `kestic-agent`, a private App of the
maintainer installed only on this repository. Its permissions: contents, pull
requests and issues read/write; actions, checks, commit statuses and metadata
read. No administration, no workflows, no webhook: the agent cannot change
rulesets, secrets or `.github/workflows/`. GitHub shows its pull requests,
comments, labels and merges as `kestic-agent[bot]`; its commits carry the
maintainer as author ([agent.conf](agent.conf)), who is responsible for them.

The App's private key never enters the container. It is the Podman secret
`kestic-agent-app-key` on the host. [kestic-agent.sh](kestic-agent.sh) signs a
JSON Web Token with it (openssl, as in GitHub's «Generating a JSON Web Token
(JWT) for a GitHub App»), asks GitHub for an installation token limited to this
repository and writes it into the running container through stdin
(`kestic-agent store-token`). An installation token lives one hour;
`kestic-agent.sh` renews it when the container starts, before each command that
uses the container and whenever the watcher asks for the agent's state, as
soon as it is 40 minutes old. Without the watcher timer, `kestic-agent.sh token`
renews it by hand. JWT and token never appear in an argument list; curl reads
the authorization header from a file descriptor. Tested by
[test-kestic-agent-token.sh](test-kestic-agent-token.sh) in `just verify`.

The ruleset on `main` restricts updates to the maintainer and Renovate, so the
App cannot merge into `main`. The container adds a second guard against
mistakes: [gh-wrapper.sh](gh-wrapper.sh) rejects `gh pr merge` and a PUT on the
REST endpoint `…/pulls/<n>/merge` when the pull request targets `main` (tested
by [test-gh-wrapper.sh](test-gh-wrapper.sh)), and a `pre-push` hook rejects
pushes to `main`. Merges into other branches, such as story pull requests
against a milestone branch, go through.

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

2. Create the GitHub App (Settings → Developer settings → GitHub Apps → New
   GitHub App):
   - name `kestic-agent`, homepage the repository URL;
   - webhook **off**;
   - repository permissions: Contents, Issues and Pull requests read and write;
     Actions, Checks and Commit statuses read (Metadata read is implied); no
     other permission;
   - «Only on this account».

   Note its App ID and put it into [agent.conf](agent.conf) (`app_id=`). Install
   the App on `Swissbite/kestic` only («Only select repositories»). Generate a
   private key; the browser downloads a `.pem` file. Store it as a Podman secret
   and delete the download:

   ```bash
   podman secret create --replace kestic-agent-app-key ~/Downloads/kestic-agent.*.private-key.pem
   shred -u ~/Downloads/kestic-agent.*.private-key.pem
   ```

   To rotate the key, generate a new one on the App's page, store it the same
   way and delete the old key there. A container that was started with a
   personal access token (`kestic-agent-gh-token`) has to be stopped and
   started again; then remove that secret (`podman secret rm
   kestic-agent-gh-token`) and revoke the token on GitHub.

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
.devcontainer/kestic-agent.sh token         # a new GitHub token now (otherwise renewed when due)
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
  stays the only one. Something the maintainer did after the agent's last turn
  started continues the idle session: a comment or review on the issue or on a
  pull request from its branch, merging that pull request, or removing
  `needs-maintainer`. Only people with write access count, never bots or
  outsiders: the repository is public, and a comment must not wake an agent
  that runs without permission prompts. A session that is gone (Claude Code
  exited, the container restarted) is resumed, at most twice until it reaches
  a turn again.
- Otherwise it starts the open issue of the milestone with the lowest number
  that is not labelled `needs-maintainer` and has no open «blocked by» issue.
  New issues therefore queue at the end; a «blocked by» link changes the order.

Busy or idle comes from two Claude Code hooks in
[managed-settings.json](managed-settings.json): a prompt starts a turn, the end
of a turn marks the session idle (`kestic-agent.sh agent-state`). A session
whose terminal has not changed for 15 minutes counts as idle too, because the
Stop hook does not run after an interrupt or an API error. The watcher treats
the state line from the container as untrusted input.

The watcher logs what it does and, where `notify-send` exists, tells the
desktop once: an issue that waits for the maintainer, a session that does not
come back after two resumes, an agent idle for four hours without a result, and
(once an hour) a failed renewal of the agent's GitHub token.
[test-kestic-agent-watch.sh](test-kestic-agent-watch.sh) in `just verify` tests
its decisions with a fake `gh`; after changing a `--jq` filter, run one pass
with `KESTIC_AGENT_SH` pointing to a script that prints a state line, against
the real milestone.

Run it every five minutes with a systemd user timer (`<checkout>` is the
host checkout; `gh` on the host only reads issues and pull requests; the
watcher also keeps the agent's GitHub token fresh):

```ini
# ~/.config/systemd/user/kestic-agent-watch.service
[Service]
Type=oneshot
TimeoutStartSec=30min
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
CLI) with Podman as the container engine. The firewall and the token directory
come along; the token comes from the host with
`KESTIC_AGENT_NAME=<container name> .devcontainer/kestic-agent.sh token`, and the
git author from `KESTIC_AGENT_GIT_NAME` and `KESTIC_AGENT_GIT_EMAIL` in the host
environment. The default CI image tag in it and in the
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
podman secret rm kestic-agent-app-key           # the App's key; delete the key on GitHub too
podman rmi ghcr.io/swissbite/kestic-ci:<tag>    # the base image
```
