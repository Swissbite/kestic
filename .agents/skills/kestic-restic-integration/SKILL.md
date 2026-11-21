---
name: kestic-restic-integration
description: Rules, invariants, and test-harness patterns for restic invocation, JSON parsing, exit-code mapping, dual-version integration tests (0.18.0 + latest), SFTP/S3/REST test servers, and secret handling across M1–M6.
---

# Kestic Restic Integration & Test Harness Skill

Use this skill for all stories invoking `restic`, parsing `restic --json` output, handling locks/errors, configuring destinations (Local, REST, SFTP, S3, generic), or managing restic binaries.

## Dual-Version Integration Testing (`AC-M1-Q-2`)

- Every `"Integration"` acceptance criterion must run against **both** `restic 0.18.0` and `restic-latest` installed by `scripts/bootstrap.sh`.
- Always resolve test binaries via `restic_test_binaries()` (`core/tests/common/mod.rs`, `daemon/tests/common/mod.rs`) and `kestic_test_support::tools::rest_server_binary()`, never hardcode `/usr/bin/restic`.
- Repositories in tests come from `kestic_test_support::repo_template::copy_template_repo(restic, dir)`: a copy of a per-run template with its own generated password and repository ID, connected through the production `ConnectExisting` path (`ResticRig::local_repo` in core tests, `add_local_destination`/`M3Rig::add_backup` in daemon tests). Run a real `restic init` only in tests that verify creating a repository.
- Never write a password or credential literal in a test: use `kestic_test_support::password::strong_password()` (shell metacharacters, quotes, inner space, non-ASCII, emoji) and check leaks with `assert_no_secret`, which never prints the secret.
- `key add` / `key passwd` in tests create keys with restic's full (slow) KDF parameters again, and restic tries every key on open; use them only in tests that need them.
- Local test servers bind to random free ports and are cleaned up on `Drop`:
  - **REST (M1+):** `RestServerGuard` (`rest-server` from `bootstrap.sh`); `start_with_auth` generates the password (`server.password()`, `server.repo_dir()`, `server.restart()`).
  - **S3 (M5):** `rclone serve s3` from `bootstrap.sh` (`>= 1.65`) on random port; pass keys via `RCLONE_AUTH_KEY='"<key>,<secret>"'` (CSV-quoted) instead of `--auth-key` in argv.
  - **SFTP (M5):** non-root `sshd` on random port with temporary host/user keys.

## Mandatory Command & Security Invariants

- **Secrets never in argv, logs, config, or command history (`NFR-02`):**
  - Repo password via `RESTIC_PASSWORD_COMMAND="kestic secret get <ziel-id>"` (never `RESTIC_PASSWORD`).
  - Backend credentials (`RESTIC_REST_USERNAME`, `RESTIC_REST_PASSWORD`, S3/B2/env vars) only via child-process environment, never embedded in repo URLs.
  - Password creation/change (`key add`, `M6-S02`, `M6-S06`): write `--new-password-file` under `$XDG_RUNTIME_DIR/kestic/` with permissions `0600` and remove unconditionally on `Drop` (even on error).
- **SSH / SFTP isolation (`NFR-07`, `AC-M1-Q-1`, `P24`):**
  - Every SSH invocation by Kestic must pass `-o UserKnownHostsFile=<path>` via `sftp.args`.
  - In tests, additionally pass `-F <test_ssh_config>` and `-o IdentitiesOnly=yes` because OpenSSH resolves `~` via `/etc/passwd`, not `$HOME`. Real `~/.ssh/known_hosts` and `~/.ssh/config` must remain untouched.
- **Tagging & Grouping (`ADR-0006`, `FR-28`):**
  - Backup snapshots carry tag `kestic:<backup-id>`; metadata snapshots (M6) carry `kestic-meta:<backup-id>`.
  - Always pass `--group-by tags` to `backup` and `forget` so parent detection and retention work across host changes (`AC-M1-07-2`, `AC-M6-04-3`).
- **Safety flags:**
  - **Never** pass `--retry-lock` (`P33`, M3-S04).
  - **Never** pass `--unsafe-allow-remove-all` (`FR-05`, M2-S05).
  - Always pass `--overwrite <never|if-newer|if-changed|always>` explicitly on `restore` (`NFR-08`, M4-S05).
  - Always pass `--no-lock` on dry-run previews (`forget --dry-run --no-lock`, `prune --dry-run --no-lock`, `P41`).
- **Scheduling priority:** Backup runs use `nice` `10` and `ionice` idle class (`AC-M1-03-3`).
- **Time & Clock (`P36`, M1-S18):** Never call `SystemTime::now()` directly in `core/` or `daemon/`; inject `kestic_core::clock::Clock` (`SystemClock` / `FakeClock`).

## Tolerant Parsing & Error Mapping

- All serde structs for `restic --json` (`backup`, `snapshots`, `diff`, `ls`, `forget`, `version`) must use `#[serde(default)]` and ignore unknown fields and unknown `message_type` variants (`NFR-10`, `AC-M1-03-1`).
- Handle snapshots without `summary` (pre-0.17 fixture in `core/tests/fixtures/snapshot_pre_0_17_no_summary.json`, `AC-M4-01-5`).
- Map exit codes (`0`, `1`, `3`, `10`, `11`, `12`, unknown) and `exit_error` stderr JSON strictly according to `docs/spec/restic-integration.md`.
- Note on `chmod 000` tests (M2-S07, M2-S08): tests must not run as `root`, otherwise unreadable-file tests (`Exit 3`) break.

## Release Download & Verification (M5-S06)

- Production builds hardcode GitHub releases URL and restic PGP fingerprint `CF8F 18F2 8445 7597 3F79 D4E1 91A6 868B D3F7 A907`.
- Overrides for release mock tests are gated strictly behind Cargo feature `test-trust-anchor` (active only in tests, never in `.deb` builds or via env vars in release builds).
