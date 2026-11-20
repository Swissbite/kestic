// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! CXX-Qt `RunLogsModel` implementation (`QAbstractListModel` for redacted run logs).

use super::{SyncPlan, plan_model_sync, qobject};
use crate::state_cache::shared_state;
use cxx_qt::CxxQtType;
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use std::pin::Pin;

#[derive(Clone, Default, PartialEq)]
struct LogRow {
    line_text: String,
    level: String,
}

#[derive(Default)]
pub struct RunLogsModelRust {
    rows: Vec<LogRow>,
    level_filter: String,
    query: String,
}

impl RunLogsModelRust {
    fn compute_rows(&self) -> Vec<LogRow> {
        let guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
        let Some(detail) = &guard.run_detail else {
            return Vec::new();
        };
        let query_lower = self.query.trim().to_lowercase();
        let mut out = Vec::new();
        for line in &detail.log_lines {
            let level = if line.contains("\"message_type\":\"error\"") || line.contains("error") {
                "error"
            } else if line.contains("warning") {
                "warning"
            } else {
                "info"
            };
            if self.level_filter == "warnings" && level != "warning" && level != "error" {
                continue;
            }
            if self.level_filter == "errors" && level != "error" {
                continue;
            }
            if !query_lower.is_empty() && !line.to_lowercase().contains(&query_lower) {
                continue;
            }
            out.push(LogRow {
                line_text: line.clone(),
                level: level.to_string(),
            });
        }
        out
    }
}

impl qobject::RunLogsModel {
    pub fn sync(mut self: Pin<&mut Self>) {
        let new_rows = self.as_ref().compute_rows();
        match plan_model_sync(&self.rows, new_rows, |a, b| a == b) {
            SyncPlan::NoChange => {}
            SyncPlan::InPlace {
                first,
                last,
                new_rows,
            } => {
                self.as_mut().rust_mut().rows = new_rows;
                let parent = QModelIndex::default();
                let top = self.as_ref().index_logs(first, 0, &parent);
                let bottom = self.as_ref().index_logs(last, 0, &parent);
                self.as_mut().data_changed_logs(&top, &bottom);
            }
            SyncPlan::Reset(new_rows) => {
                self.as_mut().begin_reset_logs();
                self.as_mut().rust_mut().rows = new_rows;
                self.as_mut().end_reset_logs();
            }
        }
    }

    pub fn set_filter(mut self: Pin<&mut Self>, level_filter: &QString, query: &QString) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.level_filter = level_filter.to_string();
            rust.query = query.to_string();
        }
        self.sync();
    }

    pub fn row_count(&self, _parent: &QModelIndex) -> i32 {
        self.rows.len() as i32
    }

    pub fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        roles.insert(
            qobject::RunLogRoles::LineText.repr,
            QByteArray::from("lineText"),
        );
        roles.insert(qobject::RunLogRoles::Level.repr, QByteArray::from("level"));
        roles
    }

    pub fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let Some(row) = usize::try_from(index.row())
            .ok()
            .and_then(|i| self.rows.get(i))
        else {
            return QVariant::default();
        };
        match (qobject::RunLogRoles { repr: role }) {
            qobject::RunLogRoles::LineText => QVariant::from(&QString::from(&row.line_text)),
            qobject::RunLogRoles::Level => QVariant::from(&QString::from(&row.level)),
            _ => QVariant::default(),
        }
    }
}
