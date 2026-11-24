// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Destination initialization, connection verification, password updates, and preflight reachability checks.

use crate::backend_credentials::BackendCredentials;
use crate::clock::Clock;
use crate::known_hosts::{HostKeyCheck, HostKeyError, ScannedHostKey, check_host_key};
use crate::model::{Destination, DestinationKind, normalize_rest_url_to_ascii};
use crate::provider::ResolvedRestic;
use crate::repo_identity::RepositoryId;
use crate::restic::{
    ResticCommandSpec, ResticExecutionResult, ResticFailureKind, ResticRunError, ResticRuntime,
    SECRET_KIND_REPO,
};
use crate::secrets::{SecretError, SecretStore, SecretValue};
use crate::sync::LockUnpoisoned;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use thiserror::Error;

/// Default preflight connect timeout (10 s connect, overall failure <= 15 s).
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Longest a connection test waits for restic before it reports that the server does not
/// respond.
pub const CONNECTION_TEST_TIMEOUT: Duration = Duration::from_secs(15);

/// Errors returned during destination preflight, initialization, connection, or password verification.
#[derive(Debug, Error)]
pub enum DestinationError {
    #[error("destination unreachable: {0}")]
    Unreachable(String),
    #[error("authentication at destination failed: {0}")]
    AuthFailed(String),
    #[error("the server does not respond: {0}")]
    NotResponding(String),
    #[error("wrong repository password")]
    WrongPassword,
    #[error("repository missing: {0}")]
    RepositoryMissing(String),
    #[error("password store error: {0}")]
    Secret(#[from] SecretError),
    #[error("restic invocation failed: {0}")]
    Runner(#[from] ResticRunError),
    /// restic ended with an error; `kind` is how its output was classified for the destination.
    #[error("restic failed (exit {exit_code}): {message}")]
    CommandFailed {
        exit_code: i32,
        message: String,
        kind: ResticFailureKind,
    },
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// `restic cat config` succeeded but printed no repository ID; its first output line.
    #[error("restic printed no repository ID: {0}")]
    NoRepositoryId(String),
    /// SFTP: no `known_hosts` file knows the server; the key the server shows.
    #[error("unknown SSH host key of {}", .0.name)]
    HostKeyUnknown(Box<ScannedHostKey>),
    /// SFTP: a `known_hosts` file has another key of the same type for the server: that file.
    #[error("SSH host key changed; the old entry is in {}", .0.display())]
    HostKeyChanged(PathBuf),
    /// SFTP: the server rejected the SSH key (ssh's line).
    #[error("the server rejected the SSH key: {0}")]
    SshKeyRejected(String),
    /// SFTP: the host key could not be checked (jump host, missing tool, unusable paths).
    #[error(transparent)]
    HostKeyCheck(#[from] HostKeyError),
}

impl DestinationError {
    /// Whether an SFTP server answered with a host key that is unknown or changed: it is
    /// reachable, but restic must not connect to it.
    pub fn is_host_key_problem(&self) -> bool {
        matches!(self, Self::HostKeyUnknown(_) | Self::HostKeyChanged(_))
    }
}

/// The result of [`preflight_check_with_restic`] as reachability: a server with an unknown or
/// changed host key is reachable; the run itself stops at that key and reports it.
pub fn reachability(result: Result<(), DestinationError>) -> Result<(), DestinationError> {
    result.or_else(|err| {
        if err.is_host_key_problem() {
            Ok(())
        } else {
            Err(err)
        }
    })
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
/// - `Rest`, `S3`: performs TCP connect + HTTP request within `connect_timeout` (default 10 s).
///   Any HTTP response (including 401/403/404) proves TCP+HTTP reachability.
/// - `Generic`: nothing Kestic can check without restic; passes.
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
            http_reachable(&ascii_url, connect_timeout)
        }
        DestinationKind::S3 { endpoint, .. } => {
            let ascii_url = crate::s3::normalized_endpoint(endpoint)
                .map_err(|err| DestinationError::Unreachable(err.to_string()))?;
            http_reachable(&ascii_url, connect_timeout)
        }
        // Only restic can reach these backends: see [`preflight_check_with_restic`].
        DestinationKind::Sftp { .. } | DestinationKind::Generic { .. } => Ok(()),
    }
}

/// The pre-check before a run: [`preflight_check_destination`], and for a destination that only
/// restic can reach (SFTP, generic form) `restic cat --no-lock config` with the stored
/// credentials; for SFTP the host key check comes first, and both together stop after
/// [`CONNECTION_TEST_TIMEOUT`]. A destination that does not answer or cannot be reached fails,
/// and so does an unknown or changed SFTP host key (restic does not start); every other answer
/// (no repository, wrong password, refused login) is left to the run, which reports it.
pub fn preflight_check_with_restic(
    dest: &Destination,
    runtime: &ResticRuntime<'_>,
) -> Result<(), DestinationError> {
    preflight_check_destination(dest, DEFAULT_CONNECT_TIMEOUT)?;
    if !dest.kind.needs_restic_preflight() {
        return Ok(());
    }
    let started = Instant::now();
    verify_sftp_host_key(dest)?;
    // A locked password store says nothing about the destination; the run reports it.
    let Ok(backend) = BackendCredentials::load_stored(dest, runtime.secret_store) else {
        return Ok(());
    };
    let probe = ProbeRuntime {
        // The time left after the host key check.
        timeout: Some(
            CONNECTION_TEST_TIMEOUT
                .saturating_sub(started.elapsed())
                .max(Duration::from_secs(1)),
        ),
        host_key_checked: true,
        ..ProbeRuntime::preflight(runtime)
    };
    let result = probe_repository_with(dest, &backend, probe).map(|_| ());
    match result {
        Err(
            err @ (DestinationError::Unreachable(_)
            | DestinationError::NotResponding(_)
            | DestinationError::HostKeyUnknown(_)
            | DestinationError::HostKeyChanged(_)),
        ) => Err(err),
        _ => Ok(()),
    }
}

/// For an SFTP destination: compares the key the server shows with the `known_hosts` files
/// before restic connects. A server that shows no key is left to restic, which reports why.
pub fn verify_sftp_host_key(dest: &Destination) -> Result<(), DestinationError> {
    let DestinationKind::Sftp {
        host, port, user, ..
    } = &dest.kind
    else {
        return Ok(());
    };
    let paths = crate::sftp::KnownHostsPaths::from_env().map_err(HostKeyError::from)?;
    match check_host_key(host, *port, user, &paths, crate::known_hosts::SCAN_TIMEOUT)? {
        HostKeyCheck::Known | HostKeyCheck::NoKey(_) => Ok(()),
        HostKeyCheck::Unknown(key) => Err(DestinationError::HostKeyUnknown(Box::new(key))),
        HostKeyCheck::Changed(file) => Err(DestinationError::HostKeyChanged(file)),
    }
}

/// Any HTTP answer from `url` within `connect_timeout` (plus 2 s for the answer).
fn http_reachable(url: &str, connect_timeout: Duration) -> Result<(), DestinationError> {
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
        .get(url)
        .call()
        .map(|_| ())
        .map_err(|err| DestinationError::Unreachable(err.to_string()))
}

/// Whether a repository exists at a destination (see [`probe_repository`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepositoryProbe {
    /// No repository yet: `init` can create one.
    Missing,
    /// The S3 bucket does not exist yet: `init` creates it together with the repository.
    BucketMissing,
    /// A repository is already there.
    Exists,
}

/// Checks with the backend credentials, but without a repository password, whether a
/// repository exists at `dest`: runs `restic cat --no-lock config` with a random password, so
/// a wrong login is found before anything is created. Writes nothing.
pub fn probe_repository(
    dest: &Destination,
    backend: &BackendCredentials,
    runtime: &ResticRuntime<'_>,
) -> Result<RepositoryProbe, DestinationError> {
    probe_repository_with(dest, backend, ProbeRuntime::connection_test(runtime))
}

fn probe_repository_with(
    dest: &Destination,
    backend: &BackendCredentials,
    probe: ProbeRuntime<'_>,
) -> Result<RepositoryProbe, DestinationError> {
    preflight_check_destination(dest, DEFAULT_CONNECT_TIMEOUT)?;
    let throwaway = SecretValue::from_str_secret(
        crate::model::RunId::generate()
            .map_err(|e| std::io::Error::other(e.to_string()))?
            .as_str(),
    );
    let res = run_with_temp_repo_password_file(
        dest,
        mode_subcommand(DestinationSetupMode::ConnectExisting),
        &throwaway,
        backend,
        probe,
    )?
    .result;
    match res.presentation.kind {
        ResticFailureKind::None | ResticFailureKind::WrongPassword => Ok(RepositoryProbe::Exists),
        ResticFailureKind::RepositoryMissing
            if is_missing_bucket(res.presentation.raw_detail.as_deref().unwrap_or_default()) =>
        {
            Ok(RepositoryProbe::BucketMissing)
        }
        ResticFailureKind::RepositoryMissing => Ok(RepositoryProbe::Missing),
        _ => Err(map_execution_failure(dest, &res)),
    }
}

/// A repository that `init` created or `cat config` opened.
#[derive(Debug)]
pub struct PreparedRepository {
    pub execution: ResticExecutionResult,
    /// The repository ID restic printed (`init --json`, `cat config`); `None` when its output
    /// held none.
    pub repository_id: Option<RepositoryId>,
}

/// Runs `init` or `cat config` for `dest` without saving anything.
///
/// Uses `--password-file` inside a private `0700` temp directory, so no password ever appears in
/// `/proc/<pid>/cmdline`.
pub fn prepare_repository(
    dest: &Destination,
    mode: DestinationSetupMode,
    repo_password: &SecretValue,
    backend: &BackendCredentials,
    runtime: &ResticRuntime<'_>,
) -> Result<PreparedRepository, DestinationError> {
    preflight_check_destination(dest, DEFAULT_CONNECT_TIMEOUT)?;

    let out = run_with_temp_repo_password_file(
        dest,
        mode_subcommand(mode),
        repo_password,
        backend,
        ProbeRuntime::setup(mode, runtime),
    )?;

    if out.result.exit_code != 0 {
        return Err(map_execution_failure(dest, &out.result));
    }
    Ok(PreparedRepository {
        execution: out.result,
        repository_id: out.repository_id,
    })
}

/// Sets up a destination (either `init` or `connect`) and persists secrets ONLY on success; when
/// a password store write fails, the entries written before get their previous values back.
pub fn setup_destination(
    dest: &Destination,
    mode: DestinationSetupMode,
    repo_password: &SecretValue,
    backend: &BackendCredentials,
    runtime: &ResticRuntime<'_>,
) -> Result<PreparedRepository, DestinationError> {
    let prepared = prepare_repository(dest, mode, repo_password, backend, runtime)?;
    let mut previous = crate::backend_credentials::PreviousSecrets::new(dest);
    previous.replace(runtime.secret_store, SECRET_KIND_REPO, repo_password)?;
    match backend.persist(dest, runtime.secret_store) {
        Ok(_) => Ok(prepared),
        Err(err) => {
            previous.restore(runtime.secret_store);
            Err(err.into())
        }
    }
}

/// Connection test of an existing repository: checks reachability, the backend login and
/// `repo_password` with `restic cat --no-lock config`, within [`CONNECTION_TEST_TIMEOUT`], and
/// returns the repository ID. Saves nothing.
pub fn check_existing_repository(
    dest: &Destination,
    repo_password: &SecretValue,
    backend: &BackendCredentials,
    runtime: &ResticRuntime<'_>,
) -> Result<RepositoryId, DestinationError> {
    preflight_check_destination(dest, DEFAULT_CONNECT_TIMEOUT)?;
    let out = run_with_temp_repo_password_file(
        dest,
        mode_subcommand(DestinationSetupMode::ConnectExisting),
        repo_password,
        backend,
        ProbeRuntime::connection_test(runtime),
    )?;
    if out.result.exit_code != 0 {
        return Err(map_execution_failure(dest, &out.result));
    }
    out.repository_id.ok_or_else(|| {
        DestinationError::NoRepositoryId(out.result.log_lines.first().cloned().unwrap_or_default())
    })
}

/// Verifies `candidate_password` against an existing repository without running `restic key`,
/// and updates the stored secret only if verification succeeds; returns the repository ID
/// restic printed.
pub fn verify_and_update_stored_password(
    dest: &Destination,
    candidate_password: &SecretValue,
    secret_store: &dyn SecretStore,
    resolved: &ResolvedRestic,
    kestic_binary: &Path,
    clock: &dyn Clock,
) -> Result<Option<RepositoryId>, DestinationError> {
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
) -> Result<Option<RepositoryId>, DestinationError> {
    preflight_check_destination(dest, DEFAULT_CONNECT_TIMEOUT)?;

    let backend = BackendCredentials::load_stored(dest, secret_store)?;

    let res = run_with_temp_repo_password_file(
        dest,
        vec![
            "cat".to_string(),
            "--no-lock".to_string(),
            "config".to_string(),
        ],
        candidate_password,
        &backend,
        // The same read as connecting an existing repository, with the same time limit.
        ProbeRuntime {
            resolved,
            kestic_binary,
            clock,
            cache_dir,
            timeout: Some(CONNECTION_TEST_TIMEOUT),
            stop_on_unreachable: true,
            host_key_checked: false,
        },
    )?;

    if res.result.exit_code != 0 {
        return Err(map_execution_failure(dest, &res.result));
    }

    secret_store.set_secret(dest.id.as_str(), SECRET_KIND_REPO, candidate_password)?;
    Ok(res.repository_id)
}

/// Removes all Secret Service entries belonging to `dest`.
pub fn remove_destination_secrets(
    dest: &Destination,
    secret_store: &dyn SecretStore,
) -> Result<(), SecretError> {
    secret_store.delete_secret(dest.id.as_str(), SECRET_KIND_REPO)?;
    BackendCredentials::delete_all(dest, secret_store)
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

fn map_execution_failure(dest: &Destination, res: &ResticExecutionResult) -> DestinationError {
    let detail = res.presentation.detail_or_summary();
    let sftp = crate::restic::Transport::of(&dest.kind) == crate::restic::Transport::Sftp;
    match res.presentation.kind {
        ResticFailureKind::WrongPassword => DestinationError::WrongPassword,
        ResticFailureKind::RepositoryMissing => DestinationError::RepositoryMissing(detail),
        ResticFailureKind::BackendAuthFailed
            if sftp && crate::sftp::ssh_failure(&detail).is_some_and(|f| f.is_key_rejected()) =>
        {
            DestinationError::SshKeyRejected(detail)
        }
        ResticFailureKind::BackendAuthFailed => DestinationError::AuthFailed(detail),
        ResticFailureKind::DestinationUnreachable | ResticFailureKind::DnsResolutionFailed => {
            DestinationError::Unreachable(detail)
        }
        _ => DestinationError::CommandFailed {
            exit_code: res.exit_code,
            message: detail,
            kind: res.presentation.kind.clone(),
        },
    }
}

struct ProbeRuntime<'a> {
    resolved: &'a ResolvedRestic,
    kestic_binary: &'a Path,
    clock: &'a dyn Clock,
    cache_dir: Option<&'a Path>,
    /// restic is stopped after this time and the server counts as not responding.
    timeout: Option<Duration>,
    /// With a time limit: a connection error restic keeps retrying (refused, reset, DNS) also
    /// stops restic at once. A refused login or a missing repository always does.
    stop_on_unreachable: bool,
    /// The SFTP host key was checked just before; otherwise it is checked before restic starts.
    host_key_checked: bool,
}

impl<'a> ProbeRuntime<'a> {
    fn without_timeout(runtime: &ResticRuntime<'a>) -> Self {
        Self {
            resolved: runtime.resolved,
            kestic_binary: runtime.kestic_binary,
            clock: runtime.clock,
            cache_dir: runtime.cache_dir,
            timeout: None,
            stop_on_unreachable: false,
            host_key_checked: false,
        }
    }

    /// A user waits for the answer: every error restic would keep retrying ends the test.
    fn connection_test(runtime: &ResticRuntime<'a>) -> Self {
        Self {
            timeout: Some(CONNECTION_TEST_TIMEOUT),
            stop_on_unreachable: true,
            ..Self::without_timeout(runtime)
        }
    }

    /// The check before a run: a connection error may pass while restic retries, so only the
    /// time limit decides that the destination cannot be reached.
    fn preflight(runtime: &ResticRuntime<'a>) -> Self {
        Self {
            timeout: Some(CONNECTION_TEST_TIMEOUT),
            ..Self::without_timeout(runtime)
        }
    }

    /// Connecting only reads the repository config, so it gets the connection test's time limit
    /// and early stop: restic would otherwise retry a revoked key or a deleted bucket for
    /// minutes while the caller waits. `init` fails at once and writes, so it runs to its end.
    fn setup(mode: DestinationSetupMode, runtime: &ResticRuntime<'a>) -> Self {
        match mode {
            DestinationSetupMode::InitNew => Self::without_timeout(runtime),
            DestinationSetupMode::ConnectExisting => Self::connection_test(runtime),
        }
    }
}

/// The result of one probe and the repository ID in restic's output, if any.
#[derive(Debug)]
struct ProbeOutput {
    result: ResticExecutionResult,
    repository_id: Option<RepositoryId>,
}

fn run_with_temp_repo_password_file(
    dest: &Destination,
    subcommand_args: Vec<String>,
    repo_password: &SecretValue,
    backend: &BackendCredentials,
    probe: ProbeRuntime<'_>,
) -> Result<ProbeOutput, DestinationError> {
    // Where inherited credentials are removed, restic without Kestic's own would fall back to
    // the user's credential files and send their keys to this endpoint.
    if !dest.kind.backend_env_cleared_prefixes().is_empty()
        && let Some(kind) = backend.first_missing(&dest.kind)
    {
        return Err(DestinationError::Runner(
            ResticRunError::MissingBackendSecret {
                destination: dest.id.clone(),
                kind,
            },
        ));
    }
    // The check before a run checked the host key just before; a connection test or setup checks
    // it here, and again when ssh refused it.
    let check_host_key_here = !probe.host_key_checked;
    if check_host_key_here {
        verify_sftp_host_key(dest)?;
    }
    let tmp_dir = create_private_temp_dir()?;
    let res = (|| -> Result<ProbeOutput, DestinationError> {
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

        execute_probe_command(
            &spec,
            repo_password,
            backend,
            probe.clock,
            ProbeLimits {
                timeout: probe.timeout,
                stop_on_unreachable: probe.stop_on_unreachable,
            },
        )
    })();

    let _ = fs::remove_dir_all(&tmp_dir);
    // ssh refused the host key although the check passed (it changed meanwhile): the check
    // again tells unknown from changed.
    if check_host_key_here
        && let Ok(ProbeOutput { result, .. }) = &res
        && matches!(
            result.presentation.kind,
            ResticFailureKind::HostKeyUnknown | ResticFailureKind::HostKeyChanged
        )
    {
        verify_sftp_host_key(dest)?;
    }
    // A refused key is no unreachable server, also in the check before a run.
    if let Ok(ProbeOutput { result, .. }) = &res
        && let Some(failure) = sftp_session_failure(dest, result)
    {
        match failure {
            crate::sftp::SshFailure::KeyRejected(line) => {
                return Err(DestinationError::SshKeyRejected(line));
            }
            crate::sftp::SshFailure::HostKeyVerificationFailed(_)
            | crate::sftp::SshFailure::HostKeyChanged(_)
                if check_host_key_here =>
            {
                verify_sftp_host_key(dest)?;
            }
            _ => {}
        }
    }
    res
}

/// A run whose SFTP session ended without ssh's reason: a key the server rejects is a refused
/// login (an error that needs the user), not an unreachable server.
pub(crate) fn explain_sftp_failure(
    dest: &Destination,
    mut result: ResticExecutionResult,
) -> ResticExecutionResult {
    if let Some(crate::sftp::SshFailure::KeyRejected(line)) = sftp_session_failure(dest, &result) {
        result.presentation.kind = ResticFailureKind::BackendAuthFailed;
        result.presentation.raw_detail = Some(line);
    }
    result
}

/// When restic reports only that the SFTP session ended, without ssh's reason: the reason ssh
/// gives on its own.
fn sftp_session_failure(
    dest: &Destination,
    result: &ResticExecutionResult,
) -> Option<crate::sftp::SshFailure> {
    let DestinationKind::Sftp {
        host, port, user, ..
    } = &dest.kind
    else {
        return None;
    };
    let session_ended = matches!(
        result.presentation.kind,
        ResticFailureKind::DestinationUnreachable | ResticFailureKind::GenericFailure
    ) && result
        .presentation
        .raw_detail
        .as_deref()
        .is_some_and(|d| d.contains(crate::restic::SFTP_SESSION_NOT_STARTED));
    if !session_ended {
        return None;
    }
    let paths = crate::sftp::KnownHostsPaths::from_env().ok()?;
    crate::known_hosts::login_failure(host, *port, user, &paths)
}

/// When a probe stops restic (see [`ProbeRuntime`]).
#[derive(Clone, Copy)]
struct ProbeLimits {
    timeout: Option<Duration>,
    stop_on_unreachable: bool,
}

fn execute_probe_command(
    spec: &ResticCommandSpec,
    repo_password: &SecretValue,
    backend: &BackendCredentials,
    clock: &dyn Clock,
    limits: ProbeLimits,
) -> Result<ProbeOutput, DestinationError> {
    let timeout = limits.timeout;
    let started_at = clock.now_utc();
    let mut cmd = Command::new(&spec.restic_binary);
    cmd.args(spec.restic_argv())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("RESTIC_PASSWORD")
        .env_remove("RESTIC_PASSWORD_COMMAND")
        .env_remove("RESTIC_PASSWORD_FILE")
        .env_remove("RESTIC_REPOSITORY_FILE");

    let env_keys = crate::restic::apply_backend_env(&mut cmd, spec, backend)?;

    let mut secrets_to_redact = vec![repo_password.as_str()?];
    secrets_to_redact.extend(backend.redaction_values());

    // Its own process group, so stopping it also reaches processes the backend started below
    // restic that could keep its output open.
    cmd.process_group(0);
    // Held until restic has ended, so the app-managed binary is not replaced meanwhile.
    let _lease = crate::restic_gate::ResticBinaryGate::global().read();
    let mut child = cmd.spawn()?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let child: SharedChild = Arc::new(Mutex::new(child));
    let watchdog = timeout.map(|limit| Watchdog::start(Arc::clone(&child), limit));
    let stdout_reader = stdout.map(|mut out| {
        thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = out.read_to_end(&mut buf);
            buf
        })
    });
    let mut stderr_lines = Vec::new();
    let mut abort_line: Option<String> = None;
    let lines = stderr.map(|err| {
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        thread::spawn(move || {
            // Lines are read as bytes: one line that is no UTF-8 must not end the reading early.
            for raw in BufReader::new(err).split(b'\n').map_while(Result::ok) {
                if tx.send(raw).is_err() {
                    break;
                }
            }
        });
        rx
    });
    let mut gave_up_reading = false;
    if let Some(rx) = lines {
        loop {
            let raw = match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(raw) => raw,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // restic was stopped and has ended, but a process it started still holds
                    // its output open: that process cannot be signalled safely any more, and
                    // its output does not matter.
                    if watchdog.as_ref().is_some_and(Watchdog::has_fired)
                        && wait_try(&child).is_some()
                    {
                        gave_up_reading = true;
                        break;
                    }
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            };
            let line = String::from_utf8_lossy(&raw);
            let redacted = crate::restic::redact_secrets(&line, &secrets_to_redact);
            // Only a read with a time limit stops restic early; `init` runs to its end.
            let conclusive = timeout
                .and_then(|_| conclusive_retry_error(&redacted))
                .filter(|error| limits.stop_on_unreachable || !is_connection_error(error))
                .map(ToOwned::to_owned);
            stderr_lines.push(redacted);
            if let Some(error) = conclusive {
                // restic would retry this for many minutes; for a test the answer is known.
                let _ = signal_if_running(&child, crate::restic::send_sigterm_to_group);
                abort_line = Some(error);
                break;
            }
        }
    }
    // An early stop that restic ignores ends with the watchdog's time limit; the conclusive
    // error found before stays the answer.
    let status = wait_shared(&child)?;
    let timed_out = watchdog.is_some_and(Watchdog::stop);
    if timed_out && abort_line.is_none() {
        return Err(DestinationError::NotResponding(format!(
            "restic did not finish within {} s",
            timeout.unwrap_or_default().as_secs()
        )));
    }
    // Output held open by a left-behind process is not waited for.
    let stdout_bytes = stdout_reader
        .filter(|h| !gave_up_reading || h.is_finished())
        .and_then(|h| h.join().ok())
        .unwrap_or_default();
    let finished_at = clock.now_utc();
    let duration_ms = crate::clock::elapsed_ms(started_at, finished_at);
    let exit_code = if abort_line.is_some() {
        1
    } else {
        status.code().unwrap_or(1)
    };

    let stdout_raw =
        crate::restic::redact_secrets(&String::from_utf8_lossy(&stdout_bytes), &secrets_to_redact);
    let stderr_raw = stderr_lines.join("\n");

    let mut exit_error_msg: Option<String> = abort_line;
    if exit_error_msg.is_none() {
        for line in stderr_raw.lines().chain(stdout_raw.lines()) {
            if let Ok(crate::restic::ResticStreamEvent::ExitError { message, .. }) =
                crate::restic::parse_json_line(line)
            {
                exit_error_msg = Some(message);
            }
        }
    }

    let presentation = classify_probe(
        spec.transport,
        exit_code,
        exit_error_msg.as_deref(),
        &stderr_raw,
    );

    // Read before redaction: a short password could be part of the ID.
    let repository_id = (exit_code == 0)
        .then(|| RepositoryId::from_cat_config(&String::from_utf8_lossy(&stdout_bytes)).ok())
        .flatten();
    let mut log_lines: Vec<String> = stdout_raw.lines().map(ToOwned::to_owned).collect();
    log_lines.extend(stderr_lines);

    let result = ResticExecutionResult {
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
    };
    Ok(ProbeOutput {
        result,
        repository_id,
    })
}

/// Whether a restic message says the S3 bucket does not exist; the repository address restic
/// repeats in its messages does not count (a bucket may be called `nosuchbucket`).
fn is_missing_bucket(message: &str) -> bool {
    let text = crate::restic::without_locations(&message.to_lowercase());
    text.contains("bucket does not exist") || text.contains("nosuchbucket")
}

/// How a probe sees restic's result: like a run, except that a missing S3 bucket means «no
/// repository yet» (`init` creates the bucket).
fn classify_probe(
    transport: crate::restic::Transport,
    exit_code: i32,
    exit_error_msg: Option<&str>,
    raw_stderr: &str,
) -> crate::restic::ErrorPresentation {
    let mut presentation = crate::restic::classify_exit_for(
        transport,
        exit_code,
        exit_error_msg,
        Vec::new(),
        raw_stderr,
    );
    if exit_code == 1 && is_missing_bucket(&presentation.detail_or_summary()) {
        presentation.kind = ResticFailureKind::RepositoryMissing;
    }
    if presentation.kind == ResticFailureKind::GenericFailure
        && let Some(rclone) = rclone_failure(exit_error_msg, raw_stderr)
    {
        return rclone;
    }
    presentation
}

/// When restic reports only that rclone ended («error talking HTTP to rclone»), the reason is in
/// rclone's own lines (`rclone: … CRITICAL: …`): their classification, if it names a cause.
fn rclone_failure(
    exit_error_msg: Option<&str>,
    raw_stderr: &str,
) -> Option<crate::restic::ErrorPresentation> {
    let restic_says_rclone_ended = exit_error_msg
        .unwrap_or(raw_stderr)
        .contains("error talking HTTP to rclone");
    if !restic_says_rclone_ended {
        return None;
    }
    raw_stderr
        .lines()
        .filter(|line| line.trim_start().starts_with("rclone:"))
        .map(|line| crate::restic::classify_exit(1, Some(line), Vec::new(), ""))
        .find(|p| p.kind != ResticFailureKind::GenericFailure)
}

/// How long a process stopped with SIGTERM gets to end before SIGKILL.
const KILL_GRACE: Duration = Duration::from_secs(2);

/// A child process that the reading thread and the [`Watchdog`] share: it is reaped and signalled
/// only under the lock, so a signal never reaches a process ID that was reaped and reused.
type SharedChild = Arc<Mutex<Child>>;

/// Sends `signal` to `child` and its children unless it has already ended (and been reaped);
/// returns whether it was sent.
fn signal_if_running(child: &SharedChild, signal: fn(u32)) -> bool {
    let mut guard = child.lock_unpoisoned();
    let running = matches!(guard.try_wait(), Ok(None));
    if running {
        signal(guard.id());
    }
    running
}

/// The exit status of `child` if it has ended (reaping it).
fn wait_try(child: &SharedChild) -> Option<std::process::ExitStatus> {
    child.lock_unpoisoned().try_wait().ok().flatten()
}

/// Waits for `child` to end without holding the lock, so the watchdog can still stop it.
fn wait_shared(child: &SharedChild) -> std::io::Result<std::process::ExitStatus> {
    loop {
        if let Some(status) = child.lock_unpoisoned().try_wait()? {
            return Ok(status);
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// Stops a process tree with `SIGTERM` after a time limit, and with `SIGKILL` when it ignores
/// that, unless [`Watchdog::stop`] comes first.
struct Watchdog {
    done: mpsc::Sender<()>,
    /// The time limit has passed.
    fired: Arc<AtomicBool>,
    /// The process was still running at the time limit and was told to stop.
    stopped: Arc<AtomicBool>,
    thread: thread::JoinHandle<()>,
}

impl Watchdog {
    fn start(child: SharedChild, limit: Duration) -> Self {
        let (done, done_rx) = mpsc::channel::<()>();
        let fired = Arc::new(AtomicBool::new(false));
        let stopped = Arc::new(AtomicBool::new(false));
        let (fired_flag, stopped_flag) = (Arc::clone(&fired), Arc::clone(&stopped));
        let thread = thread::spawn(move || {
            if done_rx.recv_timeout(limit) == Err(mpsc::RecvTimeoutError::Timeout) {
                let signalled = signal_if_running(&child, crate::restic::send_sigterm_to_group);
                stopped_flag.store(signalled, Ordering::SeqCst);
                fired_flag.store(true, Ordering::SeqCst);
                // A backend that ignores SIGTERM must not keep the check waiting.
                if signalled
                    && done_rx.recv_timeout(KILL_GRACE) == Err(mpsc::RecvTimeoutError::Timeout)
                {
                    signal_if_running(&child, crate::restic::send_sigkill_to_group);
                }
            }
        });
        Self {
            done,
            fired,
            stopped,
            thread,
        }
    }

    /// Whether the time limit has passed and the process was told to stop.
    fn has_fired(&self) -> bool {
        self.fired.load(Ordering::SeqCst)
    }

    /// Stops the watchdog; returns whether it had to stop the process at the time limit.
    fn stop(self) -> bool {
        let _ = self.done.send(());
        let _ = self.thread.join();
        self.stopped.load(Ordering::SeqCst)
    }
}

/// The error of a restic line that announces a retry of an error that will not go away by
/// waiting: the backend rejected the login, the bucket does not exist or the server cannot be
/// reached.
/// Whether `error` (from [`conclusive_retry_error`]) says the server could not be reached, as
/// opposed to a refused login or a missing repository.
fn is_connection_error(error: &str) -> bool {
    matches!(
        classify_probe(crate::restic::Transport::Other, 1, Some(error), "").kind,
        ResticFailureKind::DestinationUnreachable | ResticFailureKind::DnsResolutionFailed
    )
}

fn conclusive_retry_error(line: &str) -> Option<&str> {
    // A line of ssh, such as the server's banner, is no retry of restic.
    if line.trim_start().starts_with("subprocess ") {
        return None;
    }
    // `<op> returned error, retrying after 1.403627s: <error>`: only the error is classified and
    // reported, the random duration could look like an HTTP status.
    let error = line
        .split_once("retrying after ")
        .and_then(|(_, rest)| rest.split_once(": "))
        .map(|(_, error)| error)?;
    matches!(
        classify_probe(crate::restic::Transport::Other, 1, Some(error), "").kind,
        ResticFailureKind::BackendAuthFailed
            | ResticFailureKind::RepositoryMissing
            | ResticFailureKind::DestinationUnreachable
            | ResticFailureKind::DnsResolutionFailed
    )
    .then_some(error)
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
#[path = "destination_tests.rs"]
mod tests;
