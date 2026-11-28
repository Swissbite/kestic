<!--
SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Agent backends

Each subdirectory under `.devcontainer/agents/<name>/` defines one coding-agent
variation that runs inside the shared Kestic isolation container (CI base image,
egress firewall, GitHub App token, GitHub text trust boundary, git hooks,
watcher).

Shipped variations:

- `claude` (default): Claude Code (`localhost/kestic-agent:claude`, also tagged
  `:latest`; volume `<name>-claude` mounted at `/home/kesticci/.claude`).
- `antigravity`: Antigravity CLI (`agy`; `localhost/kestic-agent:antigravity`;
  volume `<name>-gemini` mounted at `/home/kesticci/.gemini`).

## Reusing an existing variation

Select a variation with `KESTIC_AGENT_BACKEND` (or `backend=` in
`.devcontainer/agent.conf`):

```bash
export KESTIC_AGENT_BACKEND=antigravity
export KESTIC_AGENT_NAME=kestic-agent-2
export KESTIC_AGENT_MODEL=gemini-4-carbon-high   # optional
.devcontainer/kestic-agent.sh build
.devcontainer/kestic-agent.sh login
.devcontainer/kestic-agent.sh start 42 m6
```

`.devcontainer/kestic-agent.sh backends` lists all available variations and
whether their image is built.

## Adding a new variation

Other systems or repositories can add a variation without modifying
`Containerfile`, `init-firewall.sh`, `kestic-agent.sh`, `kestic-agent-inner.sh`
or `kestic-agent-watch.sh`:

- **In-tree:** create `.devcontainer/agents/<name>/`.
- **Out-of-tree:** create `<dir>/<name>/` and set
  `KESTIC_AGENT_BACKENDS_DIR=<dir> KESTIC_AGENT_BACKEND=<name>`.
  `kestic-agent.sh build` stages the external variation into a temporary build
  context automatically.

### Required files in `<name>/`

1. **`backend.conf`** (plain `key=value`, never sourced on the host):
   - `description=` human-readable label (shown by `kestic-agent.sh backends`).
   - `volume_suffix=` suffix for the agent's persistent login/state volume
     (`<name>-<volume_suffix>`, `[a-z0-9._-]+`).
   - `volume_mount=` mount path inside `/home/kesticci/` (e.g.
     `/home/kesticci/.codex`, no `..`).
   - `version_arg=` optional build-arg name forwarded from the host environment
     (e.g. `CLAUDE_CODE_VERSION`, `AGY_VERSION`); `AGENT_VERSION` is always
     forwarded when set.
   - Optional `leitstand_*_pattern=` keys documenting console patterns for
     `.agents/skills/leitstand/watch.sh` (empty disables a check).

2. **`install.sh`** (runs as `root` during `podman build`):
   - Installs the agent binary into `/usr/local/bin/` with checksum verification.
   - Creates `volume_mount` owned by `kesticci:kesticci` so a new Podman volume
     inherits the right ownership.

3. **`allowed-domains.txt`** (one domain per line, `#` comments allowed):
   - Only the domains this agent needs at runtime (API, OAuth, Remote Control).
   - Merged with `.devcontainer/allowed-domains.txt` by `init-firewall.sh`, so
     each container can reach only its own provider.

4. **`driver.sh`** (sourced by `kestic-agent-inner.sh` as `kesticci`):
   - `AGENT_DISPLAY_NAME`: name used in log messages.
   - `agent_setup`: called by `kestic-agent setup` (e.g. pre-trust workspace,
     install lifecycle hooks that call `kestic-agent hook turn|idle`).
   - `agent_login`: called by `kestic-agent login` (`exec` interactive login).
   - `agent_launch_cmd <start|resume> <issue> <prompt>`: populates the bash
     array `AGENT_CMD` launched inside tmux session `agent`. Respects
     `KESTIC_AGENT_MODEL`, `KESTIC_AGENT_EFFORT`, and
     `KESTIC_AGENT_REMOTE_CONTROL` (`0` disables) when applicable.
   - `agent_limit_reset <turn_epoch> <pane_text>`: prints the usage-limit reset
     epoch, or `0` if not applicable.
   - Optional `agent_nudge <text>`: overrides the default `tmux send-keys` nudge.

Any extra files in `<name>/` (such as `settings.json`, `hooks.json` or
`hook.sh`) are copied into `/usr/local/lib/kestic-agent/agent/` (root-owned,
available as `$AGENT_DIR` inside `driver.sh`).
