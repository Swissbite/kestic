// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! CXX-Qt `SnapshotsModel` implementation (`QAbstractListModel` for backup snapshots).

use crate::state_cache::{format_bytes, i18n, i18np, shared_state};
use crate::time_texts::format_datetime_display;
use crate::ui_texts::GuiText;
use cxx_qt::CxxQtType;
use cxx_qt_lib::{QHash, QHashPair_i32_QByteArray, QModelIndex, QVariant};
use kestic_core::sync::LockUnpoisoned;
use time::OffsetDateTime;

model_row!(SnapshotRow {
    short_id: String => "shortId",
    time_display: String => "timeDisplay",
    paths_text: String => "pathsText",
    stats_text: String => "statsText",
    section_name: String => "sectionName",
} hidden {
    snapshot_id: String,
});

#[derive(Default)]
pub struct SnapshotsModelRust {
    rows: Vec<SnapshotRow>,
}

impl SnapshotsModelRust {
    fn push_snapshot(
        out: &mut Vec<SnapshotRow>,
        s: &kestic_daemon::dbus_api::SnapshotSummaryDto,
        section_name: String,
    ) {
        let time_display =
            OffsetDateTime::parse(&s.time, &time::format_description::well_known::Rfc3339)
                .map_or_else(|_| s.time.clone(), format_datetime_display);
        let stats_text = match (s.total_files_processed, s.total_bytes_processed) {
            (Some(files), Some(bytes)) => i18np(
                "%1 file · %2",
                "%1 files · %2",
                files,
                &[&format_bytes(bytes)],
            ),
            (None, Some(bytes)) => format_bytes(bytes),
            _ => s.hostname.clone(),
        };
        out.push(SnapshotRow {
            snapshot_id: s.id.clone(),
            short_id: s.short_id.clone(),
            time_display,
            paths_text: s.paths.join(", "),
            stats_text,
            section_name,
        });
    }

    fn compute_rows(&self) -> Vec<SnapshotRow> {
        let guard = shared_state().lock_unpoisoned();
        let Some(snaps) = &guard.snapshots else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let current_section =
            i18n("Current destination (%1)").subs(&[&snaps.current_destination_name]);
        for s in &snaps.current_snapshots {
            Self::push_snapshot(&mut out, s, current_section.clone());
        }
        for prev in &snaps.previous_destinations {
            let section = GuiText::EarlierSnapshotsOn.with_args(&[&prev.destination_name]);
            for s in &prev.snapshots {
                Self::push_snapshot(&mut out, s, section.clone());
            }
        }
        out
    }
}

list_model!(
    SnapshotsModel,
    SnapshotRow,
    same: |a, b| a.snapshot_id == b.snapshot_id && a.section_name == b.section_name,
    begin_reset: begin_reset_snapshots,
    end_reset: end_reset_snapshots,
    index: index_snapshots,
    changed: data_changed_snapshots,
);
