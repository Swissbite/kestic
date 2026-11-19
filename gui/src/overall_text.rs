// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Overview headline and subtitle, built in the GUI locale from the raw
//! `OverallStatusDto` fields (counts, kind) instead of the daemon-provided
//! text, so a German GUI never shows English daemon strings (NFR-06).

use crate::state_cache::{i18n, i18np};
use kestic_core::provider::MIN_RESTIC_VERSION;
use kestic_daemon::dbus_api::{
    ActiveRunProgressDto, BackupAvailabilityDto, DaemonStateDto, OverallStatusDto,
    OverallStatusKind, REASON_DESTINATION_UNREACHABLE, REASON_SOURCE_UNAVAILABLE,
};

/// Headline for the status card (§6.2 priority order: failed, overdue, warning,
/// running, waiting, paused, empty, interrupted, cancelled, never run, ok).
pub fn overall_headline(state: &DaemonStateDto) -> String {
    let o = &state.overall;
    if state.blocking.is_some() {
        return localized_blocking_message(state);
    }
    if o.failed_count > 0 {
        return if o.failed_count == 1 {
            i18n("1 backup failed").to_string()
        } else {
            i18n("%1 backups failed").subs(&[&o.failed_count.to_string()])
        };
    }
    if o.overdue_count > 0 {
        return if o.overdue_count == 1 {
            i18n("1 backup overdue").to_string()
        } else {
            i18n("%1 backups overdue").subs(&[&o.overdue_count.to_string()])
        };
    }
    if o.warning_count > 0 {
        return if o.warning_count == 1 {
            i18n("1 backup finished with warning").to_string()
        } else {
            i18n("%1 backups finished with warnings").subs(&[&o.warning_count.to_string()])
        };
    }
    if let Some(active) = &o.active_run_progress {
        return active_run_line(active);
    }
    if o.waiting_count > 0 {
        return waiting_line(o, state.backups.len());
    }
    headline_without_counts(o, state.backups.is_empty())
}

fn waiting_line(o: &OverallStatusDto, backup_count: usize) -> String {
    if o.paused {
        if o.waiting_count == 1 {
            i18n("Schedules paused · 1 backup waiting").to_string()
        } else {
            i18n("Schedules paused · %1 backups waiting").subs(&[&o.waiting_count.to_string()])
        }
    } else if o.waiting_count == 1 {
        i18n("1 backup waiting").to_string()
    } else {
        i18n("%1 of %2 backups waiting")
            .subs(&[&o.waiting_count.to_string(), &backup_count.to_string()])
    }
}

fn headline_without_counts(o: &OverallStatusDto, no_backups: bool) -> String {
    if o.paused || o.kind == OverallStatusKind::Paused {
        i18n("Schedules paused").to_string()
    } else if no_backups {
        i18n("No backups configured yet").to_string()
    } else {
        match o.kind {
            OverallStatusKind::Interrupted => i18n("Last run interrupted").to_string(),
            OverallStatusKind::Cancelled => i18n("Last run cancelled").to_string(),
            OverallStatusKind::NeverRun => i18n("Not backed up yet").to_string(),
            _ => i18n("All data backed up").to_string(),
        }
    }
}

/// Subtitle while a run is active next to failed/overdue backups.
pub fn overall_subtitle(o: &OverallStatusDto) -> String {
    match &o.active_run_progress {
        Some(active)
            if (o.failed_count > 0 || o.overdue_count > 0) && active.backup_id.is_none() =>
        {
            active_run_line(active)
        }
        Some(active) if o.failed_count > 0 || o.overdue_count > 0 => {
            let pct = active.percent_done.round().clamp(0.0, 100.0) as u32;
            i18n("Currently running: %1 (%2%)").subs(&[&active.backup_name, &pct.to_string()])
        }
        _ => String::new(),
    }
}

/// Number of backups «Back Up All Now» can start right now (source and destination
/// available, no permanent error, run conditions met) and, when that is not every
/// backup, the hint shown before the click (FR-13b).
pub fn back_up_all_availability(state: &DaemonStateDto) -> (usize, String) {
    let total = state.backups.len();
    let startable = state
        .backups
        .iter()
        .filter(|b| b.availability.available && !b.has_error && !b.has_unmet_run_condition)
        .count();
    let hint = if total > 0 && startable < total {
        i18np(
            "Only %2 of %1 backup can be backed up now.",
            "Only %2 of %1 backups can be backed up now.",
            total,
            &[&startable.to_string()],
        )
    } else {
        String::new()
    };
    (startable, hint)
}

/// Status line of the active run: backups «‹Backup›: backing up · n%», destination
/// maintenance (no backup id) «Maintaining ‹Destination›» (glossary: Wartung).
pub fn active_run_line(active: &ActiveRunProgressDto) -> String {
    if active.backup_id.is_none() {
        return i18n("Maintaining %1").subs(&[&active.backup_name]);
    }
    let pct = active.percent_done.round().clamp(0.0, 100.0) as u32;
    i18n("%1: backing up · %2%").subs(&[&active.backup_name, &pct.to_string()])
}

/// Maintenance state line of a destination (detail page): running, or why due maintenance
/// waits (`maintenance_pending_reason`, AC-M3-05-5), in the GUI locale.
pub fn maintenance_status_line(
    running: bool,
    pending_code: Option<&str>,
    retry_at_unix: Option<i64>,
    now: time::OffsetDateTime,
) -> String {
    if running {
        return i18n("Maintenance running").to_string();
    }
    match pending_code {
        Some(code) => {
            crate::state_cache::localized_pending_reason(Some(code), "", retry_at_unix, now)
        }
        None => String::new(),
    }
}

/// Blocking-state text (§6.1) in the GUI locale from `BlockingStateDto.kind` and the
/// `ResticStatusDto` parameters; the daemon text is only the fallback for unknown codes.
pub fn localized_blocking_message(state: &DaemonStateDto) -> String {
    let Some(b) = &state.blocking else {
        return String::new();
    };
    let rs = &state.restic_status;
    let min = MIN_RESTIC_VERSION.to_string();
    match (b.kind.as_str(), rs.error_kind.as_deref()) {
        ("secret_store_locked", _) => i18n(
            "The system password store (Secret Service / KWallet) is locked. Kestic cannot access destinations.",
        )
        .to_string(),
        ("restic_unavailable", Some("not_found")) => {
            i18n("restic not found; at least %1 required").subs(&[&min])
        }
        ("restic_unavailable", Some("too_old")) => {
            let found = rs.found_version.clone().unwrap_or_default();
            i18n("restic %1 is too old; Kestic needs at least %2").subs(&[&found, &min])
        }
        ("restic_unavailable", Some("invalid_custom_path")) if rs.detail.is_some() => {
            let path = rs.binary_path.clone().unwrap_or_default();
            let reason = rs.detail.clone().unwrap_or_default();
            i18n("Custom restic path is invalid (%1): %2").subs(&[&path, &reason])
        }
        ("restic_unavailable", Some("unreadable_version")) if rs.detail.is_some() => {
            let detail = rs.detail.clone().unwrap_or_default();
            i18n("Unable to determine restic version (%1)").subs(&[&detail])
        }
        _ => b.message.clone(),
    }
}

/// Cause of the last run's error/warning in the GUI locale from the daemon's `error_kind`
/// code (`dbus-api.md`); `daemon_text` is only used when no code is available.
pub fn localized_error_kind(
    kind: Option<&str>,
    exit_code: Option<i32>,
    is_warning: bool,
    daemon_text: Option<&str>,
) -> String {
    match kind {
        Some("wrong_password") => i18n("Stored password does not match").to_string(),
        Some("repo_missing") => i18n("Repository is missing").to_string(),
        Some("locked") => i18n("Repository is locked").to_string(),
        Some("destination_unreachable") => i18n("Destination unreachable").to_string(),
        Some("source_unavailable") => i18n("Source unavailable").to_string(),
        Some("auth_failed") => i18n("Authentication at destination failed").to_string(),
        Some("incomplete_snapshot") => {
            i18n("Snapshot incomplete: some files could not be read").to_string()
        }
        Some("retention_pending") => i18n("Backed up, retention not executed").to_string(),
        Some("cancelled") => i18n("Cancelled by user").to_string(),
        Some(_) if is_warning => i18n("Backed up with warning").to_string(),
        Some(_) => match exit_code {
            Some(code) if code != 1 => i18n("Unknown error (exit %1)").subs(&[&code.to_string()]),
            _ => i18n("Backup command failed").to_string(),
        },
        None => daemon_text.unwrap_or_default().to_string(),
    }
}

/// Source-unavailable detail in the GUI locale from `BackupAvailabilityDto.source_status`.
pub fn localized_source_status(code: Option<&str>) -> String {
    match code {
        Some("drive_not_mounted") => i18n("Source unavailable: drive not mounted").to_string(),
        Some("drive_mismatch") => i18n("Source unavailable: different drive").to_string(),
        Some("drive_empty") => i18n("Source unavailable: source empty").to_string(),
        _ => i18n("Source unavailable").to_string(),
    }
}

/// Availability reason shown next to «Back Up Now» in the GUI locale (FR-30).
pub fn localized_availability_reason(avail: &BackupAvailabilityDto) -> String {
    match avail.blocked_reason.as_deref() {
        Some(REASON_SOURCE_UNAVAILABLE) => localized_source_status(avail.source_status.as_deref()),
        Some(REASON_DESTINATION_UNREACHABLE) => i18n("Destination unreachable").to_string(),
        _ => avail.reason_text.clone().unwrap_or_default(),
    }
}

/// Error reason of a backup skipped by «Back Up All Now» with `reason == "error"`.
pub fn localized_skip_error_kind(kind: Option<&str>, daemon_text: &str) -> String {
    match kind {
        Some("secret_store_locked") => i18n("Password store is locked").to_string(),
        Some("restic_unavailable") => i18n("restic is unavailable or too old.").to_string(),
        Some("auth_failed") => i18n("Authentication at destination failed").to_string(),
        Some("wrong_password") => i18n("Stored password does not match").to_string(),
        Some("destination_unreachable") => i18n("Destination unreachable").to_string(),
        _ => daemon_text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn m3_review3_error_kind_codes_map_to_gui_texts() {
        assert_eq!(
            localized_error_kind(Some("wrong_password"), Some(12), false, Some("daemon text")),
            "Stored password does not match"
        );
        assert_eq!(
            localized_error_kind(Some("repo_missing"), Some(10), false, None),
            "Repository is missing"
        );
        assert_eq!(
            localized_error_kind(Some("locked"), Some(11), false, None),
            "Repository is locked"
        );
        assert_eq!(
            localized_error_kind(Some("unknown"), Some(42), false, Some("x")),
            "Unknown error (exit 42)"
        );
        assert_eq!(
            localized_error_kind(Some("unknown"), Some(1), false, Some("x")),
            "Backup command failed"
        );
        assert_eq!(
            localized_error_kind(Some("unknown"), None, true, Some("x")),
            "Backed up with warning"
        );
        assert_eq!(
            localized_error_kind(None, None, false, Some("legacy daemon text")),
            "legacy daemon text"
        );
    }

    fn progress(backup_id: Option<&str>) -> ActiveRunProgressDto {
        ActiveRunProgressDto {
            run_id: "r".to_string(),
            backup_id: backup_id.map(str::to_string),
            backup_name: "Z".to_string(),
            percent_done: 0.0,
            bytes_done: 0,
            total_bytes: 0,
            speed_bps: 0,
            seconds_remaining: 0,
        }
    }

    #[test]
    fn m3_review3_running_maintenance_uses_maintenance_wording() {
        assert_eq!(active_run_line(&progress(None)), "Maintaining Z");
        assert_eq!(active_run_line(&progress(Some("b"))), "Z: backing up · 0%");
        let now = time::macros::datetime!(2026-12-06 11:00:00 UTC);
        assert_eq!(
            maintenance_status_line(true, None, None, now),
            "Maintenance running"
        );
        assert_eq!(
            maintenance_status_line(false, Some("on_battery"), None, now),
            "Waiting: AC power"
        );
        assert_eq!(
            maintenance_status_line(false, Some("metered"), None, now),
            "Waiting: unmetered connection"
        );
        assert_eq!(maintenance_status_line(false, None, None, now), "");
    }

    #[test]
    fn m3_review3_source_and_skip_codes_map_to_gui_texts() {
        assert_eq!(
            localized_source_status(Some("drive_not_mounted")),
            "Source unavailable: drive not mounted"
        );
        assert_eq!(
            localized_source_status(Some("path_missing")),
            "Source unavailable"
        );
        assert_eq!(
            localized_skip_error_kind(Some("secret_store_locked"), "Passwortspeicher gesperrt"),
            "Password store is locked"
        );
        assert_eq!(
            localized_skip_error_kind(None, "daemon fallback"),
            "daemon fallback"
        );
    }
}
