// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! CXX-Qt `SnapshotsModel` implementation (`QAbstractListModel` for backup snapshots).

use super::{SyncPlan, plan_model_sync, qobject};
use crate::state_cache::{format_bytes, format_datetime_display, i18n, shared_state};
use cxx_qt::CxxQtType;
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use std::pin::Pin;
use time::OffsetDateTime;

#[derive(Clone, Default, PartialEq)]
struct SnapshotRow {
    snapshot_id: String,
    short_id: String,
    time_iso: String,
    time_display: String,
    hostname: String,
    paths_text: String,
    tags_text: String,
    stats_text: String,
    section_name: String,
    is_previous: bool,
}

#[derive(Default)]
pub struct SnapshotsModelRust {
    rows: Vec<SnapshotRow>,
}

impl SnapshotsModelRust {
    fn push_snapshot(
        out: &mut Vec<SnapshotRow>,
        s: &kestic_daemon::dbus_api::SnapshotSummaryDto,
        section_name: String,
        is_previous: bool,
    ) {
        let time_display =
            OffsetDateTime::parse(&s.time, &time::format_description::well_known::Rfc3339)
                .map(format_datetime_display)
                .unwrap_or_else(|_| s.time.clone());
        let stats_text = match (s.total_files_processed, s.total_bytes_processed) {
            (Some(files), Some(bytes)) => {
                i18n("%1 files · %2").subs(&[&files.to_string(), &format_bytes(bytes)])
            }
            (None, Some(bytes)) => format_bytes(bytes),
            _ => s.hostname.clone(),
        };
        out.push(SnapshotRow {
            snapshot_id: s.id.clone(),
            short_id: s.short_id.clone(),
            time_iso: s.time.clone(),
            time_display,
            hostname: s.hostname.clone(),
            paths_text: s.paths.join(", "),
            tags_text: s.tags.join(", "),
            stats_text,
            section_name,
            is_previous,
        });
    }

    fn compute_rows(&self) -> Vec<SnapshotRow> {
        let guard = shared_state().lock().unwrap_or_else(|p| p.into_inner());
        let Some(snaps) = &guard.snapshots else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let current_section =
            i18n("Current destination (%1)").subs(&[&snaps.current_destination_name]);
        for s in snaps.current_snapshots.iter().rev() {
            Self::push_snapshot(&mut out, s, current_section.clone(), false);
        }
        for prev in &snaps.previous_destinations {
            let section = i18n("Earlier snapshots on '%1'").subs(&[&prev.destination_name]);
            for s in prev.snapshots.iter().rev() {
                Self::push_snapshot(&mut out, s, section.clone(), true);
            }
        }
        out
    }
}

impl qobject::SnapshotsModel {
    pub fn sync(mut self: Pin<&mut Self>) {
        let new_rows = self.as_ref().compute_rows();
        match plan_model_sync(&self.rows, new_rows, |a, b| {
            a.snapshot_id == b.snapshot_id && a.section_name == b.section_name
        }) {
            SyncPlan::NoChange => {}
            SyncPlan::InPlace {
                first,
                last,
                new_rows,
            } => {
                self.as_mut().rust_mut().rows = new_rows;
                let parent = QModelIndex::default();
                let top = self.as_ref().index_snapshots(first, 0, &parent);
                let bottom = self.as_ref().index_snapshots(last, 0, &parent);
                self.as_mut().data_changed_snapshots(&top, &bottom);
            }
            SyncPlan::Reset(new_rows) => {
                self.as_mut().begin_reset_snapshots();
                self.as_mut().rust_mut().rows = new_rows;
                self.as_mut().end_reset_snapshots();
            }
        }
    }

    pub fn row_count(&self, _parent: &QModelIndex) -> i32 {
        self.rows.len() as i32
    }

    pub fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        for (role, name) in [
            (qobject::SnapshotRoles::SnapshotId, "snapshotId"),
            (qobject::SnapshotRoles::ShortId, "shortId"),
            (qobject::SnapshotRoles::TimeIso, "timeIso"),
            (qobject::SnapshotRoles::TimeDisplay, "timeDisplay"),
            (qobject::SnapshotRoles::Hostname, "hostname"),
            (qobject::SnapshotRoles::PathsText, "pathsText"),
            (qobject::SnapshotRoles::TagsText, "tagsText"),
            (qobject::SnapshotRoles::StatsText, "statsText"),
            (qobject::SnapshotRoles::SectionName, "sectionName"),
            (qobject::SnapshotRoles::IsPrevious, "isPrevious"),
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
        match (qobject::SnapshotRoles { repr: role }) {
            qobject::SnapshotRoles::SnapshotId => QVariant::from(&QString::from(&row.snapshot_id)),
            qobject::SnapshotRoles::ShortId => QVariant::from(&QString::from(&row.short_id)),
            qobject::SnapshotRoles::TimeIso => QVariant::from(&QString::from(&row.time_iso)),
            qobject::SnapshotRoles::TimeDisplay => {
                QVariant::from(&QString::from(&row.time_display))
            }
            qobject::SnapshotRoles::Hostname => QVariant::from(&QString::from(&row.hostname)),
            qobject::SnapshotRoles::PathsText => QVariant::from(&QString::from(&row.paths_text)),
            qobject::SnapshotRoles::TagsText => QVariant::from(&QString::from(&row.tags_text)),
            qobject::SnapshotRoles::StatsText => QVariant::from(&QString::from(&row.stats_text)),
            qobject::SnapshotRoles::SectionName => {
                QVariant::from(&QString::from(&row.section_name))
            }
            qobject::SnapshotRoles::IsPrevious => QVariant::from(&row.is_previous),
            _ => QVariant::default(),
        }
    }
}
