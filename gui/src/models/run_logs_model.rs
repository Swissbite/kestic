// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! CXX-Qt `RunLogsModel` implementation (`QAbstractListModel` for redacted run logs).

use super::qobject;
use crate::state_cache::shared_state;
use cxx_qt::CxxQtType;
use cxx_qt_lib::{QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use kestic_core::sync::LockUnpoisoned;
use std::pin::Pin;

model_row!(LogRow {
    line_text: String => "lineText",
    level: String => "level",
});

#[derive(Default)]
pub struct RunLogsModelRust {
    rows: Vec<LogRow>,
    level_filter: String,
    query: String,
}

impl RunLogsModelRust {
    fn compute_rows(&self) -> Vec<LogRow> {
        let guard = shared_state().lock_unpoisoned();
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

list_model!(
    RunLogsModel,
    LogRow,
    same: |a, b| a == b,
    begin_reset: begin_reset_logs,
    end_reset: end_reset_logs,
    index: index_logs,
    changed: data_changed_logs,
);

impl qobject::RunLogsModel {
    pub fn set_filter(mut self: Pin<&mut Self>, level_filter: &QString, query: &QString) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.level_filter = level_filter.to_string();
            rust.query = query.to_string();
        }
        self.sync();
    }
}
