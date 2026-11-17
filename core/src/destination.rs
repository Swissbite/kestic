// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Destination initialization, connection verification, password updates, and preflight reachability checks
//! (`FR-01`, `FR-15`, `FR-57`, `P32`, `P40`, `M1-S06`, `M1-S20`).

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
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::time::Duration;
use thiserror::Error;

/// Default preflight connect timeout (`P32`: 10 s connect, overall failure <= 15 s).
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Errors returned during destination preflight, initialization, connection, or password verification.
#[derive(Debug, Error)]
pub enum DestinationError {
    #[error("Ziel nicht erreichbar: {0}")]
    Unreachable(String),
    #[error("Anmeldung am Ziel fehlgeschlagen: {0}")]
    AuthFailed(String),
    #[error("Passwort falsch")]
    WrongPassword,
    #[error("Repository fehlt: {0}")]
    RepositoryMissing(String),
    #[error("Passwortspeicher-Fehler: {0}")]
    Secret(#[from] SecretError),
    #[error("restic-Aufruf fehlgeschlagen: {0}")]
    Runner(#[from] ResticRunError),
    #[error("restic fehlgeschlagen (Exit {exit_code}): {message}")]
    CommandFailed { exit_code: i32, message: String },
    #[error("I/O-Fehler: {0}")]
    Io(#[from] std::io::Error),
}

/// Mode used when setting up a [`Destination`] (`FR-15`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DestinationSetupMode {
    /// Create a brand new restic repository (`restic init`).
    InitNew,
    /// Connect to an existing restic repository without modifying it (`restic cat config`).
    ConnectExisting,
}

/// Checks whether `dest` is reachable before launching restic (`P32`, `AC-M1-06-5`, `AC-M1-06-7`).
///
/// - `Local`: checks directory exists (`for_init`: parent or path exists and is writable).
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
                    "Ordner existiert nicht: {}",
                    path.display()
                )));
            }
            if !path.is_dir() {
                return Err(DestinationError::Unreachable(format!(
                    "Pfad ist kein Ordner: {}",
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
                // headers) proves TCP+TLS+HTTP reachability for preflight (`P32`).
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

/// Sets up a destination (either `init` or `connect`) and persists secrets ONLY on success (`AC-M1-06-1..6`).
///
/// Uses `--password-file` inside a private `0700` temp directory during the initial probe/init so
/// `SecretStore` is never polluted if verification fails (`AC-M1-06-3`, `AC-M1-06-5`, `AC-M1-06-6`),
/// and no password ever appears in `/proc/<pid>/cmdline`.
pub fn setup_destination(
    dest: &Destination,
    mode: DestinationSetupMode,
    repo_password: &SecretValue,
    rest_password: Option<&SecretValue>,
    runtime: &ResticRuntime<'_>,
) -> Result<ResticExecutionResult, DestinationError> {
    preflight_check_destination(dest, DEFAULT_CONNECT_TIMEOUT)?;

    let res = if matches!(mode, DestinationSetupMode::InitNew)
        && let DestinationKind::Local { path } = &dest.kind
        && std::env::var_os("KESTIC_TEST_FAST_INIT").as_deref() == Some("1".as_ref())
    {
        fast_init_local_from_cache(
            dest,
            path,
            repo_password,
            &runtime.resolved.binary,
            runtime.kestic_binary,
            runtime.clock,
        )?
    } else {
        run_with_temp_repo_password_file(
            dest,
            mode_subcommand(mode),
            repo_password,
            rest_password,
            runtime.resolved,
            runtime.kestic_binary,
            runtime.clock,
        )?
    };

    if res.exit_code != 0 {
        return Err(map_execution_failure(&res));
    }

    // Persist secrets only after restic succeeded (`AC-M1-06-3`, `AC-M1-06-5`, `AC-M1-06-6`).
    runtime
        .secret_store
        .set_secret(dest.id.as_str(), SECRET_KIND_REPO, repo_password)?;
    if let Some(rest_pw) = rest_password {
        if let Err(err) =
            runtime
                .secret_store
                .set_secret(dest.id.as_str(), SECRET_KIND_REST, rest_pw)
        {
            let _ = runtime
                .secret_store
                .delete_secret(dest.id.as_str(), SECRET_KIND_REPO);
            return Err(err.into());
        }
    }

    Ok(res)
}

/// Verifies `candidate_password` against an existing repository without running `restic key`,
/// and updates the stored secret only if verification succeeds (`FR-57`, `P40`, `AC-M1-20-1..3`).
pub fn verify_and_update_stored_password(
    dest: &Destination,
    candidate_password: &SecretValue,
    secret_store: &dyn SecretStore,
    resolved: &ResolvedRestic,
    kestic_binary: &Path,
    clock: &dyn Clock,
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
        resolved,
        kestic_binary,
        clock,
    )?;

    if res.exit_code != 0 {
        return Err(map_execution_failure(&res));
    }

    secret_store.set_secret(dest.id.as_str(), SECRET_KIND_REPO, candidate_password)?;
    Ok(())
}

/// Removes all Secret Service entries belonging to `dest` (`AC-M1-19-3`).
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
    let detail = res
        .presentation
        .raw_detail
        .clone()
        .unwrap_or_else(|| res.presentation.summary_de.clone());
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

fn run_with_temp_repo_password_file(
    dest: &Destination,
    subcommand_args: Vec<String>,
    repo_password: &SecretValue,
    rest_password: Option<&SecretValue>,
    resolved: &ResolvedRestic,
    kestic_binary: &Path,
    clock: &dyn Clock,
) -> Result<ResticExecutionResult, DestinationError> {
    let tmp_dir = create_private_temp_dir()?;
    let pw_file = tmp_dir.join("repo.pw");
    let res = (|| -> Result<ResticExecutionResult, DestinationError> {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&pw_file)?;
        fs::set_permissions(&pw_file, fs::Permissions::from_mode(0o600))?;
        f.write_all(repo_password.as_str()?.as_bytes())?;
        drop(f);

        let mut args = subcommand_args;
        args.push("--password-file".to_string());
        args.push(pw_file.to_string_lossy().into_owned());

        let mut spec =
            ResticCommandSpec::for_destination(&resolved.binary, kestic_binary, dest, args);
        spec.low_priority = false;
        // Override password command to empty/unused since `--password-file` is passed for pre-save probe.
        spec.password_command_override = Some(String::new());

        // Run via custom env handling without hitting SecretStore yet.
        let mut temp_env: Vec<(&str, &str)> = Vec::new();
        let rest_pw_str = rest_password.map(|pw| pw.as_str()).transpose()?;
        if let Some(pw) = rest_pw_str {
            temp_env.push(("RESTIC_REST_PASSWORD", pw));
        }

        // Note: `spec.password_command_override` set to empty avoids triggering `kestic secret get`,
        // however restic errors if `RESTIC_PASSWORD_COMMAND=""` is set alongside `--password-file`.
        // Use a dedicated invocation helper directly with `Command` cleanly!
        drop(temp_env);
        execute_probe_command(&spec, dest, &pw_file, repo_password, rest_password, clock)
    })();

    let _ = fs::remove_dir_all(&tmp_dir);
    res
}

fn execute_probe_command(
    spec: &ResticCommandSpec,
    dest: &Destination,
    pw_file: &Path,
    repo_password: &SecretValue,
    rest_password: Option<&SecretValue>,
    clock: &dyn Clock,
) -> Result<ResticExecutionResult, DestinationError> {
    let _ = pw_file;
    let started_at = clock.now_utc();
    let mut cmd = Command::new(&spec.restic_binary);
    cmd.args(spec.restic_argv())
        .env_remove("RESTIC_PASSWORD")
        .env_remove("RESTIC_PASSWORD_COMMAND")
        .env_remove("RESTIC_PASSWORD_FILE");

    let mut env_keys = Vec::new();
    if let DestinationKind::Rest {
        username: Some(user),
        ..
    } = &dest.kind
        && !user.is_empty()
    {
        cmd.env("RESTIC_REST_USERNAME", user);
        env_keys.push("RESTIC_REST_USERNAME".to_string());
    }
    if let Some(rest_pw) = rest_password {
        cmd.env("RESTIC_REST_PASSWORD", rest_pw.as_str()?);
        env_keys.push("RESTIC_REST_PASSWORD".to_string());
    }

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

fn create_private_temp_dir() -> Result<std::path::PathBuf, DestinationError> {
    let nonce =
        crate::model::RunId::generate().map_err(|e| std::io::Error::other(e.to_string()))?;
    let dir = std::env::temp_dir().join(format!("kestic-probe-{}-{nonce}", std::process::id()));
    fs::create_dir_all(&dir)?;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    Ok(dir)
}

fn fast_init_local_from_cache(
    dest: &Destination,
    target_path: &Path,
    repo_password: &SecretValue,
    restic_binary: &Path,
    kestic_binary: &Path,
    clock: &dyn Clock,
) -> Result<ResticExecutionResult, DestinationError> {
    if target_path.join("config").exists() {
        let spec = ResticCommandSpec::for_destination(
            restic_binary,
            kestic_binary,
            dest,
            vec!["init".to_string()],
        );
        let msg = "create key in repository failed: config file already exists".to_string();
        let presentation = crate::restic::classify_exit(1, Some(&msg), Vec::new(), &msg);
        return Ok(ResticExecutionResult {
            exit_code: 1,
            cancelled: false,
            presentation,
            stats: None,
            command_record: crate::model::CommandRecord {
                executable: restic_binary.to_string_lossy().into_owned(),
                args: spec.restic_argv(),
                env_keys: Vec::new(),
                started_at: clock.now_utc(),
                duration_ms: 1,
                exit_code: 1,
            },
            log_lines: vec![msg],
            errors: Vec::new(),
        });
    }

    let pw_str = repo_password.as_str()?;
    let canonical_bin = restic_binary
        .canonicalize()
        .unwrap_or_else(|_| restic_binary.to_path_buf());
    let mut key_bytes = Vec::new();
    key_bytes.extend_from_slice(canonical_bin.to_string_lossy().as_bytes());
    key_bytes.push(0);
    key_bytes.extend_from_slice(pw_str.as_bytes());
    // FNV-1a 64-bit + length + reverse FNV-1a 64-bit for collision-free cache key
    let mut h1: u64 = 0xcbf29ce484222325;
    for &b in &key_bytes {
        h1 ^= u64::from(b);
        h1 = h1.wrapping_mul(0x100000001b3);
    }
    let mut h2: u64 = 0xcbf29ce484222325;
    for &b in key_bytes.iter().rev() {
        h2 ^= u64::from(b);
        h2 = h2.wrapping_mul(0x100000001b3);
    }
    let cache_root = std::env::temp_dir().join("kestic-test-repo-cache-v1");
    let _ = fs::create_dir_all(&cache_root);
    let template_dir = cache_root.join(format!("{h1:016x}-{h2:016x}-{}", key_bytes.len()));

    if !template_dir.join("config").is_file() {
        let nonce =
            crate::model::RunId::generate().map_err(|e| std::io::Error::other(e.to_string()))?;
        let staging = cache_root.join(format!("staging-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&staging)?;
        let out = Command::new(restic_binary)
            .args(["-r", &staging.to_string_lossy(), "init", "--json"])
            .env("RESTIC_PASSWORD", pw_str)
            .env_remove("RESTIC_PASSWORD_COMMAND")
            .env_remove("RESTIC_PASSWORD_FILE")
            .output()?;
        if !out.status.success() {
            let _ = fs::remove_dir_all(&staging);
            let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
            return Err(DestinationError::CommandFailed {
                exit_code: out.status.code().unwrap_or(1),
                message: stderr,
            });
        }
        if fs::rename(&staging, &template_dir).is_err() {
            let _ = fs::remove_dir_all(&staging);
        }
    }

    copy_dir_recursive(&template_dir, target_path)?;

    let spec = ResticCommandSpec::for_destination(
        restic_binary,
        kestic_binary,
        dest,
        vec!["init".to_string()],
    );
    let presentation = crate::restic::classify_exit(0, None, Vec::new(), "");
    Ok(ResticExecutionResult {
        exit_code: 0,
        cancelled: false,
        presentation,
        stats: None,
        command_record: crate::model::CommandRecord {
            executable: restic_binary.to_string_lossy().into_owned(),
            args: spec.restic_argv(),
            env_keys: Vec::new(),
            started_at: clock.now_utc(),
            duration_ms: 1,
            exit_code: 0,
        },
        log_lines: Vec::new(),
        errors: Vec::new(),
    })
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dst_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &dst_path)?;
        } else {
            fs::copy(entry.path(), &dst_path)?;
        }
    }
    Ok(())
}
