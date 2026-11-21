// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Destination initialization, connection verification, password updates, and preflight reachability checks.

use crate::clock::Clock;
use crate::model::{Destination, DestinationKind, normalize_rest_url_to_ascii};
use crate::provider::ResolvedRestic;
use crate::restic::{
    ResticCommandSpec, ResticExecutionResult, ResticFailureKind, ResticRunError, ResticRuntime,
    SECRET_KIND_REPO, SECRET_KIND_REST,
};
use crate::secrets::{SecretError, SecretStore, SecretValue};
use std::fs;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;
use std::process::Command;
use std::time::Duration;
use thiserror::Error;

/// Default preflight connect timeout (10 s connect, overall failure <= 15 s).
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Errors returned during destination preflight, initialization, connection, or password verification.
#[derive(Debug, Error)]
pub enum DestinationError {
    #[error("destination unreachable: {0}")]
    Unreachable(String),
    #[error("authentication at destination failed: {0}")]
    AuthFailed(String),
    #[error("wrong repository password")]
    WrongPassword,
    #[error("repository missing: {0}")]
    RepositoryMissing(String),
    #[error("password store error: {0}")]
    Secret(#[from] SecretError),
    #[error("restic invocation failed: {0}")]
    Runner(#[from] ResticRunError),
    #[error("restic failed (exit {exit_code}): {message}")]
    CommandFailed { exit_code: i32, message: String },
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Mode used when setting up a [`Destination`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DestinationSetupMode {
    /// Create a brand new restic repository (`restic init`).
    InitNew,
    /// Connect to an existing restic repository without modifying it (`restic cat config`).
    ConnectExisting,
}

/// Checks whether `dest` is reachable before launching restic.
///
/// - `Local`: checks that the path exists and is a directory.
/// - `Rest`: performs TCP connect + HTTP request within `connect_timeout` (default 10 s).
///   Any HTTP response (including 401/403/404) proves TCP+HTTP reachability.
pub fn preflight_check_destination(
    dest: &Destination,
    connect_timeout: Duration,
) -> Result<(), DestinationError> {
    match &dest.kind {
        DestinationKind::Local { path } => {
            if !path.exists() {
                return Err(DestinationError::Unreachable(format!(
                    "folder does not exist: {}",
                    path.display()
                )));
            }
            if !path.is_dir() {
                return Err(DestinationError::Unreachable(format!(
                    "path is not a folder: {}",
                    path.display()
                )));
            }
            Ok(())
        }
        DestinationKind::Rest { url, .. } => {
            let ascii_url = normalize_rest_url_to_ascii(url)
                .map_err(|err| DestinationError::Unreachable(err.to_string()))?;

            let tls_config = ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::NativeTls)
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build();
            let config = ureq::Agent::config_builder()
                .tls_config(tls_config)
                .timeout_connect(Some(connect_timeout))
                .timeout_global(Some(connect_timeout + Duration::from_secs(2)))
                .http_status_as_error(false)
                // Any HTTP status (including 3xx redirects, which may carry unescaped/IDN Location
                // headers) proves TCP+TLS+HTTP reachability for preflight.
                .max_redirects(0)
                .max_redirects_will_error(false)
                .build();
            let agent = ureq::Agent::new_with_config(config);

            agent
                .get(&ascii_url)
                .call()
                .map(|_| ())
                .map_err(|err| DestinationError::Unreachable(err.to_string()))
        }
    }
}

/// Sets up a destination (either `init` or `connect`) and persists secrets ONLY on success.
///
/// Uses `--password-file` inside a private `0700` temp directory during the initial probe/init so
/// `SecretStore` is never polluted if verification fails,
/// and no password ever appears in `/proc/<pid>/cmdline`.
pub fn setup_destination(
    dest: &Destination,
    mode: DestinationSetupMode,
    repo_password: &SecretValue,
    rest_password: Option<&SecretValue>,
    runtime: &ResticRuntime<'_>,
) -> Result<ResticExecutionResult, DestinationError> {
    preflight_check_destination(dest, DEFAULT_CONNECT_TIMEOUT)?;

    let res = run_with_temp_repo_password_file(
        dest,
        mode_subcommand(mode),
        repo_password,
        rest_password,
        ProbeRuntime {
            resolved: runtime.resolved,
            kestic_binary: runtime.kestic_binary,
            clock: runtime.clock,
            cache_dir: runtime.cache_dir,
        },
    )?;

    if res.exit_code != 0 {
        return Err(map_execution_failure(&res));
    }

    // Persist secrets only after restic succeeded.
    runtime
        .secret_store
        .set_secret(dest.id.as_str(), SECRET_KIND_REPO, repo_password)?;
    if let Some(rest_pw) = rest_password
        && let Err(err) =
            runtime
                .secret_store
                .set_secret(dest.id.as_str(), SECRET_KIND_REST, rest_pw)
    {
        let _ = runtime
            .secret_store
            .delete_secret(dest.id.as_str(), SECRET_KIND_REPO);
        return Err(err.into());
    }

    Ok(res)
}

/// Verifies `candidate_password` against an existing repository without running `restic key`,
/// and updates the stored secret only if verification succeeds.
pub fn verify_and_update_stored_password(
    dest: &Destination,
    candidate_password: &SecretValue,
    secret_store: &dyn SecretStore,
    resolved: &ResolvedRestic,
    kestic_binary: &Path,
    clock: &dyn Clock,
) -> Result<(), DestinationError> {
    verify_and_update_stored_password_with_cache(
        dest,
        candidate_password,
        secret_store,
        resolved,
        kestic_binary,
        clock,
        None,
    )
}

/// Like [`verify_and_update_stored_password`] with explicit `RESTIC_CACHE_DIR` isolation.
pub fn verify_and_update_stored_password_with_cache(
    dest: &Destination,
    candidate_password: &SecretValue,
    secret_store: &dyn SecretStore,
    resolved: &ResolvedRestic,
    kestic_binary: &Path,
    clock: &dyn Clock,
    cache_dir: Option<&Path>,
) -> Result<(), DestinationError> {
    preflight_check_destination(dest, DEFAULT_CONNECT_TIMEOUT)?;

    let rest_pw = if dest.kind.uses_rest_credentials() {
        secret_store.get_secret(dest.id.as_str(), SECRET_KIND_REST)?
    } else {
        None
    };

    let res = run_with_temp_repo_password_file(
        dest,
        vec![
            "cat".to_string(),
            "--no-lock".to_string(),
            "config".to_string(),
        ],
        candidate_password,
        rest_pw.as_ref(),
        ProbeRuntime {
            resolved,
            kestic_binary,
            clock,
            cache_dir,
        },
    )?;

    if res.exit_code != 0 {
        return Err(map_execution_failure(&res));
    }

    secret_store.set_secret(dest.id.as_str(), SECRET_KIND_REPO, candidate_password)?;
    Ok(())
}

/// Removes all Secret Service entries belonging to `dest`.
pub fn remove_destination_secrets(
    dest: &Destination,
    secret_store: &dyn SecretStore,
) -> Result<(), SecretError> {
    secret_store.delete_secret(dest.id.as_str(), SECRET_KIND_REPO)?;
    secret_store.delete_secret(dest.id.as_str(), SECRET_KIND_REST)?;
    Ok(())
}

fn mode_subcommand(mode: DestinationSetupMode) -> Vec<String> {
    match mode {
        DestinationSetupMode::InitNew => vec!["init".to_string()],
        DestinationSetupMode::ConnectExisting => {
            vec![
                "cat".to_string(),
                "--no-lock".to_string(),
                "config".to_string(),
            ]
        }
    }
}

fn map_execution_failure(res: &ResticExecutionResult) -> DestinationError {
    let detail = res.presentation.detail_or_summary();
    match res.presentation.kind {
        ResticFailureKind::WrongPassword => DestinationError::WrongPassword,
        ResticFailureKind::RepositoryMissing => DestinationError::RepositoryMissing(detail),
        ResticFailureKind::BackendAuthFailed => DestinationError::AuthFailed(detail),
        ResticFailureKind::DestinationUnreachable | ResticFailureKind::DnsResolutionFailed => {
            DestinationError::Unreachable(detail)
        }
        _ => DestinationError::CommandFailed {
            exit_code: res.exit_code,
            message: detail,
        },
    }
}

struct ProbeRuntime<'a> {
    resolved: &'a ResolvedRestic,
    kestic_binary: &'a Path,
    clock: &'a dyn Clock,
    cache_dir: Option<&'a Path>,
}

fn run_with_temp_repo_password_file(
    dest: &Destination,
    subcommand_args: Vec<String>,
    repo_password: &SecretValue,
    rest_password: Option<&SecretValue>,
    probe: ProbeRuntime<'_>,
) -> Result<ResticExecutionResult, DestinationError> {
    let tmp_dir = create_private_temp_dir()?;
    let res = (|| -> Result<ResticExecutionResult, DestinationError> {
        let pw_file = write_private_password_file(&tmp_dir, repo_password)?;

        let mut args = subcommand_args;
        args.push("--password-file".to_string());
        args.push(pw_file.to_string_lossy().into_owned());

        let mut spec = ResticCommandSpec::for_destination(
            &probe.resolved.binary,
            probe.kestic_binary,
            dest,
            args,
        )
        .with_cache_dir(probe.cache_dir);
        spec.low_priority = false;
        // Override password command to empty/unused since `--password-file` is passed for pre-save probe.
        spec.password_command_override = Some(String::new());

        // The probe runs restic directly with `--password-file`: restic rejects an empty
        // `RESTIC_PASSWORD_COMMAND` next to a password file, so the runner is bypassed.

        execute_probe_command(&spec, repo_password, rest_password, probe.clock)
    })();

    let _ = fs::remove_dir_all(&tmp_dir);
    res
}

fn execute_probe_command(
    spec: &ResticCommandSpec,
    repo_password: &SecretValue,
    rest_password: Option<&SecretValue>,
    clock: &dyn Clock,
) -> Result<ResticExecutionResult, DestinationError> {
    let started_at = clock.now_utc();
    let mut cmd = Command::new(&spec.restic_binary);
    cmd.args(spec.restic_argv())
        .env_remove("RESTIC_PASSWORD")
        .env_remove("RESTIC_PASSWORD_COMMAND")
        .env_remove("RESTIC_PASSWORD_FILE");

    let env_keys = crate::restic::apply_backend_env(&mut cmd, spec, rest_password)?;

    let out = cmd.output()?;
    let finished_at = clock.now_utc();
    let duration_ms = (finished_at - started_at).whole_milliseconds().max(0) as u64;
    let exit_code = out.status.code().unwrap_or(1);

    let mut secrets_to_redact = vec![repo_password.as_str()?];
    if let Some(rest_pw) = rest_password {
        secrets_to_redact.push(rest_pw.as_str()?);
    }

    let stdout_raw =
        crate::restic::redact_secrets(&String::from_utf8_lossy(&out.stdout), &secrets_to_redact);
    let stderr_raw =
        crate::restic::redact_secrets(&String::from_utf8_lossy(&out.stderr), &secrets_to_redact);

    let mut exit_error_msg: Option<String> = None;
    for line in stderr_raw.lines().chain(stdout_raw.lines()) {
        if let Ok(crate::restic::ResticStreamEvent::ExitError { message, .. }) =
            crate::restic::parse_json_line(line)
        {
            exit_error_msg = Some(message);
        }
    }

    let presentation = crate::restic::classify_exit(
        exit_code,
        exit_error_msg.as_deref(),
        Vec::new(),
        &stderr_raw,
    );

    let mut log_lines: Vec<String> = stdout_raw.lines().map(ToOwned::to_owned).collect();
    log_lines.extend(stderr_raw.lines().map(ToOwned::to_owned));

    Ok(ResticExecutionResult {
        exit_code,
        cancelled: false,
        presentation,
        stats: None,
        command_record: crate::model::CommandRecord {
            executable: spec.restic_binary.to_string_lossy().into_owned(),
            args: spec.restic_argv(),
            env_keys,
            started_at,
            duration_ms,
            exit_code,
        },
        log_lines,
        errors: Vec::new(),
    })
}

/// A new directory only the owner can enter (`0700` from the moment it exists).
fn create_private_temp_dir() -> Result<std::path::PathBuf, DestinationError> {
    let nonce =
        crate::model::RunId::generate().map_err(|e| std::io::Error::other(e.to_string()))?;
    let dir = std::env::temp_dir().join(format!("kestic-probe-{}-{nonce}", std::process::id()));
    fs::DirBuilder::new().mode(0o700).create(&dir)?;
    Ok(dir)
}

/// Writes `password` to a new `repo.pw` in `dir`, readable only by the owner (`0600` from the
/// moment it exists).
fn write_private_password_file(
    dir: &Path,
    password: &SecretValue,
) -> Result<std::path::PathBuf, DestinationError> {
    let path = dir.join("repo.pw");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    file.write_all(password.as_str()?.as_bytes())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kestic_test_support::password::strong_password;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn probe_password_file_is_private() {
        let dir = create_private_temp_dir().expect("create probe dir");
        let password = strong_password();
        let file = write_private_password_file(&dir, &SecretValue::from_str_secret(&password))
            .expect("write password file");

        let dir_mode = fs::metadata(&dir)
            .expect("dir metadata")
            .permissions()
            .mode();
        let file_mode = fs::metadata(&file)
            .expect("file metadata")
            .permissions()
            .mode();
        let content = fs::read(&file).expect("read password file");
        fs::remove_dir_all(&dir).expect("remove probe dir");

        // (a) the directory is owner-only
        assert_eq!(dir_mode & 0o777, 0o700);
        // (b) the password file is owner read/write only
        assert_eq!(file_mode & 0o777, 0o600);
        // (c) the file holds exactly the password
        assert!(
            content == password.as_bytes(),
            "password file content differs"
        );
    }
}
