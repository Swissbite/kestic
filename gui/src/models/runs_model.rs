// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! CXX-Qt `RunsModel` implementation (`QAbstractListModel` for recent backup/maintenance runs).

use super::{SyncPlan, plan_model_sync, qobject};
use crate::state_cache::{
    daemon_now, format_bytes, format_day_group, format_duration_ms, format_run_kind_name,
    format_run_status, format_time_short, shared_state,
};
use cxx_qt::CxxQtType;
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use kestic_core::model::{RunKind, RunStatus};
use std::pin::Pin;

#[derive(Clone, Default, PartialEq)]
struct RunRow {
    run_id: String,
    kind: String,
    backup_id: String,
    title: String,
    destination_id: String,
    destination_name: String,
    status: String,
    status_text: String,
    status_icon: String,
    status_color_kind: String,
    started_at_iso: String,
    started_at_ms: f64,
    day_group: String,
    time_short: String,
    duration_text: String,
    exit_code: i32,
    has_exit_code: bool,
    summary_text: String,
    data_added_text: String,
}

#[derive(Default)]
pub struct RunsModelRust {
    rows: Vec<RunRow>,
    filter_backup_id: String,
    issues_only: bool,
    query: String,
}

impl RunsModelRust {
    fn compute_rows(&self) -> Vec<RunRow> {
        let guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
        let Some(state) = &guard.state else {
            return Vec::new();
        };
        let now = daemon_now(state);

        let query_lower = self.query.trim().to_lowercase();
        let mut out = Vec::new();
        for run in &state.recent_runs {
            let backup_id_opt = run.backup_id();
            let backup_id_str = backup_id_opt
                .map(|id| id.as_str().to_string())
                .unwrap_or_default();

            if !self.filter_backup_id.is_empty() && backup_id_str != self.filter_backup_id {
                continue;
            }
            if self.issues_only
                && !matches!(
                    run.status,
                    RunStatus::Error | RunStatus::Warning | RunStatus::Interrupted
                )
            {
                continue;
            }

            let backup_name = backup_id_opt.and_then(|bid| {
                state
                    .backups
                    .iter()
                    .find(|b| &b.backup.id == bid)
                    .map(|b| b.backup.name.as_str())
            });
            let deleted_destination =
                crate::state_cache::tr(&kestic_i18n::messages::deleted_destination_message());
            let dest_name = state
                .destinations
                .iter()
                .find(|d| d.destination.id == run.destination_id)
                .map_or(deleted_destination.as_str(), |d| {
                    d.destination.name.as_str()
                });

            let title = format_run_kind_name(run, backup_name, dest_name);
            let summary_text = run.summary_message.clone().unwrap_or_default();

            if !query_lower.is_empty()
                && !title.to_lowercase().contains(&query_lower)
                && !dest_name.to_lowercase().contains(&query_lower)
                && !summary_text.to_lowercase().contains(&query_lower)
            {
                continue;
            }

            let kind = match run.kind {
                RunKind::Backup { .. } => "backup",
                RunKind::Maintenance => "maintenance",
                RunKind::Restore { .. } => "restore",
                RunKind::Meta => "meta",
            }
            .to_string();
            let status = match run.status {
                RunStatus::Pending => "pending",
                RunStatus::Running => "running",
                RunStatus::Ok => "ok",
                RunStatus::Warning => "warning",
                RunStatus::Error => "error",
                RunStatus::Cancelled => "cancelled",
                RunStatus::Interrupted => "interrupted",
            }
            .to_string();

            let (status_text, status_icon, status_color_kind) =
                format_run_status(&run.kind, run.status);
            let duration_text = run
                .duration_ms
                .map(format_duration_ms)
                .unwrap_or_else(|| status_text.clone());
            let data_added_text = run
                .stats
                .as_ref()
                .map(|s| format_bytes(s.data_added))
                .unwrap_or_else(|| String::from("0 B"));

            out.push(RunRow {
                run_id: run.id.as_str().to_string(),
                kind,
                backup_id: backup_id_str,
                title,
                destination_id: run.destination_id.as_str().to_string(),
                destination_name: dest_name.to_string(),
                status,
                status_text,
                status_icon: status_icon.to_string(),
                status_color_kind: status_color_kind.to_string(),
                started_at_iso: run
                    .started_at
                    .format(&time::format_description::well_known::Rfc3339)
                    .unwrap_or_default(),
                started_at_ms: (run.started_at.unix_timestamp() as f64) * 1000.0,
                day_group: format_day_group(run.started_at, now),
                time_short: format_time_short(run.started_at),
                duration_text,
                exit_code: run.exit_code.unwrap_or(0),
                has_exit_code: run.exit_code.is_some(),
                summary_text,
                data_added_text,
            });
        }
        out
    }
}

impl qobject::RunsModel {
    pub fn sync(mut self: Pin<&mut Self>) {
        let new_rows = self.as_ref().compute_rows();
        match plan_model_sync(&self.rows, new_rows, |a, b| {
            a.run_id == b.run_id && a.day_group == b.day_group
        }) {
            SyncPlan::NoChange => {}
            SyncPlan::InPlace {
                first,
                last,
                new_rows,
            } => {
                self.as_mut().rust_mut().rows = new_rows;
                let parent = QModelIndex::default();
                let top = self.as_ref().index_runs(first, 0, &parent);
                let bottom = self.as_ref().index_runs(last, 0, &parent);
                self.as_mut().data_changed_runs(&top, &bottom);
            }
            SyncPlan::Reset(new_rows) => {
                self.as_mut().begin_reset_runs();
                self.as_mut().rust_mut().rows = new_rows;
                self.as_mut().end_reset_runs();
            }
        }
    }

    pub fn set_filter(
        mut self: Pin<&mut Self>,
        backup_id: &QString,
        issues_only: bool,
        query: &QString,
    ) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.filter_backup_id = backup_id.to_string();
            rust.issues_only = issues_only;
            rust.query = query.to_string();
        }
        self.sync();
    }

    pub fn row_count(&self, _parent: &QModelIndex) -> i32 {
        self.rows.len() as i32
    }

    pub fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        for (role, name) in [
            (qobject::RunRoles::RunId, "runId"),
            (qobject::RunRoles::Kind, "kind"),
            (qobject::RunRoles::BackupId, "backupId"),
            (qobject::RunRoles::Title, "title"),
            (qobject::RunRoles::DestinationId, "destinationId"),
            (qobject::RunRoles::DestinationName, "destinationName"),
            (qobject::RunRoles::Status, "status"),
            (qobject::RunRoles::StatusText, "statusText"),
            (qobject::RunRoles::StatusIcon, "statusIcon"),
            (qobject::RunRoles::StatusColorKind, "statusColorKind"),
            (qobject::RunRoles::StartedAtIso, "startedAtIso"),
            (qobject::RunRoles::StartedAtMs, "startedAtMs"),
            (qobject::RunRoles::DayGroup, "dayGroup"),
            (qobject::RunRoles::TimeShort, "timeShort"),
            (qobject::RunRoles::DurationText, "durationText"),
            (qobject::RunRoles::ExitCode, "exitCode"),
            (qobject::RunRoles::HasExitCode, "hasExitCode"),
            (qobject::RunRoles::SummaryText, "summaryText"),
            (qobject::RunRoles::DataAddedText, "dataAddedText"),
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
        match (qobject::RunRoles { repr: role }) {
            qobject::RunRoles::RunId => QVariant::from(&QString::from(&row.run_id)),
            qobject::RunRoles::Kind => QVariant::from(&QString::from(&row.kind)),
            qobject::RunRoles::BackupId => QVariant::from(&QString::from(&row.backup_id)),
            qobject::RunRoles::Title => QVariant::from(&QString::from(&row.title)),
            qobject::RunRoles::DestinationId => QVariant::from(&QString::from(&row.destination_id)),
            qobject::RunRoles::DestinationName => {
                QVariant::from(&QString::from(&row.destination_name))
            }
            qobject::RunRoles::Status => QVariant::from(&QString::from(&row.status)),
            qobject::RunRoles::StatusText => QVariant::from(&QString::from(&row.status_text)),
            qobject::RunRoles::StatusIcon => QVariant::from(&QString::from(&row.status_icon)),
            qobject::RunRoles::StatusColorKind => {
                QVariant::from(&QString::from(&row.status_color_kind))
            }
            qobject::RunRoles::StartedAtIso => QVariant::from(&QString::from(&row.started_at_iso)),
            qobject::RunRoles::StartedAtMs => QVariant::from(&row.started_at_ms),
            qobject::RunRoles::DayGroup => QVariant::from(&QString::from(&row.day_group)),
            qobject::RunRoles::TimeShort => QVariant::from(&QString::from(&row.time_short)),
            qobject::RunRoles::DurationText => QVariant::from(&QString::from(&row.duration_text)),
            qobject::RunRoles::ExitCode => QVariant::from(&row.exit_code),
            qobject::RunRoles::HasExitCode => QVariant::from(&row.has_exit_code),
            qobject::RunRoles::SummaryText => QVariant::from(&QString::from(&row.summary_text)),
            qobject::RunRoles::DataAddedText => {
                QVariant::from(&QString::from(&row.data_added_text))
            }
            _ => QVariant::default(),
        }
    }
}
