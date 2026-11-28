// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! CXX-Qt `RunsModel` implementation (`QAbstractListModel` for recent backup/maintenance runs).

use super::qobject;
use super::rows::{id_at, row_of};
use crate::run_texts::run_status;
use crate::state_cache::shared_state;
use crate::status_texts::format_run_kind_name;
use crate::summary_texts::format_duration_ms;
use crate::time_texts::{daemon_now, format_day_group, format_time_short};
use cxx_qt::CxxQtType;
use cxx_qt_lib::{QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use kestic_core::model::RunStatus;
use kestic_core::sync::LockUnpoisoned;
use std::pin::Pin;

model_row!(RunRow {
    run_id: String => "runId",
    title: String => "title",
    status_text: String => "statusText",
    status_icon: String => "statusIcon",
    status_color_kind: String => "statusColorKind",
    day_group: String => "dayGroup",
    time_short: String => "timeShort",
    duration_text: String => "durationText",
    summary_text: String => "summaryText",
});

#[derive(Default)]
pub struct RunsModelRust {
    rows: Vec<RunRow>,
    filter_backup_id: String,
    issues_only: bool,
    query: String,
}

impl RunsModelRust {
    fn compute_rows(&self) -> Vec<RunRow> {
        let guard = shared_state().lock_unpoisoned();
        let Some(state) = &guard.state else {
            return Vec::new();
        };
        let now = daemon_now(state);

        let query_lower = self.query.trim().to_lowercase();
        let mut out = Vec::new();
        for run in &state.recent_runs {
            let backup_id_opt = run.backup_id();
            let backup_id_str = backup_id_opt.map_or_default(|id| id.as_str().to_string());

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
                crate::i18n::tr(&kestic_i18n::messages::deleted_destination_message());
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

            let (status_text, status_icon, status_color_kind) = run_status(run);
            let duration_text = run
                .duration_ms
                .map_or_else(|| status_text.clone(), format_duration_ms);
            out.push(RunRow {
                run_id: run.id.as_str().to_string(),
                title,
                status_text,
                status_icon: status_icon.to_string(),
                status_color_kind: status_color_kind.to_string(),
                day_group: format_day_group(run.started_at, now),
                time_short: format_time_short(run.started_at),
                duration_text,
                summary_text,
            });
        }
        out
    }
}

list_model!(
    RunsModel,
    RunRow,
    same: |a, b| a.run_id == b.run_id && a.day_group == b.day_group,
    begin_reset: begin_reset_runs,
    end_reset: end_reset_runs,
    index: index_runs,
    changed: data_changed_runs,
);

impl qobject::RunsModel {
    /// Row of the run `run_id`, or -1 if unknown.
    pub fn row_of(&self, run_id: &QString) -> i32 {
        let run_id = run_id.to_string();
        row_of(&self.rows, |row| row.run_id == run_id)
    }

    /// Id of the run in `row`, empty if there is no such row.
    pub fn id_at(&self, row: i32) -> QString {
        id_at(&self.rows, row, |row| &row.run_id)
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
}
