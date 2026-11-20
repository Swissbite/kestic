// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! CXX-Qt `DestinationsModel` implementation (`QAbstractListModel` for backup destinations).

use super::{SyncPlan, plan_model_sync, qobject};
use crate::overall_text::maintenance_status_line;
use crate::state_cache::{
    daemon_now, format_datetime_display, format_destination_status, format_maintenance_entry,
    format_slot_timestamp, i18n, shared_state,
};
use cxx_qt::CxxQtType;
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use kestic_core::model::DestinationKind;
use kestic_daemon::dbus_api::{BackupSummaryDto, DestinationSummaryDto};
use std::pin::Pin;
use time::OffsetDateTime;

#[derive(Clone, Default, PartialEq)]
struct DestinationRow {
    destination_id: String,
    name: String,
    kind_name: String,
    kind_label: String,
    location: String,
    username: String,
    backup_count: i32,
    status_text: String,
    status_icon: String,
    reachable: bool,
    password_rejected: bool,
    has_stale_lock: bool,
    has_problem: bool,
    detail_text: String,
    pending_forget_count: i32,
    maintenance_json: String,
    prune_summary: String,
    check_summary: String,
    next_prune_at_text: String,
    next_check_at_text: String,
    last_prune_at_text: String,
    last_check_at_text: String,
    maintenance_running: bool,
    stale_lock_count: i32,
    maintenance_status_text: String,
}

#[derive(Default)]
pub struct DestinationsModelRust {
    rows: Vec<DestinationRow>,
}

fn build_destination_row(
    d: &DestinationSummaryDto,
    backups: &[BackupSummaryDto],
    now: OffsetDateTime,
) -> DestinationRow {
    let (kind_name, kind_label, location, username) = match &d.destination.kind {
        DestinationKind::Local { path } => (
            "local".to_string(),
            i18n("Local Folder").to_string(),
            path.to_string_lossy().into_owned(),
            String::new(),
        ),
        DestinationKind::Rest { url, username } => (
            "rest".to_string(),
            i18n("REST Server").to_string(),
            url.clone(),
            username.clone().unwrap_or_default(),
        ),
    };
    let (status_text, status_icon) = format_destination_status(d, now);
    let pending_forget_count = backups
        .iter()
        .filter(|b| b.backup.destination_id == d.destination.id && b.pending_forget)
        .count();
    let maintenance_json =
        serde_json::to_string(&d.destination.maintenance).unwrap_or_else(|_| "{}".to_string());
    let last_prune_at_text = d
        .last_prune_at_unix
        .and_then(|ts| OffsetDateTime::from_unix_timestamp(ts).ok())
        .map(format_datetime_display)
        .unwrap_or_default();
    let last_check_at_text = d
        .last_check_at_unix
        .and_then(|ts| OffsetDateTime::from_unix_timestamp(ts).ok())
        .map(format_datetime_display)
        .unwrap_or_default();
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
        reachable: d.availability.reachable,
        password_rejected: d.password_rejected,
        has_stale_lock: d.has_stale_lock,
        has_problem: d.has_problem,
        detail_text: d.availability.detail.clone().unwrap_or_default(),
        pending_forget_count: i32::try_from(pending_forget_count).unwrap_or(i32::MAX),
        maintenance_json,
        prune_summary: format_maintenance_entry(&d.destination.maintenance.prune),
        check_summary: format_maintenance_entry(&d.destination.maintenance.check),
        next_prune_at_text: format_slot_timestamp(d.next_prune_at_unix, now),
        next_check_at_text: format_slot_timestamp(d.next_check_at_unix, now),
        last_prune_at_text,
        last_check_at_text,
        maintenance_running: d.maintenance_running,
        stale_lock_count: i32::try_from(d.stale_lock_count).unwrap_or(i32::MAX),
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
        let guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
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

impl qobject::DestinationsModel {
    pub fn sync(mut self: Pin<&mut Self>) {
        let new_rows = self.as_ref().compute_rows();
        match plan_model_sync(&self.rows, new_rows, |a, b| {
            a.destination_id == b.destination_id
        }) {
            SyncPlan::NoChange => {}
            SyncPlan::InPlace {
                first,
                last,
                new_rows,
            } => {
                self.as_mut().rust_mut().rows = new_rows;
                let parent = QModelIndex::default();
                let top = self.as_ref().index_destinations(first, 0, &parent);
                let bottom = self.as_ref().index_destinations(last, 0, &parent);
                self.as_mut().data_changed_destinations(&top, &bottom);
            }
            SyncPlan::Reset(new_rows) => {
                self.as_mut().begin_reset_destinations();
                self.as_mut().rust_mut().rows = new_rows;
                self.as_mut().end_reset_destinations();
            }
        }
    }

    pub fn row_count(&self, _parent: &QModelIndex) -> i32 {
        self.rows.len() as i32
    }

    pub fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        for (role, name) in [
            (qobject::DestinationRoles::DestinationId, "destinationId"),
            (qobject::DestinationRoles::Name, "name"),
            (qobject::DestinationRoles::KindName, "kindName"),
            (qobject::DestinationRoles::KindLabel, "kindLabel"),
            (qobject::DestinationRoles::Location, "location"),
            (qobject::DestinationRoles::Username, "username"),
            (qobject::DestinationRoles::BackupCount, "backupCount"),
            (qobject::DestinationRoles::StatusText, "statusText"),
            (qobject::DestinationRoles::StatusIcon, "statusIcon"),
            (qobject::DestinationRoles::Reachable, "reachable"),
            (
                qobject::DestinationRoles::PasswordRejected,
                "passwordRejected",
            ),
            (qobject::DestinationRoles::HasStaleLock, "hasStaleLock"),
            (qobject::DestinationRoles::HasProblem, "hasProblem"),
            (qobject::DestinationRoles::DetailText, "detailText"),
            (
                qobject::DestinationRoles::PendingForgetCount,
                "pendingForgetCount",
            ),
            (
                qobject::DestinationRoles::MaintenanceJson,
                "maintenanceJson",
            ),
            (qobject::DestinationRoles::PruneSummary, "pruneSummary"),
            (qobject::DestinationRoles::CheckSummary, "checkSummary"),
            (
                qobject::DestinationRoles::NextPruneAtText,
                "nextPruneAtText",
            ),
            (
                qobject::DestinationRoles::NextCheckAtText,
                "nextCheckAtText",
            ),
            (
                qobject::DestinationRoles::LastPruneAtText,
                "lastPruneAtText",
            ),
            (
                qobject::DestinationRoles::LastCheckAtText,
                "lastCheckAtText",
            ),
            (
                qobject::DestinationRoles::MaintenanceRunning,
                "maintenanceRunning",
            ),
            (qobject::DestinationRoles::StaleLockCount, "staleLockCount"),
            (
                qobject::DestinationRoles::MaintenanceStatusText,
                "maintenanceStatusText",
            ),
        ] {
            roles.insert(role.repr, QByteArray::from(name));
        }
        roles
    }

    pub fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let Some(row) = usize::try_from(index.row())
            .ok()
            .and_then(|i| self.rows.get(i))
        else {
            return QVariant::default();
        };
        match (qobject::DestinationRoles { repr: role }) {
            qobject::DestinationRoles::DestinationId => {
                QVariant::from(&QString::from(&row.destination_id))
            }
            qobject::DestinationRoles::Name => QVariant::from(&QString::from(&row.name)),
            qobject::DestinationRoles::KindName => QVariant::from(&QString::from(&row.kind_name)),
            qobject::DestinationRoles::KindLabel => QVariant::from(&QString::from(&row.kind_label)),
            qobject::DestinationRoles::Location => QVariant::from(&QString::from(&row.location)),
            qobject::DestinationRoles::Username => QVariant::from(&QString::from(&row.username)),
            qobject::DestinationRoles::BackupCount => QVariant::from(&row.backup_count),
            qobject::DestinationRoles::StatusText => {
                QVariant::from(&QString::from(&row.status_text))
            }
            qobject::DestinationRoles::StatusIcon => {
                QVariant::from(&QString::from(&row.status_icon))
            }
            qobject::DestinationRoles::Reachable => QVariant::from(&row.reachable),
            qobject::DestinationRoles::PasswordRejected => QVariant::from(&row.password_rejected),
            qobject::DestinationRoles::HasStaleLock => QVariant::from(&row.has_stale_lock),
            qobject::DestinationRoles::HasProblem => QVariant::from(&row.has_problem),
            qobject::DestinationRoles::DetailText => {
                QVariant::from(&QString::from(&row.detail_text))
            }
            qobject::DestinationRoles::PendingForgetCount => {
                QVariant::from(&row.pending_forget_count)
            }
            qobject::DestinationRoles::MaintenanceJson => {
                QVariant::from(&QString::from(&row.maintenance_json))
            }
            qobject::DestinationRoles::PruneSummary => {
                QVariant::from(&QString::from(&row.prune_summary))
            }
            qobject::DestinationRoles::CheckSummary => {
                QVariant::from(&QString::from(&row.check_summary))
            }
            qobject::DestinationRoles::NextPruneAtText => {
                QVariant::from(&QString::from(&row.next_prune_at_text))
            }
            qobject::DestinationRoles::NextCheckAtText => {
                QVariant::from(&QString::from(&row.next_check_at_text))
            }
            qobject::DestinationRoles::LastPruneAtText => {
                QVariant::from(&QString::from(&row.last_prune_at_text))
            }
            qobject::DestinationRoles::LastCheckAtText => {
                QVariant::from(&QString::from(&row.last_check_at_text))
            }
            qobject::DestinationRoles::MaintenanceRunning => {
                QVariant::from(&row.maintenance_running)
            }
            qobject::DestinationRoles::StaleLockCount => QVariant::from(&row.stale_lock_count),
            qobject::DestinationRoles::MaintenanceStatusText => {
                QVariant::from(&QString::from(&row.maintenance_status_text))
            }
            _ => QVariant::default(),
        }
    }
}
