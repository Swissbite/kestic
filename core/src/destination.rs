// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Destination initialization, connection verification, password updates, and preflight reachability checks.

use crate::backend_credentials::BackendCredentials;
use crate::clock::Clock;
use crate::model::{Destination, DestinationKind, normalize_rest_url_to_ascii};
use crate::provider::ResolvedRestic;
use crate::restic::{
    ResticCommandSpec, ResticExecutionResult, ResticFailureKind, ResticRunError, ResticRuntime,
    SECRET_KIND_REPO,
};
use crate::secrets::{SecretError, SecretStore, SecretValue};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
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
/// - `Rest`, `S3`: performs TCP connect + HTTP request within `connect_timeout` (default 10 s).
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
            http_reachable(&ascii_url, connect_timeout)
        }
        DestinationKind::S3 { endpoint, .. } => {
            let ascii_url = crate::s3::normalized_endpoint(endpoint)
                .map_err(|err| DestinationError::Unreachable(err.to_string()))?;
            http_reachable(&ascii_url, connect_timeout)
        }
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
        ProbeRuntime::connection_test(runtime),
    )?;
    match res.presentation.kind {
        ResticFailureKind::None | ResticFailureKind::WrongPassword => Ok(RepositoryProbe::Exists),
        ResticFailureKind::RepositoryMissing
            if is_missing_bucket(res.presentation.raw_detail.as_deref().unwrap_or_default()) =>
        {
            Ok(RepositoryProbe::BucketMissing)
        }
        ResticFailureKind::RepositoryMissing => Ok(RepositoryProbe::Missing),
        _ => Err(map_execution_failure(&res)),
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
    backend: &BackendCredentials,
    runtime: &ResticRuntime<'_>,
) -> Result<ResticExecutionResult, DestinationError> {
    preflight_check_destination(dest, DEFAULT_CONNECT_TIMEOUT)?;

    let res = run_with_temp_repo_password_file(
        dest,
        mode_subcommand(mode),
        repo_password,
        backend,
        ProbeRuntime::without_timeout(runtime),
    )?;

    if res.exit_code != 0 {
        return Err(map_execution_failure(&res));
    }

    // Persist secrets only after restic succeeded.
    runtime
        .secret_store
        .set_secret(dest.id.as_str(), SECRET_KIND_REPO, repo_password)?;
    if let Err(err) = backend.persist(dest, runtime.secret_store) {
        let _ = runtime
            .secret_store
            .delete_secret(dest.id.as_str(), SECRET_KIND_REPO);
        return Err(err.into());
    }

    Ok(res)
}

/// Connection test of an existing repository: checks reachability, the backend login and
/// `repo_password` with `restic cat --no-lock config`, within [`CONNECTION_TEST_TIMEOUT`].
/// Saves nothing.
pub fn check_existing_repository(
    dest: &Destination,
    repo_password: &SecretValue,
    backend: &BackendCredentials,
    runtime: &ResticRuntime<'_>,
) -> Result<(), DestinationError> {
    preflight_check_destination(dest, DEFAULT_CONNECT_TIMEOUT)?;
    let res = run_with_temp_repo_password_file(
        dest,
        mode_subcommand(DestinationSetupMode::ConnectExisting),
        repo_password,
        backend,
        ProbeRuntime::connection_test(runtime),
    )?;
    if res.exit_code != 0 {
        return Err(map_execution_failure(&res));
    }
    Ok(())
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
        ProbeRuntime {
            resolved,
            kestic_binary,
            clock,
            cache_dir,
            timeout: None,
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
    /// restic is stopped after this time and the server counts as not responding.
    timeout: Option<Duration>,
}

impl<'a> ProbeRuntime<'a> {
    fn without_timeout(runtime: &ResticRuntime<'a>) -> Self {
        Self {
            resolved: runtime.resolved,
            kestic_binary: runtime.kestic_binary,
            clock: runtime.clock,
            cache_dir: runtime.cache_dir,
            timeout: None,
        }
    }

    fn connection_test(runtime: &ResticRuntime<'a>) -> Self {
        Self {
            timeout: Some(CONNECTION_TEST_TIMEOUT),
            ..Self::without_timeout(runtime)
        }
    }
}

fn run_with_temp_repo_password_file(
    dest: &Destination,
    subcommand_args: Vec<String>,
    repo_password: &SecretValue,
    backend: &BackendCredentials,
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

        execute_probe_command(&spec, repo_password, backend, probe.clock, probe.timeout)
    })();

    let _ = fs::remove_dir_all(&tmp_dir);
    res
}

fn execute_probe_command(
    spec: &ResticCommandSpec,
    repo_password: &SecretValue,
    backend: &BackendCredentials,
    clock: &dyn Clock,
    timeout: Option<Duration>,
) -> Result<ResticExecutionResult, DestinationError> {
    let started_at = clock.now_utc();
    let mut cmd = Command::new(&spec.restic_binary);
    cmd.args(spec.restic_argv())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("RESTIC_PASSWORD")
        .env_remove("RESTIC_PASSWORD_COMMAND")
        .env_remove("RESTIC_PASSWORD_FILE");

    let env_keys = crate::restic::apply_backend_env(&mut cmd, spec, backend)?;

    let mut secrets_to_redact = vec![repo_password.as_str()?];
    secrets_to_redact.extend(backend.redaction_values());

    let mut child = cmd.spawn()?;
    let watchdog = timeout.map(|limit| Watchdog::start(child.id(), limit));
    let stdout_reader = child.stdout.take().map(|mut out| {
        thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = out.read_to_end(&mut buf);
            buf
        })
    });
    let mut stderr_lines = Vec::new();
    let mut abort_line: Option<String> = None;
    if let Some(err) = child.stderr.take() {
        // Lines are read as bytes: one line that is no UTF-8 must not end the reading early.
        for raw in BufReader::new(err).split(b'\n').map_while(Result::ok) {
            let line = String::from_utf8_lossy(&raw);
            let redacted = crate::restic::redact_secrets(&line, &secrets_to_redact);
            // Only a connection test stops restic early; setup and `init` run to their end.
            let conclusive = timeout
                .and_then(|_| conclusive_retry_error(&redacted))
                .map(ToOwned::to_owned);
            stderr_lines.push(redacted);
            if let Some(error) = conclusive {
                // restic would retry this for many minutes; for a test the answer is known.
                crate::restic::send_sigterm_to_tree(child.id());
                abort_line = Some(error);
                break;
            }
        }
    }
    let timed_out = watchdog.is_some_and(Watchdog::stop);
    let status = child.wait()?;
    if timed_out {
        return Err(DestinationError::NotResponding(format!(
            "restic did not finish within {} s",
            timeout.unwrap_or_default().as_secs()
        )));
    }
    let stdout_bytes = stdout_reader
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

    let presentation = classify_probe(exit_code, exit_error_msg.as_deref(), &stderr_raw);

    let mut log_lines: Vec<String> = stdout_raw.lines().map(ToOwned::to_owned).collect();
    log_lines.extend(stderr_lines);

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

/// Whether a restic message says the S3 bucket does not exist; the repository address restic
/// repeats in its messages does not count (a bucket may be called `nosuchbucket`).
fn is_missing_bucket(message: &str) -> bool {
    let text = crate::restic::without_locations(&message.to_lowercase());
    text.contains("bucket does not exist") || text.contains("nosuchbucket")
}

/// How a probe sees restic's result: like a run, except that a missing S3 bucket means «no
/// repository yet» (`init` creates the bucket).
fn classify_probe(
    exit_code: i32,
    exit_error_msg: Option<&str>,
    raw_stderr: &str,
) -> crate::restic::ErrorPresentation {
    let mut presentation =
        crate::restic::classify_exit(exit_code, exit_error_msg, Vec::new(), raw_stderr);
    if exit_code == 1 && is_missing_bucket(&presentation.detail_or_summary()) {
        presentation.kind = ResticFailureKind::RepositoryMissing;
    }
    presentation
}

/// Stops a process tree with `SIGTERM` unless [`Watchdog::stop`] comes first.
struct Watchdog {
    done: mpsc::Sender<()>,
    fired: Arc<AtomicBool>,
    thread: thread::JoinHandle<()>,
}

impl Watchdog {
    fn start(pid: u32, limit: Duration) -> Self {
        let (done, done_rx) = mpsc::channel::<()>();
        let fired = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&fired);
        let thread = thread::spawn(move || {
            if done_rx.recv_timeout(limit) == Err(mpsc::RecvTimeoutError::Timeout) {
                flag.store(true, Ordering::SeqCst);
                crate::restic::send_sigterm_to_tree(pid);
            }
        });
        Self {
            done,
            fired,
            thread,
        }
    }

    /// Stops the watchdog; returns whether it had already stopped the process.
    fn stop(self) -> bool {
        let _ = self.done.send(());
        let _ = self.thread.join();
        self.fired.load(Ordering::SeqCst)
    }
}

/// The error of a restic line that announces a retry of an error that will not go away by
/// waiting: the backend rejected the login, the bucket does not exist or the server cannot be
/// reached.
fn conclusive_retry_error(line: &str) -> Option<&str> {
    // `<op> returned error, retrying after 1.403627s: <error>`: only the error is classified and
    // reported, the random duration could look like an HTTP status.
    let error = line
        .split_once("retrying after ")
        .and_then(|(_, rest)| rest.split_once(": "))
        .map(|(_, error)| error)?;
    matches!(
        classify_probe(1, Some(error), "").kind,
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
mod tests {
    use super::*;
    use kestic_test_support::password::strong_password;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn retry_duration_is_not_read_as_http_status() {
        let missing = "Stat(<config/>) returned error, retrying after 1.403627s: Stat: The specified bucket does not exist";
        let refused = "Stat(<config/>) returned error, retrying after 401.5ms: dial tcp 127.0.0.1:9: connect: connection refused";
        let signature = "Stat(<config/>) returned error, retrying after 1.2s: Stat: The request signature we calculated does not match the signature you provided.";
        assert!(conclusive_retry_error(missing).is_some());
        assert_eq!(
            classify_probe(1, Some("Stat: The specified bucket does not exist"), "").kind,
            ResticFailureKind::RepositoryMissing
        );
        // A bucket called `nosuchbucket` in the repository address is no missing bucket.
        assert_eq!(
            classify_probe(
                10,
                Some("Fatal: repository does not exist: unable to open config file: Stat: The specified key does not exist.\nIs there a repository at the following location?\ns3:http://127.0.0.1:9000/nosuchbucket"),
                ""
            )
            .kind,
            ResticFailureKind::RepositoryMissing
        );
        assert!(!is_missing_bucket(
            "Is there a repository at the following location?\ns3:http://127.0.0.1:9000/nosuchbucket"
        ));
        assert!(is_missing_bucket(
            "Stat: The specified bucket does not exist"
        ));
        // A run keeps its own reading: a missing bucket is no permanent «repository missing».
        let run = crate::restic::classify_exit(
            1,
            Some("Fatal: unable to open config file: Stat: The specified bucket does not exist"),
            Vec::new(),
            "",
        );
        assert_ne!(run.kind, ResticFailureKind::RepositoryMissing);
        assert!(!run.kind.is_permanent());
        assert!(conclusive_retry_error(refused).is_some());
        assert!(conclusive_retry_error(signature).is_some());
        // The probe reports the error without the duration.
        let reported = |line: &str| classify_probe(1, conclusive_retry_error(line), "").kind;
        assert_eq!(reported(refused), ResticFailureKind::DestinationUnreachable);
        assert_eq!(
            reported(
                "Stat(<config/>) returned error, retrying after 403.2ms: Stat: The specified bucket does not exist"
            ),
            ResticFailureKind::RepositoryMissing
        );
        assert_eq!(reported(signature), ResticFailureKind::BackendAuthFailed);
        assert!(
            conclusive_retry_error(
                "Load(<lock/1>) returned error, retrying after 1.4s: unexpected HTTP 500"
            )
            .is_none()
        );
        assert!(conclusive_retry_error("Stat: The specified bucket does not exist").is_none());
        let refused_on_port = "Stat(<config/>) returned error, retrying after 508.8ms: Stat: Get \"http://127.0.0.1:40312/bucket/?location=\": dial tcp 127.0.0.1:40312: connect: connection refused";
        assert_eq!(
            crate::restic::classify_exit(1, Some(refused_on_port), Vec::new(), "").kind,
            ResticFailureKind::DestinationUnreachable
        );
        assert!(
        conclusive_retry_error(
            "Stat(<config/>) returned error, retrying after 1.403s: Stat: Please reduce your request rate."
        )
        .is_none()
    );
    }

    /// A connection test whose restic does not answer ends after the time limit with «the server
    /// does not respond» and stops restic.
    #[test]
    fn connection_test_stops_restic_that_does_not_answer() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let script = dir.path().join("restic");
        let pid_file = dir.path().join("restic.pid");
        kestic_test_support::tools::write_executable(
            &script,
            &format!(
                "#!/bin/sh\necho $$ > '{}'\nexec sleep 60\n",
                pid_file.display()
            ),
        );
        let resolved = ResolvedRestic {
            binary: script,
            version: semver::Version::new(0, 18, 0),
        };
        let dest = Destination::new(
            "Slow",
            DestinationKind::Local {
                path: dir.path().to_path_buf(),
            },
        )
        .expect("destination");
        let clock = crate::clock::FakeClock::new(time::macros::datetime!(2026-12-01 10:00 UTC));
        let helper = dir.path().join("kestic");
        let started = std::time::Instant::now();

        let res = run_with_temp_repo_password_file(
            &dest,
            mode_subcommand(DestinationSetupMode::ConnectExisting),
            &SecretValue::from_str_secret(strong_password()),
            &BackendCredentials::none(),
            ProbeRuntime {
                resolved: &resolved,
                kestic_binary: &helper,
                clock: &clock,
                cache_dir: None,
                timeout: Some(Duration::from_secs(1)),
            },
        );

        assert!(
            matches!(res, Err(DestinationError::NotResponding(_))),
            "{res:?}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "waited {:?}",
            started.elapsed()
        );
        let pid = fs::read_to_string(&pid_file).expect("restic started");
        assert!(
            !Path::new(&format!("/proc/{}", pid.trim())).exists(),
            "restic still runs"
        );
    }

    /// The connection test waits at most 15 s for restic; setup has no limit.
    #[test]
    fn connection_test_limit_is_15_seconds() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let resolved = ResolvedRestic {
            binary: dir.path().join("restic"),
            version: semver::Version::new(0, 18, 0),
        };
        let helper = dir.path().join("kestic");
        let store = crate::secrets::InMemorySecretStore::new();
        let clock = crate::clock::FakeClock::new(time::macros::datetime!(2026-12-01 10:00 UTC));
        let runtime = ResticRuntime {
            resolved: &resolved,
            kestic_binary: &helper,
            secret_store: &store,
            clock: &clock,
            cache_dir: None,
        };
        assert_eq!(
            ProbeRuntime::connection_test(&runtime).timeout,
            Some(Duration::from_secs(15))
        );
        assert_eq!(ProbeRuntime::without_timeout(&runtime).timeout, None);
    }

    /// A line that is no UTF-8 does not end the reading: the time limit still stops restic.
    #[test]
    fn connection_test_reads_on_after_a_line_that_is_no_utf8() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let script = dir.path().join("restic");
        kestic_test_support::tools::write_executable(
            &script,
            "#!/bin/sh\nprintf '\\377\\n' >&2\nexec sleep 60\n",
        );
        let resolved = ResolvedRestic {
            binary: script,
            version: semver::Version::new(0, 18, 0),
        };
        let dest = Destination::new(
            "Slow",
            DestinationKind::Local {
                path: dir.path().to_path_buf(),
            },
        )
        .expect("destination");
        let clock = crate::clock::FakeClock::new(time::macros::datetime!(2026-12-01 10:00 UTC));
        let helper = dir.path().join("kestic");
        let started = std::time::Instant::now();

        let res = run_with_temp_repo_password_file(
            &dest,
            mode_subcommand(DestinationSetupMode::ConnectExisting),
            &SecretValue::from_str_secret(strong_password()),
            &BackendCredentials::none(),
            ProbeRuntime {
                resolved: &resolved,
                kestic_binary: &helper,
                clock: &clock,
                cache_dir: None,
                timeout: Some(Duration::from_secs(1)),
            },
        );

        assert!(
            matches!(res, Err(DestinationError::NotResponding(_))),
            "{res:?}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "waited {:?}",
            started.elapsed()
        );
    }

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
