// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Restic command construction, tolerant `--json` stream parsing, execution, and error mapping.

use crate::backend_credentials::{BackendCredentials, BackendSecretSlot};
use crate::clock::Clock;
use crate::model::{CommandRecord, Destination, DestinationId, RunStats, RunStatus};
use crate::provider::ResolvedRestic;
use crate::secrets::{SecretError, SecretStore, SecretValue};
use crate::sync::LockUnpoisoned;
use kestic_i18n::codes::ErrorKind;
use kestic_i18n::messages::{Message, completed_message, dns_failed_message};
use serde::Deserialize;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use thiserror::Error;

/// Secret kind selector passed to `kestic secret get <dest-id> [kind]`.
pub const SECRET_KIND_REPO: &str = "repo";
pub use crate::backend_credentials::SECRET_KIND_REST;

/// Shared runtime dependencies required to execute `restic` commands cleanly.
#[derive(Clone, Copy)]
pub struct ResticRuntime<'a> {
    pub resolved: &'a ResolvedRestic,
    pub kestic_binary: &'a Path,
    pub secret_store: &'a dyn SecretStore,
    pub clock: &'a dyn Clock,
    pub cache_dir: Option<&'a Path>,
}

impl ResticRuntime<'_> {
    /// Runs `spec` against `destination`, looking secrets up in the runtime's secret store.
    pub(crate) fn run<F>(
        &self,
        spec: &ResticCommandSpec,
        destination: &Destination,
        cancel: Option<&CancelHandle>,
        extra_env: &[(&str, &str)],
        on_event: F,
    ) -> Result<ResticExecutionResult, ResticRunError>
    where
        F: FnMut(&ResticStreamEvent) + Send,
    {
        run_restic(
            spec,
            destination,
            Some(self.secret_store),
            self.clock,
            cancel,
            extra_env,
            on_event,
        )
    }
}

/// Action offered to the user for recoverable errors; a code from
/// `kestic-i18n`, re-exported for the restic error mapping.
pub use kestic_i18n::codes::RecoveryAction;

/// Classified restic outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResticFailureKind {
    None,
    Cancelled,
    IncompleteSnapshot { unreadable_items: Vec<String> },
    DestinationUnreachable,
    DnsResolutionFailed,
    BackendAuthFailed,
    GenericFailure,
    RepositoryMissing,
    RepositoryLocked,
    WrongPassword,
    Unknown { exit_code: i32 },
}

impl ResticFailureKind {
    /// Whether this failure needs the user before any further scheduled run makes sense
    /// (e.g. exit 12 wrong password, exit 10 missing repository, rejected backend credentials). Other
    /// failures (e.g. disk full, unknown errors) are alerted at once but do not
    /// block the next regular slot.
    pub fn is_permanent(&self) -> bool {
        matches!(
            self,
            Self::WrongPassword | Self::RepositoryMissing | Self::BackendAuthFailed
        )
    }

    /// Stable, locale-independent error-kind code used in D-Bus DTOs (`error_kind`).
    ///
    /// DNS failures count as `destination_unreachable`, everything unrecognised as `unknown`;
    /// the raw restic/daemon text is carried separately as detail.
    pub fn error_kind(&self) -> ErrorKind {
        match self {
            Self::WrongPassword => ErrorKind::WrongPassword,
            Self::RepositoryMissing => ErrorKind::RepoMissing,
            Self::RepositoryLocked => ErrorKind::Locked,
            Self::DestinationUnreachable | Self::DnsResolutionFailed => {
                ErrorKind::DestinationUnreachable
            }
            Self::BackendAuthFailed => ErrorKind::AuthFailed,
            Self::IncompleteSnapshot { .. } => ErrorKind::IncompleteSnapshot,
            Self::Cancelled => ErrorKind::Cancelled,
            Self::None | Self::GenericFailure | Self::Unknown { .. } => ErrorKind::Unknown,
        }
    }

    /// One-sentence summary for notifications and the run history; finer than
    /// [`ErrorKind`] (DNS, «Completed», exit code of unknown errors).
    pub fn summary_message(&self) -> Message {
        match self {
            Self::None => completed_message(),
            Self::DnsResolutionFailed => dns_failed_message(),
            Self::Unknown { exit_code } => ErrorKind::Unknown.message(Some(*exit_code), false),
            other => other.error_kind().message(None, false),
        }
    }

    /// Canonical English summary; the translatable form is
    /// [`ResticFailureKind::summary_message`].
    pub fn default_summary(&self) -> String {
        self.summary_message().english()
    }
}

/// User-facing error metadata derived from exit code and stderr/`exit_error`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorPresentation {
    pub status: RunStatus,
    pub kind: ResticFailureKind,
    pub action: Option<RecoveryAction>,
    pub raw_detail: Option<String>,
}

impl ErrorPresentation {
    /// Returns `raw_detail` when present, falling back to [`ResticFailureKind::default_summary`].
    pub fn detail_or_summary(&self) -> String {
        self.raw_detail
            .clone()
            .unwrap_or_else(|| self.kind.default_summary())
    }
}

/// Parsed `message_type = "status"` line from `restic backup --json`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct ResticStatusMessage {
    pub percent_done: f64,
    pub total_files: Option<u64>,
    pub files_done: Option<u64>,
    pub total_bytes: Option<u64>,
    pub bytes_done: Option<u64>,
    pub seconds_elapsed: Option<u64>,
    pub seconds_remaining: Option<u64>,
    pub current_files: Vec<String>,
}

/// Parsed `message_type = "error"` line from `restic --json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct ResticErrorMessage {
    pub message: String,
    pub item: Option<String>,
    pub during: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawSummaryMessage {
    files_new: u64,
    files_changed: u64,
    files_unmodified: u64,
    dirs_new: u64,
    dirs_changed: u64,
    dirs_unmodified: u64,
    data_added: u64,
    total_files_processed: u64,
    total_bytes_processed: u64,
    snapshot_id: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawExitErrorMessage {
    code: Option<i32>,
    message: String,
}

/// Parsed `message_type = "verbose_status"` line from `restic backup -vv --json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct ResticVerboseStatusMessage {
    pub action: String,
    pub item: String,
    pub duration: Option<serde_json::Value>,
    pub data_size: Option<u64>,
    pub metadata_size: Option<u64>,
    pub total_files: Option<u64>,
}

/// Tolerant event representation for NDJSON lines emitted by `restic --json`.
#[derive(Debug, Clone, PartialEq)]
pub enum ResticStreamEvent {
    Status(ResticStatusMessage),
    VerboseStatus(ResticVerboseStatusMessage),
    Summary(RunStats),
    Error(ResticErrorMessage),
    ExitError { code: Option<i32>, message: String },
    Unknown { message_type: Option<String> },
}

/// Parses one JSON line emitted by `restic --json`, ignoring unknown fields/types.
pub fn parse_json_line(line: &str) -> Result<ResticStreamEvent, serde_json::Error> {
    let value: serde_json::Value = serde_json::from_str(line.trim())?;
    let msg_type = value
        .get("message_type")
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned);

    match msg_type.as_deref() {
        Some("status") => {
            let status: ResticStatusMessage = serde_json::from_value(value)?;
            Ok(ResticStreamEvent::Status(status))
        }
        Some("verbose_status") => {
            let vstatus: ResticVerboseStatusMessage = serde_json::from_value(value)?;
            Ok(ResticStreamEvent::VerboseStatus(vstatus))
        }
        Some("summary") => {
            let raw: RawSummaryMessage = serde_json::from_value(value)?;
            Ok(ResticStreamEvent::Summary(RunStats {
                files_new: raw.files_new,
                files_changed: raw.files_changed,
                files_unmodified: raw.files_unmodified,
                dirs_new: raw.dirs_new,
                dirs_changed: raw.dirs_changed,
                dirs_unmodified: raw.dirs_unmodified,
                data_added: raw.data_added,
                total_files_processed: raw.total_files_processed,
                total_bytes_processed: raw.total_bytes_processed,
                snapshot_id: raw.snapshot_id,
            }))
        }
        Some("error") => {
            let err: ResticErrorMessage = serde_json::from_value(value)?;
            Ok(ResticStreamEvent::Error(err))
        }
        Some("exit_error") => {
            let err: RawExitErrorMessage = serde_json::from_value(value)?;
            Ok(ResticStreamEvent::ExitError {
                code: err.code,
                message: err.message,
            })
        }
        _ => Ok(ResticStreamEvent::Unknown {
            message_type: msg_type,
        }),
    }
}

/// Maps restic exit code and error output into [`ErrorPresentation`].
pub fn classify_exit(
    exit_code: i32,
    exit_error_msg: Option<&str>,
    unreadable_items: Vec<String>,
    raw_stderr: &str,
) -> ErrorPresentation {
    let raw_detail = exit_error_msg
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            let trimmed = raw_stderr.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        });

    let detail_lower = raw_detail.as_deref().unwrap_or_default().to_lowercase();

    match exit_code {
        0 => ErrorPresentation {
            status: RunStatus::Ok,
            kind: ResticFailureKind::None,
            action: None,
            raw_detail,
        },
        3 => ErrorPresentation {
            status: RunStatus::Warning,
            kind: ResticFailureKind::IncompleteSnapshot { unreadable_items },
            action: Some(RecoveryAction::AddToExcludes),
            raw_detail,
        },
        10 => ErrorPresentation {
            status: RunStatus::Error,
            kind: ResticFailureKind::RepositoryMissing,
            action: None,
            raw_detail,
        },
        11 => ErrorPresentation {
            status: RunStatus::Error,
            kind: ResticFailureKind::RepositoryLocked,
            action: Some(RecoveryAction::UnlockRepository),
            raw_detail,
        },
        12 => ErrorPresentation {
            status: RunStatus::Error,
            kind: ResticFailureKind::WrongPassword,
            action: Some(RecoveryAction::UpdateStoredPassword),
            raw_detail,
        },
        1 => {
            if detail_lower.contains("no such host")
                || detail_lower.contains("temporary failure in name resolution")
                || detail_lower.contains("name or service not known")
                || detail_lower.contains("lookup ")
            {
                ErrorPresentation {
                    status: RunStatus::Error,
                    kind: ResticFailureKind::DnsResolutionFailed,
                    action: None,
                    raw_detail,
                }
            } else if is_backend_auth_failure(&detail_lower) {
                ErrorPresentation {
                    status: RunStatus::Error,
                    kind: ResticFailureKind::BackendAuthFailed,
                    action: None,
                    raw_detail,
                }
            } else if is_missing_bucket(&detail_lower) {
                ErrorPresentation {
                    status: RunStatus::Error,
                    kind: ResticFailureKind::RepositoryMissing,
                    action: None,
                    raw_detail,
                }
            } else if detail_lower.contains("connection refused")
                || detail_lower.contains("connection reset")
                || detail_lower.contains("connection closed")
                || detail_lower.contains("broken pipe")
                || detail_lower.contains("unexpected eof")
                || detail_lower.contains("use of closed network connection")
                || detail_lower.contains("network is unreachable")
                || detail_lower.contains("no route to host")
                || detail_lower.contains("i/o timeout")
                || detail_lower.contains("timed out")
                || detail_lower.contains("handshake timeout")
                || detail_lower.contains("context deadline exceeded")
            {
                ErrorPresentation {
                    status: RunStatus::Error,
                    kind: ResticFailureKind::DestinationUnreachable,
                    action: None,
                    raw_detail,
                }
            } else {
                ErrorPresentation {
                    status: RunStatus::Error,
                    kind: ResticFailureKind::GenericFailure,
                    action: None,
                    raw_detail,
                }
            }
        }
        other => ErrorPresentation {
            status: RunStatus::Error,
            kind: ResticFailureKind::Unknown { exit_code: other },
            action: None,
            raw_detail,
        },
    }
}

/// Whether the lowercase restic message says the backend rejected the login: HTTP 401/403 or
/// the S3 errors for a wrong access key or secret key.
fn is_backend_auth_failure(detail_lower: &str) -> bool {
    [
        "401",
        "403",
        "unauthorized",
        "forbidden",
        "authentication",
        "access denied",
        "signature we calculated does not match",
        "signaturedoesnotmatch",
        "access key id you provided does not exist",
        "invalidaccesskeyid",
    ]
    .iter()
    .any(|needle| detail_lower.contains(needle))
}

/// Whether the lowercase restic message says the S3 bucket does not exist.
fn is_missing_bucket(detail_lower: &str) -> bool {
    detail_lower.contains("bucket does not exist") || detail_lower.contains("nosuchbucket")
}

/// Replaces any known secret values in `text` with `***REDACTED***`.
pub fn redact_secrets(text: &str, secrets: &[&str]) -> String {
    let mut out = text.to_string();
    for secret in secrets {
        if !secret.is_empty() {
            out = out.replace(secret, "***REDACTED***");
        }
    }
    out
}

/// Errors encountered before or while spawning a restic child process.
#[derive(Debug, Error)]
pub enum ResticRunError {
    #[error("failed to read repository secret: {0}")]
    Secret(#[from] SecretError),
    #[error("missing backend credential `{kind}` in secret store for destination `{destination}`")]
    MissingBackendSecret {
        destination: DestinationId,
        kind: &'static str,
    },
    #[error("failed to spawn restic command: {0}")]
    Io(#[from] std::io::Error),
}

/// Specification for running a restic command safely via `RESTIC_PASSWORD_COMMAND`.
#[derive(Debug, Clone)]
pub struct ResticCommandSpec {
    pub restic_binary: PathBuf,
    pub kestic_binary: PathBuf,
    pub destination_id: DestinationId,
    pub repository_uri: String,
    pub subcommand_args: Vec<String>,
    /// Backend variables that are not secret (REST user name, S3 region).
    pub backend_env: Vec<(String, String)>,
    /// Backend credentials restic reads from the environment.
    pub backend_secret_slots: Vec<BackendSecretSlot>,
    pub low_priority: bool,
    pub json_output: bool,
    pub password_command_override: Option<String>,
    pub cache_dir: Option<PathBuf>,
}

impl ResticCommandSpec {
    /// Creates a new specification for running restic against `destination`.
    pub fn for_destination(
        restic_binary: impl Into<PathBuf>,
        kestic_binary: impl Into<PathBuf>,
        destination: &Destination,
        subcommand_args: Vec<String>,
    ) -> Self {
        Self {
            restic_binary: restic_binary.into(),
            kestic_binary: kestic_binary.into(),
            destination_id: destination.id.clone(),
            repository_uri: destination.kind.repository_uri(),
            subcommand_args,
            backend_env: destination.kind.backend_plain_env(),
            backend_secret_slots: destination.kind.backend_secret_slots(),
            low_priority: true,
            json_output: true,
            password_command_override: None,
            cache_dir: None,
        }
    }

    /// Sets an explicit `RESTIC_CACHE_DIR` for this command invocation.
    pub fn with_cache_dir(mut self, dir: Option<&Path>) -> Self {
        self.cache_dir = dir.map(Path::to_path_buf);
        self
    }

    /// Creates a specification for running `restic unlock` against `destination`.
    pub fn unlock(
        restic_binary: impl Into<PathBuf>,
        kestic_binary: impl Into<PathBuf>,
        destination: &Destination,
    ) -> Self {
        let mut spec = Self::for_destination(
            restic_binary,
            kestic_binary,
            destination,
            vec!["unlock".to_string()],
        );
        spec.low_priority = false;
        spec
    }

    /// Formats the command invocation for user inspection / clipboard.
    ///
    /// Never includes plaintext secrets.
    pub fn to_transparent_shell_command(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        let pw_cmd = self
            .password_command_override
            .clone()
            .unwrap_or_else(|| format!("kestic secret get {}", self.destination_id));
        parts.push(format!("RESTIC_PASSWORD_COMMAND={}", quote_shell(&pw_cmd)));

        for (name, value) in &self.backend_env {
            parts.push(format!("{name}={}", quote_shell(value)));
        }
        for slot in &self.backend_secret_slots {
            parts.push(format!(
                "{}=\"$(kestic secret get {} {})\"",
                slot.env_name, self.destination_id, slot.kind
            ));
        }

        if self.low_priority {
            parts.push("ionice".to_string());
            parts.push("-c".to_string());
            parts.push("3".to_string());
            parts.push("nice".to_string());
            parts.push("-n".to_string());
            parts.push("10".to_string());
        }

        parts.push(quote_shell(&self.restic_binary.to_string_lossy()));
        for arg in self.restic_argv() {
            parts.push(quote_shell(&arg));
        }
        parts.join(" ")
    }

    /// Returns the argument vector passed directly to `restic`.
    pub fn restic_argv(&self) -> Vec<String> {
        let mut args = vec!["-r".to_string(), self.repository_uri.clone()];
        if self.json_output && !self.subcommand_args.iter().any(|a| a == "--json") {
            args.push("--json".to_string());
        }
        args.extend(self.subcommand_args.iter().cloned());
        args
    }

    fn resolve_invocation(&self) -> (PathBuf, Vec<String>) {
        let restic_args = self.restic_argv();
        if !self.low_priority {
            return (self.restic_binary.clone(), restic_args);
        }

        let has_ionice = which::which("ionice").is_ok();
        let has_nice = which::which("nice").is_ok();

        match (has_ionice, has_nice) {
            (true, true) => {
                let mut args = vec![
                    "-c".to_string(),
                    "3".to_string(),
                    "nice".to_string(),
                    "-n".to_string(),
                    "10".to_string(),
                    self.restic_binary.to_string_lossy().into_owned(),
                ];
                args.extend(restic_args);
                (PathBuf::from("ionice"), args)
            }
            (false, true) => {
                let mut args = vec![
                    "-n".to_string(),
                    "10".to_string(),
                    self.restic_binary.to_string_lossy().into_owned(),
                ];
                args.extend(restic_args);
                (PathBuf::from("nice"), args)
            }
            (true, false) => {
                let mut args = vec![
                    "-c".to_string(),
                    "3".to_string(),
                    self.restic_binary.to_string_lossy().into_owned(),
                ];
                args.extend(restic_args);
                (PathBuf::from("ionice"), args)
            }
            (false, false) => (self.restic_binary.clone(), restic_args),
        }
    }
}

/// A recorded restic invocation split back into its parts: the inverse of the low-priority
/// wrapper `ionice -c 3 [nice -n 10] <restic>` / `nice -n 10 <restic>` that
/// [`ResticCommandSpec`] adds when it runs restic, and of `-r <repo> [--json]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedInvocation {
    pub restic_binary: String,
    pub low_priority: bool,
    pub json_output: bool,
    pub subcommand_args: Vec<String>,
}

/// Splits a recorded `executable` + `args` (history `CommandRecord`) into a [`RecordedInvocation`].
pub fn parse_recorded_invocation(executable: &str, args: &[String]) -> RecordedInvocation {
    let tokens: Vec<&str> = std::iter::once(executable)
        .chain(args.iter().map(String::as_str))
        .collect();
    let is_program = |token: Option<&&str>, name: &str| {
        token.is_some_and(|t| Path::new(t).file_name().is_some_and(|n| n == name))
    };
    let mut i = 0;
    let mut low_priority = false;
    if is_program(tokens.get(i), "ionice")
        && tokens.get(i + 1) == Some(&"-c")
        && tokens.len() > i + 3
    {
        i += 3;
        low_priority = true;
    }
    if is_program(tokens.get(i), "nice") && tokens.get(i + 1) == Some(&"-n") && tokens.len() > i + 3
    {
        i += 3;
        low_priority = true;
    }
    let restic_binary = tokens.get(i).copied().unwrap_or(executable).to_string();
    let mut rest = tokens.get(i + 1..).unwrap_or(&[]);
    if rest.first() == Some(&"-r") && rest.len() >= 2 {
        rest = &rest[2..];
    }
    let json_output = rest.first() == Some(&"--json");
    if json_output {
        rest = &rest[1..];
    }
    RecordedInvocation {
        restic_binary,
        low_priority,
        json_output,
        subcommand_args: rest.iter().map(|s| (*s).to_string()).collect(),
    }
}

fn quote_shell(raw: &str) -> String {
    shlex::try_quote(raw).map_or_else(|_| format!("'{raw}'"), |cow| cow.into_owned())
}

/// Handle allowing another thread to request graceful cancellation (`SIGTERM`).
#[derive(Debug, Clone, Default)]
pub struct CancelHandle {
    cancelled: Arc<AtomicBool>,
    child_pid: Arc<Mutex<Option<u32>>>,
}

impl CancelHandle {
    pub fn new() -> Self {
        Self::default()
    }

    /// Marks the run cancelled and sends `SIGTERM` if the child process is currently running.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        let pid_opt = *self.child_pid.lock_unpoisoned();
        if let Some(pid) = pid_opt {
            // If `ionice`/`nice` wrapped `restic`, try signalling both the process group / child tree
            // as well as the direct PID. First look up direct child `restic` processes if any.
            send_sigterm_to_tree(pid);
        }
    }

    /// Returns `true` if cancellation was requested.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    fn set_pid(&self, pid: Option<u32>) {
        let mut guard = self.child_pid.lock_unpoisoned();
        *guard = pid;
    }
}

/// Sends `SIGTERM` to the children of `pid` (restic behind a wrapper) and to `pid` itself.
pub(crate) fn send_sigterm_to_tree(pid: u32) {
    // `nice`/`ionice` exec into restic directly on Linux (`execve`), so `pid` IS restic,
    // but sending SIGTERM via `pkill -TERM -P <pid>` first handles wrapper shells too.
    let _ = Command::new("pkill")
        .args(["-TERM", "-P", &pid.to_string()])
        .status();
    let _ = Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status();
}

/// Result of executing a restic command via [`run_restic`].
#[derive(Debug, Clone)]
pub struct ResticExecutionResult {
    pub exit_code: i32,
    pub cancelled: bool,
    pub presentation: ErrorPresentation,
    pub stats: Option<RunStats>,
    pub command_record: CommandRecord,
    pub log_lines: Vec<String>,
    pub errors: Vec<ResticErrorMessage>,
}

struct ResolvedRunSecrets {
    backend: BackendCredentials,
    repo_password_for_redaction: Option<SecretValue>,
}

fn resolve_restic_secrets(
    destination: &Destination,
    secret_store: Option<&dyn SecretStore>,
) -> Result<ResolvedRunSecrets, ResticRunError> {
    let mut backend = BackendCredentials::none();
    let mut repo_password_for_redaction: Option<SecretValue> = None;
    if let Some(store) = secret_store {
        if let Some(pw) = store.get_secret(destination.id.as_str(), SECRET_KIND_REPO)? {
            repo_password_for_redaction = Some(pw);
        }
        backend = BackendCredentials::load_stored(destination, store)?;
        if let Some(kind) = backend.first_missing(&destination.kind) {
            return Err(ResticRunError::MissingBackendSecret {
                destination: destination.id.clone(),
                kind,
            });
        }
    }
    Ok(ResolvedRunSecrets {
        backend,
        repo_password_for_redaction,
    })
}

struct PreparedCommand {
    cmd: Command,
    executable: String,
    args: Vec<String>,
    env_keys: Vec<String>,
}

/// Sets the cache directory and the backend variables of `spec` (credentials from `backend`) on
/// `cmd`; returns the names of the variables set so a command record can list them without
/// values.
pub(crate) fn apply_backend_env(
    cmd: &mut Command,
    spec: &ResticCommandSpec,
    backend: &BackendCredentials,
) -> Result<Vec<String>, SecretError> {
    let mut env_keys = Vec::new();
    if let Some(ref cache_dir) = spec.cache_dir {
        cmd.env("RESTIC_CACHE_DIR", cache_dir);
        env_keys.push("RESTIC_CACHE_DIR".to_string());
    }
    for (name, value) in &spec.backend_env {
        cmd.env(name, value);
        env_keys.push(name.clone());
    }
    env_keys.extend(backend.apply_env(cmd, &spec.backend_secret_slots)?);
    Ok(env_keys)
}

fn build_restic_command(
    spec: &ResticCommandSpec,
    backend: &BackendCredentials,
    extra_env: &[(&str, &str)],
) -> Result<PreparedCommand, ResticRunError> {
    let pw_command = spec.password_command_override.clone().unwrap_or_else(|| {
        format!(
            "{} secret get {}",
            quote_shell(&spec.kestic_binary.to_string_lossy()),
            spec.destination_id
        )
    });

    let (executable, args) = spec.resolve_invocation();
    let mut cmd = Command::new(&executable);
    cmd.args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("RESTIC_PASSWORD")
        .env("RESTIC_PASSWORD_COMMAND", &pw_command);

    let mut env_keys = vec!["RESTIC_PASSWORD_COMMAND".to_string()];

    env_keys.extend(apply_backend_env(&mut cmd, spec, backend)?);
    for (k, v) in extra_env {
        cmd.env(k, v);
        env_keys.push((*k).to_string());
    }

    Ok(PreparedCommand {
        cmd,
        executable: executable.to_string_lossy().into_owned(),
        args,
        env_keys,
    })
}

fn handle_stream_line<F>(
    line: &str,
    secrets_to_redact: &[&str],
    is_stdout: bool,
    stats: &mut Option<RunStats>,
    error_messages: &mut Vec<ResticErrorMessage>,
    exit_error_msg: &mut Option<String>,
    on_event: &mut F,
) -> String
where
    F: FnMut(&ResticStreamEvent),
{
    let redacted = redact_secrets(line, secrets_to_redact);
    if let Ok(event) = parse_json_line(&redacted) {
        match &event {
            ResticStreamEvent::Summary(s) if is_stdout => *stats = Some(s.clone()),
            ResticStreamEvent::Error(e) => error_messages.push(e.clone()),
            ResticStreamEvent::ExitError { message, .. } => {
                *exit_error_msg = Some(message.clone());
            }
            ResticStreamEvent::Summary(_)
            | ResticStreamEvent::Status(_)
            | ResticStreamEvent::VerboseStatus(_)
            | ResticStreamEvent::Unknown { .. } => {}
        }
        on_event(&event);
    }
    redacted
}

/// Executes `spec` with environment isolation (`RESTIC_PASSWORD_COMMAND`, `RESTIC_PASSWORD` unset).
///
/// If `secret_store` is provided, looks up the backend credentials of `destination` (REST
/// password, S3 keys) and passes them as environment variables (never as arguments).
pub fn run_restic<F>(
    spec: &ResticCommandSpec,
    destination: &Destination,
    secret_store: Option<&dyn SecretStore>,
    clock: &dyn Clock,
    cancel: Option<&CancelHandle>,
    extra_env: &[(&str, &str)],
    mut on_event: F,
) -> Result<ResticExecutionResult, ResticRunError>
where
    F: FnMut(&ResticStreamEvent) + Send,
{
    let started_at = clock.now_utc();
    let secrets = resolve_restic_secrets(destination, secret_store)?;
    let mut prepared = build_restic_command(spec, &secrets.backend, extra_env)?;

    let mut child: Child = prepared.cmd.spawn()?;
    if let Some(handle) = cancel {
        handle.set_pid(Some(child.id()));
        if handle.is_cancelled() {
            handle.cancel();
        }
    }

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let mut secrets_to_redact: Vec<&str> = Vec::new();
    if let Some(s) = secrets.repo_password_for_redaction.as_ref() {
        secrets_to_redact.push(s.as_str()?);
    }
    secrets_to_redact.extend(secrets.backend.redaction_values());

    let stderr_handle = thread::spawn(move || -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(err_stream) = stderr {
            let reader = BufReader::new(err_stream);
            lines.extend(reader.lines().map_while(Result::ok));
        }
        lines
    });

    let mut log_lines = Vec::new();
    let mut stats: Option<RunStats> = None;
    let mut error_messages: Vec<ResticErrorMessage> = Vec::new();
    let mut exit_error_msg: Option<String> = None;

    if let Some(out_stream) = stdout {
        for line in BufReader::new(out_stream).lines().map_while(Result::ok) {
            let redacted = handle_stream_line(
                &line,
                &secrets_to_redact,
                true,
                &mut stats,
                &mut error_messages,
                &mut exit_error_msg,
                &mut on_event,
            );
            log_lines.push(redacted);
        }
    }

    let raw_stderr_lines = stderr_handle.join().unwrap_or_default();
    let mut redacted_stderr_lines = Vec::with_capacity(raw_stderr_lines.len());
    for line in raw_stderr_lines {
        let redacted = handle_stream_line(
            &line,
            &secrets_to_redact,
            false,
            &mut stats,
            &mut error_messages,
            &mut exit_error_msg,
            &mut on_event,
        );
        redacted_stderr_lines.push(redacted.clone());
        log_lines.push(redacted);
    }

    let status = child.wait()?;
    if let Some(handle) = cancel {
        handle.set_pid(None);
    }

    let finished_at = clock.now_utc();
    let duration_ms = crate::clock::elapsed_ms(started_at, finished_at);
    let exit_code = status.code().unwrap_or(130);
    let cancelled = cancel.is_some_and(CancelHandle::is_cancelled);

    let raw_stderr = redacted_stderr_lines.join("\n");
    let unreadable_items = error_messages
        .iter()
        .filter_map(|e| e.item.clone())
        .collect();
    let mut presentation = classify_exit(
        exit_code,
        exit_error_msg.as_deref(),
        unreadable_items,
        &raw_stderr,
    );
    if cancelled {
        presentation.status = RunStatus::Cancelled;
        presentation.kind = ResticFailureKind::Cancelled;
        presentation.action = None;
    }

    Ok(ResticExecutionResult {
        exit_code,
        cancelled,
        presentation,
        stats,
        command_record: CommandRecord {
            executable: prepared.executable,
            args: prepared.args,
            env_keys: prepared.env_keys,
            started_at,
            duration_ms,
            exit_code,
        },
        log_lines,
        errors: error_messages,
    })
}
