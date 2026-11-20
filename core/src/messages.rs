// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The single mapping from language-independent codes ([`crate::codes`]) to English msgids
//! (`NFR-06`, `docs/spec/dbus-api.md` §Sprachunabhängige Codes).
//!
//! `core` never translates: it returns a [`Message`] (msgid + `%1`… arguments). The daemon (tray,
//! notifications, run history) and the GUI translate it with their own i18n in their own
//! language, both from `po/de/kestic.po`. Every msgid is wrapped in [`i18n_noop`] so
//! `xgettext --keyword=i18n_noop:1` extracts it.

use crate::codes::{
    BlockedReason, BlockingKind, ConnectionTestCode, ErrorKind, OverallStatusKind,
    PendingReasonCode, ResticStatusCode, ServiceErrorCode, SkipReason, SourceStatus,
};
use crate::provider::MIN_RESTIC_VERSION;
use crate::restic::{RecoveryAction, ResticFailureKind};

/// Marks `msgid` for extraction (`xgettext --keyword=i18n_noop:1`) without translating it.
pub const fn i18n_noop(msgid: &'static str) -> &'static str {
    msgid
}

/// Translatable text: English msgid with `%1`… placeholders and their arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub msgid: &'static str,
    pub args: Vec<String>,
}

impl Message {
    /// Message without placeholders.
    pub fn new(msgid: &'static str) -> Self {
        Self {
            msgid,
            args: Vec::new(),
        }
    }

    /// Message whose `%1`… placeholders are filled from `args`.
    pub fn with_args(msgid: &'static str, args: &[&str]) -> Self {
        Self {
            msgid,
            args: args.iter().map(|a| (*a).to_string()).collect(),
        }
    }

    /// Untranslated English text (msgid with substituted arguments), for logs and fallbacks.
    pub fn english(&self) -> String {
        substitute_placeholders(self.msgid, &self.args)
    }
}

/// Single-pass `%1`..`%9` substitution (a `%` not followed by a used digit stays as it is).
pub fn substitute_placeholders(template: &str, args: &[String]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '%'
            && let Some(digit) = chars.peek().and_then(|d| d.to_digit(10))
            && digit > 0
            && let Some(arg) = args.get(digit as usize - 1)
        {
            out.push_str(arg);
            chars.next();
            continue;
        }
        out.push(ch);
    }
    out
}

/// Text shown to the user: either a translatable message or raw data (restic or system
/// error detail), which is shown as it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiText {
    Message(Message),
    Raw(String),
}

/// Rounded percentage (0–100) for progress lines.
fn percent(percent_done: f64) -> String {
    (percent_done.round().clamp(0.0, 100.0) as u32).to_string()
}

impl ErrorKind {
    /// Cause of a failed run or a run with warning. `exit_code` only matters for `unknown`
    /// errors; an `unknown` warning is «Backed up with warning».
    pub fn message(self, exit_code: Option<i32>, is_warning: bool) -> Message {
        let msgid = match self {
            Self::WrongPassword => i18n_noop("Stored password does not match"),
            Self::RepoMissing => i18n_noop("Repository is missing"),
            Self::Locked => i18n_noop("Repository is locked"),
            Self::DestinationUnreachable => i18n_noop("Destination unreachable"),
            Self::SourceUnavailable => i18n_noop("Source unavailable"),
            Self::AuthFailed => i18n_noop("Authentication at destination failed"),
            Self::IncompleteSnapshot => {
                i18n_noop("Snapshot incomplete: some files could not be read")
            }
            Self::RetentionPending => i18n_noop("Backed up, retention not executed"),
            Self::Cancelled => i18n_noop("Cancelled by user"),
            Self::SecretStoreLocked => i18n_noop("Password store is locked"),
            Self::ResticUnavailable => i18n_noop("restic is unavailable or too old."),
            Self::Unknown if is_warning => i18n_noop("Backed up with warning"),
            Self::Unknown => {
                return match exit_code {
                    Some(code) if code != 1 => Message::with_args(
                        i18n_noop("Unknown error (exit %1)"),
                        &[&code.to_string()],
                    ),
                    _ => Message::new(i18n_noop("Backup command failed")),
                };
            }
        };
        Message::new(msgid)
    }
}

impl ErrorKind {
    /// Text of an error without run context: the message of the kind, or for `unknown` the
    /// raw `detail` (English restic, system or daemon error text) when there is one.
    pub fn text(self, detail: Option<&str>) -> UiText {
        match detail {
            Some(raw) if self == Self::Unknown && !raw.trim().is_empty() => {
                UiText::Raw(raw.to_string())
            }
            _ => UiText::Message(self.message(None, false)),
        }
    }
}

impl ResticFailureKind {
    /// One-sentence summary of a classified restic outcome (`FR-37`) for notifications and the
    /// run history; finer than [`ErrorKind`] (DNS, «Completed»).
    pub fn summary_message(&self) -> Message {
        match self {
            Self::None => Message::new(i18n_noop("Completed")),
            Self::DnsResolutionFailed => Message::new(i18n_noop(
                "Destination hostname could not be resolved (DNS)",
            )),
            Self::GenericFailure => Message::new(i18n_noop("Backup command failed")),
            Self::Unknown { exit_code } => Message::with_args(
                i18n_noop("Unknown error (exit %1)"),
                &[&exit_code.to_string()],
            ),
            other => ErrorKind::from(other).message(None, false),
        }
    }
}

impl RecoveryAction {
    /// Button label of the recovery action (`FR-37`).
    pub fn label_message(self) -> Message {
        Message::new(match self {
            Self::UpdateStoredPassword => i18n_noop("Update Saved Password…"),
            Self::AddToExcludes => i18n_noop("Add to Exclude Rules…"),
            Self::CheckResticSource => i18n_noop("Configure restic Source…"),
            Self::UnlockRepository => i18n_noop("Release Lock…"),
        })
    }
}

impl PendingReasonCode {
    /// Status line «Waiting: …» of a waiting backup or of waiting maintenance;
    /// `retry_hint` is the formatted next attempt for `backoff` («–» when unknown).
    pub fn waiting_message(self, retry_hint: &str) -> Message {
        let msgid = match self {
            Self::SourceUnavailable => i18n_noop("Waiting: source unavailable"),
            Self::DestinationUnreachable => i18n_noop("Waiting: destination unreachable"),
            Self::OnBattery => i18n_noop("Waiting: AC power"),
            Self::NoNetwork => i18n_noop("Waiting: network"),
            Self::Metered => i18n_noop("Waiting: unmetered connection"),
            Self::WrongNetwork => i18n_noop("Waiting: allowed network"),
            Self::Queue => i18n_noop("Waiting: another run on this destination"),
            Self::Backoff => {
                let hint = if retry_hint.is_empty() {
                    "–"
                } else {
                    retry_hint
                };
                return Message::with_args(i18n_noop("Waiting: next attempt at %1"), &[hint]);
            }
        };
        Message::new(msgid)
    }

    /// The missing precondition as part of «Condition not met: …» (`battery_percent` and the
    /// allowed networks of the backup fill in the details).
    pub fn condition_message(
        self,
        battery_percent: Option<u8>,
        allowed_networks: &[String],
    ) -> Message {
        let msgid = match self {
            Self::OnBattery => {
                return match battery_percent {
                    Some(pct) => {
                        Message::with_args(i18n_noop("AC power (battery %1%)"), &[&pct.to_string()])
                    }
                    None => Message::new(i18n_noop("AC power")),
                };
            }
            Self::WrongNetwork => {
                return Message::with_args(
                    i18n_noop("allowed network (%1)"),
                    &[&allowed_networks.join(", ")],
                );
            }
            Self::NoNetwork => i18n_noop("network connection"),
            Self::Metered => i18n_noop("unmetered connection"),
            Self::SourceUnavailable => i18n_noop("source available"),
            Self::DestinationUnreachable => i18n_noop("destination reachable"),
            Self::Queue => i18n_noop("another run on this destination"),
            Self::Backoff => i18n_noop("retry cooldown"),
        };
        Message::new(msgid)
    }
}

/// «Condition not met: %1» around the already translated, comma-joined condition parts.
pub fn unmet_conditions_message(translated_parts: &str) -> Message {
    Message::with_args(i18n_noop("Condition not met: %1"), &[translated_parts])
}

impl SourceStatus {
    /// Why the source is unavailable; `None` (no detail) is plain «Source unavailable».
    pub fn message(status: Option<Self>) -> Message {
        Message::new(match status {
            Some(Self::DriveNotMounted) => i18n_noop("Source unavailable: drive not mounted"),
            Some(Self::DriveMismatch) => i18n_noop("Source unavailable: different drive"),
            Some(Self::DriveEmpty) => i18n_noop("Source unavailable: source empty"),
            Some(Self::PathMissing) | None => i18n_noop("Source unavailable"),
        })
    }
}

impl BlockedReason {
    /// Availability reason next to «Back Up Now» (FR-30).
    pub fn message(self, source_status: Option<SourceStatus>) -> Message {
        match self {
            Self::SourceUnavailable => SourceStatus::message(source_status),
            Self::DestinationUnreachable => Message::new(i18n_noop("Destination unreachable")),
            Self::ConditionNotMet => Message::new(i18n_noop("Run condition not met")),
        }
    }

    /// Status line of a scheduled backup that waits because it is unavailable; without a
    /// known reason «Waiting: unavailable».
    pub fn waiting_message(reason: Option<Self>, source_status: Option<SourceStatus>) -> Message {
        match reason {
            Some(Self::SourceUnavailable) => SourceStatus::message(source_status),
            Some(Self::DestinationUnreachable) => {
                PendingReasonCode::DestinationUnreachable.waiting_message("")
            }
            Some(Self::ConditionNotMet) | None => Message::new(i18n_noop("Waiting: unavailable")),
        }
    }

    /// Message after a blocked manual start (`ManualRunBlocked`, FR-30).
    pub fn manual_block_message(self, backup_name: &str) -> Message {
        let msgid = match self {
            Self::SourceUnavailable => i18n_noop("Cannot back up '%1': source is unavailable."),
            Self::ConditionNotMet => i18n_noop("Cannot back up '%1': run conditions are not met."),
            Self::DestinationUnreachable => {
                i18n_noop("Cannot back up '%1': destination is unreachable.")
            }
        };
        Message::with_args(msgid, &[backup_name])
    }
}

impl SkipReason {
    /// Reason of a backup skipped by «Back Up All Now» (FR-13b). For `error`, `error_kind`
    /// names the cause; an unknown cause shows the raw `detail`.
    pub fn text(self, error_kind: Option<ErrorKind>, detail: Option<&str>) -> UiText {
        let msgid = match self {
            Self::PermanentError => i18n_noop("permanent error, fix required"),
            Self::SourceUnavailable => i18n_noop("source unavailable"),
            Self::DestinationUnreachable => i18n_noop("destination unreachable"),
            Self::ConditionNotMet => i18n_noop("run conditions not met"),
            Self::Error => return error_kind.unwrap_or(ErrorKind::Unknown).text(detail),
        };
        UiText::Message(Message::new(msgid))
    }
}

impl ResticStatusCode {
    /// Why restic is unusable; `binary_path`, `found_version` and the raw `detail` fill in
    /// the placeholders.
    pub fn message(
        self,
        binary_path: Option<&str>,
        found_version: Option<&str>,
        detail: Option<&str>,
    ) -> Message {
        let min = MIN_RESTIC_VERSION.to_string();
        match self {
            Self::NotFound => {
                Message::with_args(i18n_noop("restic not found; at least %1 required"), &[&min])
            }
            Self::TooOld => Message::with_args(
                i18n_noop("restic %1 is too old; Kestic needs at least %2"),
                &[found_version.unwrap_or_default(), &min],
            ),
            Self::InvalidCustomPath => Message::with_args(
                i18n_noop("Custom restic path is invalid (%1): %2"),
                &[binary_path.unwrap_or_default(), detail.unwrap_or_default()],
            ),
            Self::UnreadableVersion => Message::with_args(
                i18n_noop("Unable to determine restic version (%1)"),
                &[detail.unwrap_or_default()],
            ),
        }
    }
}

/// Restic status line: «restic ‹version›» when usable, otherwise the reason.
pub fn restic_status_text(
    error_kind: Option<ResticStatusCode>,
    version: Option<&str>,
    binary_path: Option<&str>,
    found_version: Option<&str>,
    detail: Option<&str>,
) -> UiText {
    match error_kind {
        Some(code) => UiText::Message(code.message(binary_path, found_version, detail)),
        None => UiText::Raw(format!("restic {}", version.unwrap_or_default())),
    }
}

impl BlockingKind {
    /// Blocking-state text (§6.1); `restic` is the reason of an unusable restic.
    pub fn message(self, restic: Option<Message>) -> Message {
        match self {
            Self::SecretStoreLocked => Message::new(i18n_noop(
                "The system password store (Secret Service / KWallet) is locked. Kestic cannot access destinations.",
            )),
            Self::ResticUnavailable => {
                restic.unwrap_or_else(|| ErrorKind::ResticUnavailable.message(None, false))
            }
        }
    }
}

impl ConnectionTestCode {
    /// Failure text of a connection test. Only codes whose `detail` is the actual information
    /// (`invalid_input`, `internal`, `command_failed`, `daemon_unreachable`) show it raw.
    pub fn text(self, detail: &str) -> UiText {
        let message = match self {
            Self::DuplicateLocation => Message::with_args(
                i18n_noop("This location is already used by destination '%1'."),
                &[detail],
            ),
            Self::RepositoryExists => Message::new(i18n_noop(
                "This folder already contains a restic repository. Choose 'Connect existing repository' instead.",
            )),
            Self::NotWritable => {
                Message::with_args(i18n_noop("Folder is not writable: %1"), &[detail])
            }
            Self::MissingPassword => {
                Message::new(i18n_noop("No repository password provided or stored."))
            }
            Self::DestinationUnreachable => ErrorKind::DestinationUnreachable.message(None, false),
            Self::SecretStoreLocked => ErrorKind::SecretStoreLocked.message(None, false),
            Self::ResticUnavailable => ErrorKind::ResticUnavailable.message(None, false),
            Self::WrongPassword => ErrorKind::WrongPassword.message(None, false),
            Self::AuthFailed => ErrorKind::AuthFailed.message(None, false),
            Self::RepositoryMissing => ErrorKind::RepoMissing.message(None, false),
            Self::InvalidInput | Self::Internal | Self::CommandFailed | Self::DaemonUnreachable => {
                return UiText::Raw(detail.to_string());
            }
        };
        UiText::Message(message)
    }
}

impl ServiceErrorCode {
    /// Text of a failed D-Bus call; the error message itself is never shown as main text.
    pub fn message(self) -> Message {
        Message::new(match self {
            Self::InvalidInput => i18n_noop("Invalid input."),
            Self::NotFound => i18n_noop("The item no longer exists."),
            Self::InUse => i18n_noop("Remove or reassign all backups on this destination first."),
            Self::ManualRunBlocked => {
                i18n_noop("Backup could not start because source or destination is unavailable.")
            }
            Self::DestinationUnreachable => i18n_noop("Destination unreachable."),
            Self::WrongPassword => {
                i18n_noop("Wrong password: the repository rejected this password.")
            }
            Self::AuthFailed => i18n_noop("Authentication at destination failed."),
            Self::SecretStoreLocked => {
                i18n_noop("The system password store (Secret Service / KWallet) is locked.")
            }
            Self::ResticUnavailable => i18n_noop("restic is unavailable or too old."),
            Self::AlreadyRunning => i18n_noop("Another run is already active on this destination."),
            Self::LockInUse => {
                i18n_noop("Cannot unlock: repository lock belongs to an active or remote process")
            }
            Self::DuplicateLocation => {
                i18n_noop("This location is already used by another destination.")
            }
            Self::Internal => i18n_noop("Internal error in the background service."),
            Self::DaemonUnreachable => i18n_noop("Background service unavailable."),
        })
    }
}

/// Result line of a connection test: «Reachable» or the failure text.
pub fn connection_test_text(ok: bool, code: Option<ConnectionTestCode>, detail: &str) -> UiText {
    match (ok, code) {
        (true, _) => UiText::Message(Message::new(i18n_noop("Reachable"))),
        (false, Some(code)) => code.text(detail),
        (false, None) => UiText::Raw(detail.to_string()),
    }
}

/// The active run as shown in headline, tray and subtitle.
#[derive(Debug, Clone, Copy)]
pub struct ActiveRunInfo<'a> {
    /// Backup name, or destination name for maintenance.
    pub name: &'a str,
    pub is_maintenance: bool,
    pub percent_done: f64,
}

/// «‹Backup›: backing up · n%» or «Maintaining ‹Destination›» (glossary: Wartung).
pub fn active_run_message(run: &ActiveRunInfo<'_>) -> Message {
    if run.is_maintenance {
        Message::with_args(i18n_noop("Maintaining %1"), &[run.name])
    } else {
        Message::with_args(
            i18n_noop("%1: backing up · %2%"),
            &[run.name, &percent(run.percent_done)],
        )
    }
}

/// Counts behind the overall status (`OverallStatusDto`).
#[derive(Debug, Clone, Copy)]
pub struct OverallCounts {
    pub kind: OverallStatusKind,
    pub failed: usize,
    pub overdue: usize,
    pub warning: usize,
    pub waiting: usize,
    pub backups: usize,
    /// Schedules paused globally.
    pub paused: bool,
}

/// Headline of the overall status (§6.2 priority: blocked, failed, overdue, warning, running,
/// waiting, paused, empty, interrupted, cancelled, never run, ok) for overview and tray.
pub fn overall_headline(
    counts: &OverallCounts,
    active: Option<&ActiveRunInfo<'_>>,
    blocking: Option<Message>,
) -> Message {
    let n = |c: usize| c.to_string();
    if let Some(blocking) = blocking {
        return blocking;
    }
    if counts.failed > 0 {
        return match counts.failed {
            1 => Message::new(i18n_noop("1 backup failed")),
            c => Message::with_args(i18n_noop("%1 backups failed"), &[&n(c)]),
        };
    }
    if counts.overdue > 0 {
        return match counts.overdue {
            1 => Message::new(i18n_noop("1 backup overdue")),
            c => Message::with_args(i18n_noop("%1 backups overdue"), &[&n(c)]),
        };
    }
    if counts.warning > 0 {
        return match counts.warning {
            1 => Message::new(i18n_noop("1 backup finished with warning")),
            c => Message::with_args(i18n_noop("%1 backups finished with warnings"), &[&n(c)]),
        };
    }
    if let Some(active) = active {
        return active_run_message(active);
    }
    if counts.waiting > 0 {
        return waiting_headline(counts);
    }
    if counts.paused || counts.kind == OverallStatusKind::Paused {
        return Message::new(i18n_noop("Schedules paused"));
    }
    if counts.backups == 0 {
        return Message::new(i18n_noop("No backups configured yet"));
    }
    Message::new(match counts.kind {
        OverallStatusKind::Interrupted => i18n_noop("Last run interrupted"),
        OverallStatusKind::Cancelled => i18n_noop("Last run cancelled"),
        OverallStatusKind::NeverRun => i18n_noop("Not backed up yet"),
        _ => i18n_noop("All data backed up"),
    })
}

fn waiting_headline(counts: &OverallCounts) -> Message {
    let waiting = counts.waiting.to_string();
    match (counts.paused, counts.waiting) {
        (true, 1) => Message::new(i18n_noop("Schedules paused · 1 backup waiting")),
        (true, _) => Message::with_args(
            i18n_noop("Schedules paused · %1 backups waiting"),
            &[&waiting],
        ),
        (false, 1) => Message::new(i18n_noop("1 backup waiting")),
        (false, _) => Message::with_args(
            i18n_noop("%1 of %2 backups waiting"),
            &[&waiting, &counts.backups.to_string()],
        ),
    }
}

/// Subtitle while a run is active next to failed or overdue backups (the headline names
/// those); `None` otherwise.
pub fn overall_subtitle(
    counts: &OverallCounts,
    active: Option<&ActiveRunInfo<'_>>,
) -> Option<Message> {
    let active = active?;
    if counts.failed == 0 && counts.overdue == 0 {
        return None;
    }
    if active.is_maintenance {
        return Some(active_run_message(active));
    }
    Some(Message::with_args(
        i18n_noop("Currently running: %1 (%2%)"),
        &[active.name, &percent(active.percent_done)],
    ))
}

#[cfg(test)]
#[path = "messages_tests.rs"]
mod tests;
