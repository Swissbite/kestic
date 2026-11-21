// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Overview headline and subtitle and the other code-based status texts, built in the GUI
//! locale from the language-independent DTO codes via the shared mapping in
//! `kestic_i18n::messages`; the DTOs carry no daemon-language texts.

use crate::state_cache::{i18np, tr};
use kestic_daemon::dbus_api::{BackupAvailabilityDto, DaemonStateDto, blocking_message};
use kestic_i18n::codes::PendingReasonCode;
use kestic_i18n::messages::{maintenance_running_message, overall_headline as headline_message};

/// Headline for the status card (priority order of the overall status, `kestic_i18n::messages::overall_headline`).
pub fn overall_headline(state: &DaemonStateDto) -> String {
    let o = &state.overall;
    let active = o.active_run_info();
    tr(&headline_message(
        &o.counts(state.backups.len()),
        active.as_ref(),
        blocking_message(state.blocking.as_ref(), &state.restic_status),
    ))
}

/// Subtitle while a run is active next to failed/overdue backups.
pub fn overall_subtitle(state: &DaemonStateDto) -> String {
    let o = &state.overall;
    let active = o.active_run_info();
    kestic_i18n::messages::overall_subtitle(&o.counts(state.backups.len()), active.as_ref())
        .map(|m| tr(&m))
        .unwrap_or_default()
}

/// Number of backups «Back Up All Now» can start right now (source and destination
/// available, no permanent error, run conditions met) and, when that is not every
/// backup, the hint shown before the click.
pub fn back_up_all_availability(state: &DaemonStateDto) -> (usize, String) {
    let total = state.backups.len();
    let startable = state
        .backups
        .iter()
        .filter(|b| b.availability.available && !b.has_error && !b.has_unmet_run_condition)
        .count();
    let hint = if total > 0 && startable < total {
        i18np(
            "Only %2 of %1 backup can start now.",
            "Only %2 of %1 backups can start now.",
            total,
            &[&startable.to_string()],
        )
    } else {
        String::new()
    };
    (startable, hint)
}

/// Maintenance state line of a destination (detail page): running, or why due maintenance
/// waits (`maintenance_pending_reason`), in the GUI locale.
pub fn maintenance_status_line(
    running: bool,
    pending: Option<PendingReasonCode>,
    retry_at_unix: Option<i64>,
    now: time::OffsetDateTime,
) -> String {
    if running {
        return tr(&maintenance_running_message());
    }
    pending
        .map(|code| crate::state_cache::pending_reason_line(code, retry_at_unix, now))
        .unwrap_or_default()
}

/// Blocking-state text in the GUI locale from `BlockingStateDto.kind` and the
/// `ResticStatusDto` parameters.
pub fn localized_blocking_message(state: &DaemonStateDto) -> String {
    blocking_message(state.blocking.as_ref(), &state.restic_status)
        .map(|m| tr(&m))
        .unwrap_or_default()
}

/// Availability reason shown next to «Back Up Now» in the GUI locale.
pub fn localized_availability_reason(avail: &BackupAvailabilityDto) -> String {
    avail
        .blocked_reason
        .map(|reason| tr(&reason.message(avail.source_status)))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_maintenance_uses_maintenance_wording() {
        let now = time::macros::datetime!(2026-12-06 11:00:00 UTC);
        assert_eq!(
            maintenance_status_line(true, None, None, now),
            "Maintenance running"
        );
        assert_eq!(
            maintenance_status_line(false, Some(PendingReasonCode::OnBattery), None, now),
            "Waiting: AC power"
        );
        assert_eq!(
            maintenance_status_line(false, Some(PendingReasonCode::Metered), None, now),
            "Waiting: unmetered connection"
        );
        assert_eq!(maintenance_status_line(false, None, None, now), "");
    }
}
