// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![allow(dead_code)]

use std::env;
use std::path::PathBuf;

use kestic_test_support::tools;

/// Returns `(label, binary_path)` pairs for both restic 0.18.0 and latest (`AC-M1-Q-2`).
///
/// Each path is an isolation wrapper (see [`isolated_restic`]): restic child processes always run
/// with `HOME` and all `XDG_*` directories inside `target/tmp`, never in the real user profile.
pub fn restic_test_binaries() -> Vec<(&'static str, PathBuf)> {
    tools::resolve_restic_binaries()
        .into_iter()
        .map(|(label, real)| (label, isolated_restic(&real, label)))
        .collect()
}

/// The wrapped latest restic, for scenarios that need only one binary.
pub fn latest_restic() -> PathBuf {
    restic_test_binaries()
        .into_iter()
        .last()
        .expect("restic binary")
        .1
}

/// Writes (once, atomically) a shell wrapper that points `HOME`, `XDG_CONFIG_HOME`,
/// `XDG_DATA_HOME`, `XDG_CACHE_HOME`, `XDG_STATE_HOME` and `XDG_RUNTIME_DIR` at a temp tree below
/// `CARGO_TARGET_TMPDIR` and then `exec`s the real restic binary (same PID, signals unchanged).
fn isolated_restic(real: &std::path::Path, tag: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("kestic-restic-isolation");
    let home = root.join("home");
    let run = root.join("run");
    for sub in ["home", "cfg", "data", "cache", "state", "run"] {
        std::fs::create_dir_all(root.join(sub)).expect("mkdir restic isolation dir");
    }
    std::fs::set_permissions(&run, std::fs::Permissions::from_mode(0o700))
        .expect("chmod 0700 XDG_RUNTIME_DIR");

    // Override a variable only while it still has the value of the test process (or is unset):
    // tests that hand restic an explicit `HOME` (portable `$HOME` exclude tests) keep theirs.
    let mut script = String::from("#!/bin/sh\n");
    let vars: [(&str, PathBuf); 6] = [
        ("HOME", home),
        ("XDG_CONFIG_HOME", root.join("cfg")),
        ("XDG_DATA_HOME", root.join("data")),
        ("XDG_CACHE_HOME", root.join("cache")),
        ("XDG_STATE_HOME", root.join("state")),
        ("XDG_RUNTIME_DIR", run),
    ];
    for (name, isolated) in &vars {
        let inherited = env::var(name).unwrap_or_default();
        script.push_str(&format!(
            "case \"${{{name}:-}}\" in '{inherited}'|'') export {name}='{isolated}' ;; esac\n",
            isolated = isolated.display()
        ));
    }
    script.push_str(&format!("exec '{}' \"$@\"\n", real.display()));
    let wrapper = root.join(format!("{tag}-isolated.sh"));
    if std::fs::read_to_string(&wrapper).is_ok_and(|existing| existing == script) {
        return wrapper;
    }
    let tmp = root.join(format!("{tag}-isolated.sh.{}", std::process::id()));
    tools::write_executable(&tmp, &script);
    std::fs::rename(&tmp, &wrapper).expect("install restic isolation wrapper");
    wrapper
}

/// Runs a real backup through the production path with the OS mountinfo and `HOME`
/// (the `BackupExecutionContext` the daemon builds, minus its injected inspector).
pub fn execute_backup<F>(
    backup: &kestic_core::model::Backup,
    destination: &kestic_core::model::Destination,
    state_dir: &std::path::Path,
    runtime: &kestic_core::restic::ResticRuntime<'_>,
    cancel: Option<&kestic_core::restic::CancelHandle>,
    extra_env: &[(&str, &str)],
    on_event: F,
) -> Result<kestic_core::restic::ResticExecutionResult, kestic_core::backup::BackupRunError>
where
    F: FnMut(&kestic_core::restic::ResticStreamEvent) + Send,
{
    let home = kestic_core::mount::current_home_dir();
    let ctx = kestic_core::backup::BackupExecutionContext {
        state_dir,
        home_dir: home.as_deref(),
        inspector: &kestic_core::mount::SystemMountInspector,
        cancel,
        extra_env,
    };
    kestic_core::backup::execute_backup_with_inspector(backup, destination, runtime, &ctx, on_event)
}

/// Dry-run counterpart of [`execute_backup`] (production `execute_backup_dry_run_with`).
pub fn execute_backup_dry_run<F>(
    backup: &kestic_core::model::Backup,
    destination: &kestic_core::model::Destination,
    state_dir: &std::path::Path,
    runtime: &kestic_core::restic::ResticRuntime<'_>,
    cancel: Option<&kestic_core::restic::CancelHandle>,
    extra_env: &[(&str, &str)],
    on_event: F,
) -> Result<kestic_core::backup::BackupDryRunResult, kestic_core::backup::BackupRunError>
where
    F: FnMut(&kestic_core::restic::ResticStreamEvent) + Send,
{
    let home = kestic_core::mount::current_home_dir();
    let ctx = kestic_core::backup::BackupExecutionContext {
        state_dir,
        home_dir: home.as_deref(),
        inspector: &kestic_core::mount::SystemMountInspector,
        cancel,
        extra_env,
    };
    kestic_core::backup::execute_backup_dry_run_with(backup, destination, runtime, &ctx, on_event)
}

/// Loads an existing `config.toml` through the production loader (`load_optional_config`).
pub fn load_config(
    path: &std::path::Path,
) -> Result<kestic_core::config::AppConfig, kestic_core::config::ConfigError> {
    kestic_core::config::load_optional_config(path)?.ok_or_else(|| {
        kestic_core::config::ConfigError::Io(std::io::Error::from(std::io::ErrorKind::NotFound))
    })
}

/// Writes an executable shell script emulating `kestic secret get <dest-id> [kind]` backed by
/// a `FileSecretStore` hex file, allowing standalone `cargo test -p kestic-core` runs.
fn write_secret_get_helper(dir: &std::path::Path, secrets_hex_path: &std::path::Path) -> PathBuf {
    let script_path = dir.join("kestic-helper");
    let script = format!(
        r#"#!/usr/bin/env python3
import sys, pathlib
if len(sys.argv) < 3 or sys.argv[1] != "secret" or sys.argv[2] != "get":
    sys.exit(2)
target = sys.argv[3] if len(sys.argv) > 3 else ""
kind = sys.argv[4] if len(sys.argv) > 4 else "repo"
p = pathlib.Path({path:?})
if not p.exists():
    sys.exit(1)
for line in p.read_text(encoding="utf-8").splitlines():
    if "=" not in line:
        continue
    key, hex_val = line.strip().split("=", 1)
    if key == target + ":" + kind:
        sys.stdout.write(bytes.fromhex(hex_val).decode("utf-8"))
        sys.exit(0)
sys.exit(1)
"#,
        path = secrets_hex_path.to_string_lossy()
    );
    tools::write_executable(&script_path, &script);
    script_path
}

/// Returns an isolated per-test `RESTIC_CACHE_DIR` inside `root` so tests never write to `~/.cache/restic`.
pub fn restic_cache_dir(root: &std::path::Path) -> PathBuf {
    let dir = root.join("restic-cache");
    std::fs::create_dir_all(&dir).expect("mkdir restic-cache");
    dir
}

/// Owns everything a [`kestic_core::restic::ResticRuntime`] borrows: the resolved binary, the
/// `kestic secret get` helper, a file secret store, a fake clock and a private restic cache.
pub struct ResticRig {
    pub resolved: kestic_core::provider::ResolvedRestic,
    pub helper: PathBuf,
    pub store: kestic_core::secrets::FileSecretStore,
    pub clock: kestic_core::clock::FakeClock,
    pub cache_dir: PathBuf,
}

impl ResticRig {
    /// Builds the rig below `dir` for `restic_bin` with the fake clock set to `now`.
    pub fn new(
        dir: &std::path::Path,
        restic_bin: &std::path::Path,
        now: time::OffsetDateTime,
    ) -> Self {
        let secrets_hex = dir.join("secrets.hex");
        Self {
            resolved: kestic_core::provider::check_binary_version(restic_bin)
                .expect("resolve restic"),
            helper: write_secret_get_helper(dir, &secrets_hex),
            store: kestic_core::secrets::FileSecretStore::new(&secrets_hex),
            clock: kestic_core::clock::FakeClock::new(now),
            cache_dir: restic_cache_dir(dir),
        }
    }

    pub fn runtime(&self) -> kestic_core::restic::ResticRuntime<'_> {
        self.runtime_with(&self.resolved)
    }

    /// Runtime of this rig that runs `resolved` (e.g. a spy wrapper) instead of its own binary.
    pub fn runtime_with<'a>(
        &'a self,
        resolved: &'a kestic_core::provider::ResolvedRestic,
    ) -> kestic_core::restic::ResticRuntime<'a> {
        kestic_core::restic::ResticRuntime {
            resolved,
            kestic_binary: &self.helper,
            secret_store: &self.store,
            clock: &self.clock,
            cache_dir: Some(&self.cache_dir),
        }
    }

    /// Copies a prepared repository to `repo_dir` (own generated password, own repository ID)
    /// and connects it as local destination `name` through the production setup path
    /// (`ConnectExisting`, which stores the password in this rig's secret store). Returns the
    /// destination and its password.
    pub fn local_repo(
        &self,
        name: &str,
        repo_dir: &std::path::Path,
    ) -> (kestic_core::model::Destination, String) {
        let repo =
            kestic_test_support::repo_template::copy_template_repo(&self.resolved.binary, repo_dir);
        let dest = kestic_core::model::Destination::new(
            name,
            kestic_core::model::DestinationKind::Local {
                path: repo_dir.to_path_buf(),
            },
        )
        .expect("local destination");
        kestic_core::destination::setup_destination(
            &dest,
            kestic_core::destination::DestinationSetupMode::ConnectExisting,
            &kestic_core::secrets::SecretValue::from_str_secret(&repo.password),
            None,
            &self.runtime(),
        )
        .unwrap_or_else(|e| {
            panic!(
                "connecting the local repository {} with {} failed: {e}",
                repo_dir.display(),
                self.resolved.binary.display()
            )
        });
        (dest, repo.password)
    }
}
