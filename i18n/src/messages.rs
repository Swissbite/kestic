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
    PendingReasonCode, RecoveryAction, ResticStatusCode, ServiceErrorCode, SkipReason,
    SourceStatus,
};

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
            Self::WrongPassword => i18n_noop("Saved password does not match"),
            Self::RepoMissing => i18n_noop("Repository is missing"),
            Self::Locked => i18n_noop("Repository is locked"),
            Self::DestinationUnreachable => i18n_noop("Destination unreachable"),
            Self::SourceUnavailable => i18n_noop("Source unavailable"),
            Self::AuthFailed => i18n_noop("Login to the destination failed"),
            Self::IncompleteSnapshot => {
                i18n_noop("Snapshot incomplete: some files could not be read")
            }
            Self::RetentionPending => i18n_noop("Backed up, retention not applied yet"),
            Self::Cancelled => i18n_noop("Cancelled by user"),
            Self::SecretStoreLocked => i18n_noop("Password store is locked"),
            Self::ResticUnavailable => i18n_noop("restic is unavailable or too old."),
            Self::Unknown if is_warning => i18n_noop("Backed up with warning"),
            Self::Unknown => {
                return match exit_code {
                    Some(code) if code != 1 => Message::with_args(
                        i18n_noop("Unknown error (code %1)"),
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

/// «Completed»: a run that ended without error.
pub fn completed_message() -> Message {
    Message::new(i18n_noop("Completed"))
}

/// The destination host name could not be resolved.
pub fn dns_failed_message() -> Message {
    Message::new(i18n_noop("Destination name not found (DNS)"))
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
            Self::Backoff => i18n_noop("waiting time before next attempt"),
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
            Some(Self::DriveEmpty) => i18n_noop("Source unavailable: folder is empty"),
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
            Self::ConditionNotMet => Message::new(i18n_noop("Run conditions not met")),
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
            Self::PermanentError => i18n_noop("Permanent error, needs fixing"),
            Self::SourceUnavailable => return UiText::Message(SourceStatus::message(None)),
            Self::DestinationUnreachable => {
                return UiText::Message(ErrorKind::DestinationUnreachable.message(None, false));
            }
            Self::ConditionNotMet => {
                return UiText::Message(BlockedReason::ConditionNotMet.message(None));
            }
            Self::Error => return error_kind.unwrap_or(ErrorKind::Unknown).text(detail),
        };
        UiText::Message(Message::new(msgid))
    }
}

/// Parameters of the restic status texts (from `ResticStatusDto` and the minimum version).
#[derive(Debug, Clone, Copy, Default)]
pub struct ResticStatusParams<'a> {
    pub binary_path: Option<&'a str>,
    pub found_version: Option<&'a str>,
    /// Raw reason for `invalid_custom_path`, raw detail for `unreadable_version`.
    pub detail: Option<&'a str>,
    /// Minimum restic version Kestic needs.
    pub min_version: &'a str,
}

impl ResticStatusCode {
    /// Why restic is unusable, filled in from `params`.
    pub fn message(self, params: &ResticStatusParams<'_>) -> Message {
        let min = params.min_version;
        match self {
            Self::NotFound => {
                Message::with_args(i18n_noop("restic not found; at least %1 required"), &[min])
            }
            Self::TooOld => Message::with_args(
                i18n_noop("restic %1 is too old; Kestic needs at least %2"),
                &[params.found_version.unwrap_or_default(), min],
            ),
            Self::InvalidCustomPath => Message::with_args(
                i18n_noop("Custom restic path is invalid (%1): %2"),
                &[
                    params.binary_path.unwrap_or_default(),
                    params.detail.unwrap_or_default(),
                ],
            ),
            Self::UnreadableVersion => Message::with_args(
                i18n_noop("Cannot read the restic version (%1)"),
                &[params.detail.unwrap_or_default()],
            ),
        }
    }
}

/// Restic status line: «restic ‹version›» when usable, otherwise the reason.
pub fn restic_status_text(
    error_kind: Option<ResticStatusCode>,
    version: Option<&str>,
    params: &ResticStatusParams<'_>,
) -> UiText {
    match error_kind {
        Some(code) => UiText::Message(code.message(params)),
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
            Self::NotWritable => Message::with_args(
                i18n_noop("Kestic cannot write to the folder: %1"),
                &[detail],
            ),
            Self::MissingPassword => {
                Message::new(i18n_noop("No repository password entered or saved."))
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
    /// Codes with the same meaning as an [`ErrorKind`] use its text.
    pub fn message(self) -> Message {
        let msgid = match self {
            Self::InvalidInput => i18n_noop("Invalid input."),
            Self::NotFound => i18n_noop("The item no longer exists."),
            Self::InUse => i18n_noop("Remove or reassign all backups on this destination first."),
            Self::ManualRunBlocked => {
                i18n_noop("Backup could not start because source or destination is unavailable.")
            }
            Self::DestinationUnreachable => {
                return ErrorKind::DestinationUnreachable.message(None, false);
            }
            Self::WrongPassword => {
                i18n_noop("Wrong password: the repository rejected this password.")
            }
            Self::AuthFailed => return ErrorKind::AuthFailed.message(None, false),
            Self::SecretStoreLocked => return ErrorKind::SecretStoreLocked.message(None, false),
            Self::ResticUnavailable => return ErrorKind::ResticUnavailable.message(None, false),
            Self::AlreadyRunning => i18n_noop("Another run is already active on this destination."),
            Self::LockInUse => {
                i18n_noop("Cannot release the lock: another program still uses the repository.")
            }
            Self::DuplicateLocation => {
                i18n_noop("This location is already used by another destination.")
            }
            Self::Internal => i18n_noop("Internal error in the background service."),
            Self::DaemonUnreachable => i18n_noop("Background service unavailable."),
        };
        Message::new(msgid)
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

/// «Schedules paused» (overview, tray).
pub fn schedules_paused_message() -> Message {
    Message::new(i18n_noop("Schedules paused"))
}

/// «No backups configured yet» (overview, tray, backup list).
pub fn no_backups_message() -> Message {
    Message::new(i18n_noop("No backups configured yet"))
}

/// «Not backed up yet» (overview, backup card).
pub fn not_backed_up_message() -> Message {
    Message::new(i18n_noop("Not backed up yet"))
}

/// «All data backed up» (overview, tray).
pub fn all_backed_up_message() -> Message {
    Message::new(i18n_noop("All data backed up"))
}

/// Maintenance of a destination is running right now (status line, tray).
pub fn maintaining_message(destination_name: &str) -> Message {
    Message::with_args(i18n_noop("Maintaining %1"), &[destination_name])
}

/// Maintenance is running, without naming the destination.
pub fn maintenance_running_message() -> Message {
    Message::new(i18n_noop("Maintenance running"))
}

/// Name shown for a destination that no longer exists (run history).
pub fn deleted_destination_message() -> Message {
    Message::new(i18n_noop("Deleted destination"))
}

/// A run stopped because the computer went to sleep (run history, backup card).
pub fn interrupted_by_sleep_message() -> Message {
    Message::new(i18n_noop("Interrupted by system sleep"))
}

/// Pause line: `since` and `until` are already formatted times.
pub fn pause_message(since: &str, until: Option<&str>) -> Message {
    match until {
        Some(until) => Message::with_args(i18n_noop("Paused since %1 (until %2)"), &[since, until]),
        None => Message::with_args(i18n_noop("Paused since %1"), &[since]),
    }
}

/// UI labels that both the tray (daemon) and the window (GUI) show. QML asks for them by
/// code (`appBackend.sharedLabel("back_up_now")`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Label {
    BackUpNow,
    PauseSchedules,
    ResumeSchedules,
    AllBackups,
    PauseOneHour,
    PauseThreeHours,
    PauseUntilTomorrow,
    PauseUntilResume,
    NoBackups,
    AllBackedUp,
    /// A destination with backups cannot be removed (same text as the `InUse` error).
    RemoveBackupsFirst,
}

impl Label {
    pub const ALL: [Self; 11] = [
        Self::BackUpNow,
        Self::PauseSchedules,
        Self::ResumeSchedules,
        Self::AllBackups,
        Self::PauseOneHour,
        Self::PauseThreeHours,
        Self::PauseUntilTomorrow,
        Self::PauseUntilResume,
        Self::NoBackups,
        Self::AllBackedUp,
        Self::RemoveBackupsFirst,
    ];

    /// Code used by QML.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BackUpNow => "back_up_now",
            Self::PauseSchedules => "pause_schedules",
            Self::ResumeSchedules => "resume_schedules",
            Self::AllBackups => "all_backups",
            Self::PauseOneHour => "pause_one_hour",
            Self::PauseThreeHours => "pause_three_hours",
            Self::PauseUntilTomorrow => "pause_until_tomorrow",
            Self::PauseUntilResume => "pause_until_resume",
            Self::NoBackups => "no_backups",
            Self::AllBackedUp => "all_data_backed_up",
            Self::RemoveBackupsFirst => "remove_backups_first",
        }
    }

    /// Parses a label code; `None` for unknown codes.
    pub fn parse(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.as_str() == code)
    }

    pub fn message(self) -> Message {
        match self {
            Self::BackUpNow => Message::new(i18n_noop("Back Up Now")),
            Self::PauseSchedules => Message::new(i18n_noop("Pause Schedules")),
            Self::ResumeSchedules => Message::new(i18n_noop("Resume Schedules")),
            Self::AllBackups => Message::new(i18n_noop("All Backups")),
            Self::PauseOneHour => Message::new(i18n_noop("1 Hour")),
            Self::PauseThreeHours => Message::new(i18n_noop("3 Hours")),
            Self::PauseUntilTomorrow => Message::new(i18n_noop("Until Tomorrow")),
            Self::PauseUntilResume => Message::new(i18n_noop("Until I Resume")),
            Self::NoBackups => no_backups_message(),
            Self::AllBackedUp => all_backed_up_message(),
            Self::RemoveBackupsFirst => ServiceErrorCode::InUse.message(),
        }
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
        maintaining_message(run.name)
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
        return schedules_paused_message();
    }
    if counts.backups == 0 {
        return no_backups_message();
    }
    match counts.kind {
        OverallStatusKind::Interrupted => Message::new(i18n_noop("Last run interrupted")),
        OverallStatusKind::Cancelled => Message::new(i18n_noop("Last run cancelled")),
        OverallStatusKind::NeverRun => not_backed_up_message(),
        _ => all_backed_up_message(),
    }
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
