// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! CXX-Qt `DestinationsModel` implementation (`QAbstractListModel` for backup destinations).

use super::qobject;
use super::rows::{id_at, row_map, row_of};
use crate::overall_text::maintenance_status_line;
use crate::state_cache::{
    format_destination_status, format_maintenance_entry, format_slot_timestamp, i18n, json_or,
    shared_state,
};
use crate::time_texts::{daemon_now, format_datetime_display};
use crate::ui_texts::GuiText;
use cxx_qt::CxxQtType;
use cxx_qt_lib::{QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use kestic_core::model::DestinationKind;
use kestic_core::sync::LockUnpoisoned;
use kestic_daemon::dbus_api::{BackupSummaryDto, DestinationSummaryDto};
use time::OffsetDateTime;

model_row!(DestinationRow {
    destination_id: String => "destinationId",
    name: String => "name",
    kind_name: String => "kindName",
    kind_label: String => "kindLabel",
    location: String => "location",
    username: String => "username",
    backup_count: i32 => "backupCount",
    status_text: String => "statusText",
    status_icon: String => "statusIcon",
    password_rejected: bool => "passwordRejected",
    has_stale_lock: bool => "hasStaleLock",
    has_problem: bool => "hasProblem",
    pending_forget_count: i32 => "pendingForgetCount",
    maintenance_json: String => "maintenanceJson",
    prune_summary: String => "pruneSummary",
    check_summary: String => "checkSummary",
    next_prune_at_text: String => "nextPruneAtText",
    next_check_at_text: String => "nextCheckAtText",
    last_prune_at_text: String => "lastPruneAtText",
    last_check_at_text: String => "lastCheckAtText",
    maintenance_status_text: String => "maintenanceStatusText",
});

#[derive(Default)]
pub struct DestinationsModelRust {
    rows: Vec<DestinationRow>,
}

/// Where the repository of a destination lies, as the user entered it: the folder or the URL.
pub(crate) fn location_text(kind: &DestinationKind) -> String {
    match kind {
        DestinationKind::Local { path } => path.to_string_lossy().into_owned(),
        DestinationKind::Rest { url, .. } => url.clone(),
    }
}

fn build_destination_row(
    d: &DestinationSummaryDto,
    backups: &[BackupSummaryDto],
    now: OffsetDateTime,
) -> DestinationRow {
    let (kind_name, kind_label, username) = match &d.destination.kind {
        DestinationKind::Local { .. } => (
            "local".to_string(),
            i18n("Local Folder").to_string(),
            String::new(),
        ),
        DestinationKind::Rest { username, .. } => (
            "rest".to_string(),
            GuiText::RestServer.text(),
            username.clone().unwrap_or_default(),
        ),
    };
    let location = location_text(&d.destination.kind);
    let (status_text, status_icon) = format_destination_status(d, now);
    let pending_forget_count = backups
        .iter()
        .filter(|b| b.backup.destination_id == d.destination.id && b.pending_forget)
        .count();
    let maintenance_json = json_or(&d.destination.maintenance, "{}");
    let last_prune_at_text = d
        .last_prune_at_unix
        .and_then(|ts| OffsetDateTime::from_unix_timestamp(ts).ok())
        .map_or_default(format_datetime_display);
    let last_check_at_text = d
        .last_check_at_unix
        .and_then(|ts| OffsetDateTime::from_unix_timestamp(ts).ok())
        .map_or_default(format_datetime_display);
    DestinationRow {
        destination_id: d.destination.id.as_str().to_string(),
        name: d.destination.name.clone(),
        kind_name,
        kind_label,
        location,
        username,
        backup_count: i32::try_from(d.backup_count).unwrap_or(i32::MAX),
        status_text,
        status_icon,
        password_rejected: d.password_rejected,
        has_stale_lock: d.has_stale_lock,
        has_problem: d.has_problem,
        pending_forget_count: i32::try_from(pending_forget_count).unwrap_or(i32::MAX),
        maintenance_json,
        prune_summary: format_maintenance_entry(&d.destination.maintenance.prune),
        check_summary: format_maintenance_entry(&d.destination.maintenance.check),
        next_prune_at_text: format_slot_timestamp(d.next_prune_at_unix, now),
        next_check_at_text: format_slot_timestamp(d.next_check_at_unix, now),
        last_prune_at_text,
        last_check_at_text,
        maintenance_status_text: maintenance_status_line(
            d.maintenance_running,
            d.maintenance_pending_reason,
            d.maintenance_retry_at_unix,
            now,
        ),
    }
}

impl DestinationsModelRust {
    fn compute_rows(&self) -> Vec<DestinationRow> {
        let guard = shared_state().lock_unpoisoned();
        let Some(state) = &guard.state else {
            return Vec::new();
        };
        let now = daemon_now(state);
        state
            .destinations
            .iter()
            .map(|d| build_destination_row(d, &state.backups, now))
            .collect()
    }
}

list_model!(
    DestinationsModel,
    DestinationRow,
    same: |a, b| a.destination_id == b.destination_id,
    begin_reset: begin_reset_destinations,
    end_reset: end_reset_destinations,
    index: index_destinations,
    changed: data_changed_destinations,
);

impl qobject::DestinationsModel {
    /// Row of the destination `destination_id`, or -1 if unknown.
    pub fn row_of(&self, destination_id: &QString) -> i32 {
        let destination_id = destination_id.to_string();
        row_of(&self.rows, |row| row.destination_id == destination_id)
    }

    /// Id of the destination in `row`, empty if there is no such row.
    pub fn id_at(&self, row: i32) -> QString {
        id_at(&self.rows, row, |row| &row.destination_id)
    }

    /// All roles of the destination `destination_id` as a QML object, or an invalid value if
    /// unknown.
    pub fn row_map(&self, destination_id: &QString) -> QVariant {
        let destination_id = destination_id.to_string();
        row_map(
            self.rows
                .iter()
                .find(|row| row.destination_id == destination_id),
        )
    }
}
