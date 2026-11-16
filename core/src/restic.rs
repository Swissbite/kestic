// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL

//! Restic command construction, tolerant `--json` stream parsing, execution, and error mapping
//! (`NFR-02`, `NFR-10`, `FR-34`, `FR-37`, `FR-42`, `M1-S03`, `M1-S14`, `M1-S15`).

use crate::clock::Clock;
use crate::model::{
    CommandRecord, Destination, DestinationId, DestinationKind, RunStats, RunStatus,
};
use crate::secrets::{SecretError, SecretStore, SecretValue};
use serde::Deserialize;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use thiserror::Error;

/// Secret kind selector passed to `kestic secret get <dest-id> [kind]`.
pub const SECRET_KIND_REPO: &str = "repo";
pub const SECRET_KIND_REST: &str = "rest";

/// Action offered to the user for recoverable errors (`FR-37`, `M1-S15`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryAction {
    UpdateStoredPassword,
    AddToExcludes,
    CheckResticSource,
    UnlockRepository,
}

/// Classified restic outcome (`AC-M1-03-2`, `FR-37`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResticFailureKind {
    None,
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

/// User-facing error metadata derived from exit code and stderr/`exit_error` (`M1-S15`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorPresentation {
    pub status: RunStatus,
    pub kind: ResticFailureKind,
    pub summary_de: String,
    pub summary_en: String,
    pub action: Option<RecoveryAction>,
    pub raw_detail: Option<String>,
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

/// Tolerant event representation for NDJSON lines emitted by `restic --json` (`AC-M1-03-1`).
#[derive(Debug, Clone, PartialEq)]
pub enum ResticStreamEvent {
    Status(ResticStatusMessage),
    Summary(RunStats),
    Error(ResticErrorMessage),
    ExitError { code: Option<i32>, message: String },
    Unknown { message_type: Option<String> },
}

/// Parses one JSON line emitted by `restic --json`, ignoring unknown fields/types (`AC-M1-03-1`).
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

/// Maps restic exit code and error output into [`ErrorPresentation`] (`AC-M1-03-2`, `AC-M1-15-1..3`).
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
            summary_de: "Abgeschlossen".to_string(),
            summary_en: "Completed".to_string(),
            action: None,
            raw_detail,
        },
        3 => ErrorPresentation {
            status: RunStatus::Warning,
            kind: ResticFailureKind::IncompleteSnapshot { unreadable_items },
            summary_de: "Snapshot unvollständig: Einige Dateien konnten nicht gelesen werden"
                .to_string(),
            summary_en: "Snapshot incomplete: some files could not be read".to_string(),
            action: Some(RecoveryAction::AddToExcludes),
            raw_detail,
        },
        10 => ErrorPresentation {
            status: RunStatus::Error,
            kind: ResticFailureKind::RepositoryMissing,
            summary_de: "Repository fehlt".to_string(),
            summary_en: "Repository is missing".to_string(),
            action: None,
            raw_detail,
        },
        11 => ErrorPresentation {
            status: RunStatus::Error,
            kind: ResticFailureKind::RepositoryLocked,
            summary_de: "Repository gesperrt".to_string(),
            summary_en: "Repository is locked".to_string(),
            action: Some(RecoveryAction::UnlockRepository),
            raw_detail,
        },
        12 => ErrorPresentation {
            status: RunStatus::Error,
            kind: ResticFailureKind::WrongPassword,
            summary_de: "Gespeichertes Passwort passt nicht".to_string(),
            summary_en: "Stored password does not match".to_string(),
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
                    summary_de: "Ziel-Adresse konnte nicht aufgelöst werden (DNS)".to_string(),
                    summary_en: "Destination hostname could not be resolved (DNS)".to_string(),
                    action: None,
                    raw_detail,
                }
            } else if detail_lower.contains("401")
                || detail_lower.contains("403")
                || detail_lower.contains("unauthorized")
                || detail_lower.contains("forbidden")
                || detail_lower.contains("authentication")
            {
                ErrorPresentation {
                    status: RunStatus::Error,
                    kind: ResticFailureKind::BackendAuthFailed,
                    summary_de: "Anmeldung am Ziel fehlgeschlagen".to_string(),
                    summary_en: "Authentication at destination failed".to_string(),
                    action: None,
                    raw_detail,
                }
            } else if detail_lower.contains("connection refused")
                || detail_lower.contains("network is unreachable")
                || detail_lower.contains("no route to host")
                || detail_lower.contains("i/o timeout")
                || detail_lower.contains("timed out")
                || detail_lower.contains("does not exist")
            {
                ErrorPresentation {
                    status: RunStatus::Error,
                    kind: ResticFailureKind::DestinationUnreachable,
                    summary_de: "Ziel nicht erreichbar".to_string(),
                    summary_en: "Destination unreachable".to_string(),
                    action: None,
                    raw_detail,
                }
            } else {
                ErrorPresentation {
                    status: RunStatus::Error,
                    kind: ResticFailureKind::GenericFailure,
                    summary_de: "Backup-Befehl fehlgeschlagen".to_string(),
                    summary_en: "Backup command failed".to_string(),
                    action: None,
                    raw_detail,
                }
            }
        }
        other => ErrorPresentation {
            status: RunStatus::Error,
            kind: ResticFailureKind::Unknown { exit_code: other },
            summary_de: format!("Unbekannter Fehler (Exit {other})"),
            summary_en: format!("Unknown error (exit {other})"),
            action: None,
            raw_detail,
        },
    }
}

/// Replaces any known secret values in `text` with `***REDACTED***` (`NFR-02`, `AC-M1-08-2`).
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
    #[error("missing repository password in secret store for destination `{0}`")]
    MissingRepoPassword(DestinationId),
    #[error("missing REST password in secret store for destination `{0}`")]
    MissingRestPassword(DestinationId),
    #[error("failed to spawn restic command: {0}")]
    Io(#[from] std::io::Error),
}

/// Specification for running a restic command safely via `RESTIC_PASSWORD_COMMAND` (`NFR-02`).
#[derive(Debug, Clone)]
pub struct ResticCommandSpec {
    pub restic_binary: PathBuf,
    pub kestic_binary: PathBuf,
    pub destination_id: DestinationId,
    pub repository_uri: String,
    pub subcommand_args: Vec<String>,
    pub rest_username: Option<String>,
    pub low_priority: bool,
    pub json_output: bool,
    pub password_command_override: Option<String>,
}

impl ResticCommandSpec {
    /// Creates a new specification for running restic against `destination`.
    pub fn for_destination(
        restic_binary: impl Into<PathBuf>,
        kestic_binary: impl Into<PathBuf>,
        destination: &Destination,
        subcommand_args: Vec<String>,
    ) -> Self {
        let rest_username = match &destination.kind {
            DestinationKind::Rest { username, .. } => username.clone(),
            DestinationKind::Local { .. } => None,
        };
        Self {
            restic_binary: restic_binary.into(),
            kestic_binary: kestic_binary.into(),
            destination_id: destination.id.clone(),
            repository_uri: destination.kind.repository_uri(),
            subcommand_args,
            rest_username,
            low_priority: true,
            json_output: true,
            password_command_override: None,
        }
    }

    /// Formats the command invocation for user inspection / clipboard (`FR-34`, `AC-M1-14-1`, `AC-M1-14-2`).
    ///
    /// Never includes plaintext secrets.
    pub fn to_transparent_shell_command(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        let pw_cmd = self
            .password_command_override
            .clone()
            .unwrap_or_else(|| format!("kestic secret get {}", self.destination_id));
        parts.push(format!("RESTIC_PASSWORD_COMMAND={}", quote_shell(&pw_cmd)));

        if let Some(user) = self.rest_username.as_deref().filter(|u| !u.is_empty()) {
            parts.push(format!("RESTIC_REST_USERNAME={}", quote_shell(user)));
            parts.push(format!(
                "RESTIC_REST_PASSWORD=\"$(kestic secret get {} {SECRET_KIND_REST})\"",
                self.destination_id
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

fn quote_shell(raw: &str) -> String {
    shlex::try_quote(raw)
        .map(|cow| cow.into_owned())
        .unwrap_or_else(|_| format!("'{raw}'"))
}

/// Handle allowing another thread to request graceful cancellation (`SIGTERM`, `FR-21`).
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
        let pid_opt = *self
            .child_pid
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
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
        let mut guard = self
            .child_pid
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = pid;
    }
}

fn send_sigterm_to_tree(pid: u32) {
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

/// Executes `spec` with environment isolation (`RESTIC_PASSWORD_COMMAND`, `RESTIC_PASSWORD` unset).
///
/// If `secret_store` is provided and `spec.rest_username` is set, looks up the optional REST backend
/// password from `secret_store` (`<dest-id>`, `rest`) and passes it via `RESTIC_REST_PASSWORD`
/// (never on CLI arguments).
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

    let pw_command = spec.password_command_override.clone().unwrap_or_else(|| {
        format!(
            "{} secret get {}",
            quote_shell(&spec.kestic_binary.to_string_lossy()),
            spec.destination_id
        )
    });

    let mut rest_password: Option<SecretValue> = None;
    let mut repo_password_for_redaction: Option<SecretValue> = None;
    if let Some(store) = secret_store {
        if let Some(pw) = store.get_secret(destination.id.as_str(), SECRET_KIND_REPO)? {
            repo_password_for_redaction = Some(pw);
        }
        if destination.kind.uses_rest_credentials() {
            rest_password = Some(
                store
                    .get_secret(destination.id.as_str(), SECRET_KIND_REST)?
                    .ok_or_else(|| ResticRunError::MissingRestPassword(destination.id.clone()))?,
            );
        }
    }

    let (executable, args) = spec.resolve_invocation();
    let mut cmd = Command::new(&executable);
    cmd.args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("RESTIC_PASSWORD")
        .env("RESTIC_PASSWORD_COMMAND", &pw_command);

    let mut env_keys = vec!["RESTIC_PASSWORD_COMMAND".to_string()];

    if let Some(user) = spec.rest_username.as_deref().filter(|u| !u.is_empty()) {
        cmd.env("RESTIC_REST_USERNAME", user);
        env_keys.push("RESTIC_REST_USERNAME".to_string());
    }
    if let Some(rest_pw) = rest_password.as_ref() {
        cmd.env("RESTIC_REST_PASSWORD", rest_pw.as_str()?);
        env_keys.push("RESTIC_REST_PASSWORD".to_string());
    }
    for (k, v) in extra_env {
        cmd.env(k, v);
        env_keys.push((*k).to_string());
    }

    let mut child: Child = cmd.spawn()?;
    if let Some(handle) = cancel {
        handle.set_pid(Some(child.id()));
        if handle.is_cancelled() {
            handle.cancel();
        }
    }

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let mut secrets_to_redact: Vec<&str> = Vec::new();
    if let Some(s) = repo_password_for_redaction.as_ref() {
        secrets_to_redact.push(s.as_str()?);
    }
    if let Some(s) = rest_password.as_ref() {
        secrets_to_redact.push(s.as_str()?);
    }

    let stderr_handle = thread::spawn(move || -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(err_stream) = stderr {
            let reader = BufReader::new(err_stream);
            for line in reader.lines().map_while(Result::ok) {
                lines.push(line);
            }
        }
        lines
    });

    let mut log_lines = Vec::new();
    let mut stats: Option<RunStats> = None;
    let mut error_messages: Vec<ResticErrorMessage> = Vec::new();
    let mut exit_error_msg: Option<String> = None;

    if let Some(out_stream) = stdout {
        let reader = BufReader::new(out_stream);
        for line in reader.lines().map_while(Result::ok) {
            let redacted = redact_secrets(&line, &secrets_to_redact);
            if let Ok(event) = parse_json_line(&redacted) {
                match &event {
                    ResticStreamEvent::Summary(s) => stats = Some(s.clone()),
                    ResticStreamEvent::Error(e) => error_messages.push(e.clone()),
                    ResticStreamEvent::ExitError { message, .. } => {
                        exit_error_msg = Some(message.clone());
                    }
                    ResticStreamEvent::Status(_) | ResticStreamEvent::Unknown { .. } => {}
                }
                on_event(&event);
            }
            log_lines.push(redacted);
        }
    }

    let raw_stderr_lines = stderr_handle.join().unwrap_or_default();
    let mut redacted_stderr_lines = Vec::with_capacity(raw_stderr_lines.len());
    for line in raw_stderr_lines {
        let redacted = redact_secrets(&line, &secrets_to_redact);
        if let Ok(event) = parse_json_line(&redacted) {
            if let ResticStreamEvent::ExitError { message, .. } = &event {
                exit_error_msg = Some(message.clone());
            } else if let ResticStreamEvent::Error(e) = &event {
                error_messages.push(e.clone());
            }
            on_event(&event);
        }
        redacted_stderr_lines.push(redacted.clone());
        log_lines.push(redacted);
    }

    let status = child.wait()?;
    if let Some(handle) = cancel {
        handle.set_pid(None);
    }

    let finished_at = clock.now_utc();
    let duration_ms = (finished_at - started_at).whole_milliseconds().max(0) as u64;
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
        presentation.summary_de = "Vom Benutzer abgebrochen".to_string();
        presentation.summary_en = "Cancelled by user".to_string();
    }

    let command_record = CommandRecord {
        executable: executable.to_string_lossy().into_owned(),
        args,
        env_keys,
        started_at,
        duration_ms,
        exit_code,
    };

    Ok(ResticExecutionResult {
        exit_code,
        cancelled,
        presentation,
        stats,
        command_record,
        log_lines,
        errors: error_messages,
    })
}
