// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! CXX-Qt `RestoreBackupsModel` implementation: the backups of the restore area («Which
//! backup?») with the number of snapshots on their destination.

use super::qobject;
use super::rows::{id_at, row_of};
use crate::backend::restore::restore_cache;
use crate::state_cache::{i18np, shared_state, tr};
use crate::ui_texts::GuiText;
use cxx_qt::CxxQtType;
use cxx_qt_lib::{QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use kestic_core::sync::LockUnpoisoned;
use kestic_daemon::dbus_api::DestinationSnapshotsDto;
use kestic_i18n::codes::ErrorKind;

model_row!(RestoreBackupRow {
    backup_id: String => "backupId",
    name: String => "name",
    subtitle: String => "subtitle",
});

#[derive(Default)]
pub struct RestoreBackupsModelRust {
    rows: Vec<RestoreBackupRow>,
}

/// «n snapshots».
pub(crate) fn snapshot_count_text(count: u64) -> String {
    i18np("%1 snapshot", "%1 snapshots", count, &[])
}

/// Why the snapshots of a destination could not be listed; `None` when they were.
pub(crate) fn destination_error_text(dto: &DestinationSnapshotsDto) -> Option<String> {
    match dto.error_kind? {
        ErrorKind::Unknown => Some(dto.error_detail.clone().unwrap_or_default()),
        kind => Some(tr(&kind.message(None, false))),
    }
}

/// Second line of a backup in the restore area: its number of snapshots, «loading» or why
/// the destination could not be read.
pub(crate) fn restore_backup_subtitle(
    backup_id: &str,
    destination: Option<&Result<DestinationSnapshotsDto, String>>,
) -> String {
    match destination {
        None => GuiText::LoadingSnapshots.text(),
        Some(Err(err)) => err.clone(),
        Some(Ok(dto)) => destination_error_text(dto).unwrap_or_else(|| {
            let count = dto
                .backups
                .iter()
                .find(|b| b.backup_id == backup_id)
                .map_or(0, |b| b.count);
            snapshot_count_text(count)
        }),
    }
}

impl RestoreBackupsModelRust {
    fn compute_rows(&self) -> Vec<RestoreBackupRow> {
        let guard = shared_state().lock_unpoisoned();
        let Some(state) = &guard.state else {
            return Vec::new();
        };
        let cache = restore_cache().lock_unpoisoned();
        state
            .backups
            .iter()
            .map(|b| {
                let dest_id = b.backup.destination_id.as_str();
                RestoreBackupRow {
                    backup_id: b.backup.id.as_str().to_string(),
                    name: b.backup.name.clone(),
                    subtitle: restore_backup_subtitle(
                        b.backup.id.as_str(),
                        cache.destinations.get(dest_id),
                    ),
                }
            })
            .collect()
    }
}

impl qobject::RestoreBackupsModel {
    pub fn row_of(&self, backup_id: &QString) -> i32 {
        let id = backup_id.to_string();
        row_of(&self.rows, |r| r.backup_id == id)
    }

    pub fn id_at(&self, row: i32) -> QString {
        id_at(&self.rows, row, |r| &r.backup_id)
    }
}

list_model!(
    RestoreBackupsModel,
    RestoreBackupRow,
    same: |a, b| a.backup_id == b.backup_id,
    begin_reset: begin_reset_restore_backups,
    end_reset: end_reset_restore_backups,
    index: index_restore_backups,
    changed: data_changed_restore_backups,
);
