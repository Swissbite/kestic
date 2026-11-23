// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! CXX-Qt `RestoreGroupsModel` implementation: the further snapshots of the restore area
//! («Other snapshots»): unassigned snapshots, orphaned backups and backups that are not
//! configured here, each over all destinations.

use crate::backend::restore::restore_cache;
use crate::state_cache::{i18n, i18np, shared_state};
use crate::ui_texts::GuiText;
use cxx_qt::CxxQtType;
use cxx_qt_lib::{QHash, QHashPair_i32_QByteArray, QModelIndex, QVariant};
use kestic_core::sync::LockUnpoisoned;
use kestic_daemon::dbus_api::DestinationSnapshotsDto;

/// A group of further snapshots; its code is the `group` of the snapshot list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SnapshotGroup {
    /// Snapshots without a unique Kestic tag.
    Unassigned,
    /// Snapshots of a backup that neither the configuration nor a metadata snapshot knows.
    Orphaned,
    /// Snapshots of a backup that is not configured here but described by a metadata snapshot.
    Described,
}

impl SnapshotGroup {
    pub(crate) fn parse(code: &str) -> Option<Self> {
        match code {
            "unassigned" => Some(Self::Unassigned),
            "orphaned" => Some(Self::Orphaned),
            "described" => Some(Self::Described),
            _ => None,
        }
    }

    fn code(self) -> &'static str {
        match self {
            Self::Unassigned => "unassigned",
            Self::Orphaned => "orphaned",
            Self::Described => "described",
        }
    }

    /// Name of the group in the restore area; also the title of its snapshot list.
    pub(crate) fn name(self) -> String {
        match self {
            Self::Unassigned => i18n("Unassigned").to_string(),
            Self::Orphaned => i18n("Orphaned").to_string(),
            Self::Described => i18n("Backups not configured here").to_string(),
        }
    }

    fn icon_name(self) -> &'static str {
        match self {
            Self::Unassigned => "dialog-question",
            Self::Orphaned => "dialog-warning",
            Self::Described => "dialog-information",
        }
    }

    /// Number of entries the group shows in one destination: snapshots for unassigned ones,
    /// backups otherwise.
    fn count_in(self, dto: &DestinationSnapshotsDto) -> usize {
        match self {
            Self::Unassigned => dto.unassigned.len(),
            Self::Orphaned => dto.orphaned.len(),
            Self::Described => dto.described.len(),
        }
    }

    fn count_text(self, count: usize) -> String {
        match self {
            Self::Unassigned => i18np(
                "%1 snapshot without a Kestic backup",
                "%1 snapshots without a Kestic backup",
                count,
                &[],
            ),
            Self::Orphaned => i18np(
                "%1 backup without description",
                "%1 backups without description",
                count,
                &[],
            ),
            Self::Described => i18np(
                "%1 backup with description",
                "%1 backups with description",
                count,
                &[],
            ),
        }
    }
}

model_row!(RestoreGroupRow {
    group: String => "group",
    name: String => "name",
    subtitle: String => "subtitle",
    icon_name: String => "iconName",
});

#[derive(Default)]
pub struct RestoreGroupsModelRust {
    rows: Vec<RestoreGroupRow>,
}

/// The rows of the groups for the destinations `loaded` (`None`: not answered yet). Without
/// destinations there are none; backups not configured here only show when there are some.
pub(crate) fn group_rows(
    loaded: &[Option<&Result<DestinationSnapshotsDto, String>>],
) -> Vec<(SnapshotGroup, String)> {
    if loaded.is_empty() {
        return Vec::new();
    }
    let loading = loaded.iter().any(Option::is_none);
    let listed: Vec<&DestinationSnapshotsDto> = loaded
        .iter()
        .flatten()
        .filter_map(|r| r.as_ref().ok())
        .filter(|dto| dto.error_kind.is_none())
        .collect();
    let count = |group: SnapshotGroup| listed.iter().map(|dto| group.count_in(dto)).sum();
    [
        SnapshotGroup::Unassigned,
        SnapshotGroup::Orphaned,
        SnapshotGroup::Described,
    ]
    .into_iter()
    .filter_map(|group| {
        let n: usize = count(group);
        if group == SnapshotGroup::Described && (loading || n == 0) {
            return None;
        }
        let subtitle = if loading {
            GuiText::LoadingSnapshots.text()
        } else {
            group.count_text(n)
        };
        Some((group, subtitle))
    })
    .collect()
}

impl RestoreGroupsModelRust {
    fn compute_rows(&self) -> Vec<RestoreGroupRow> {
        let dest_ids: Vec<String> =
            shared_state()
                .lock_unpoisoned()
                .state
                .as_ref()
                .map_or_else(Vec::new, |s| {
                    s.config
                        .destinations
                        .iter()
                        .map(|d| d.id.as_str().to_string())
                        .collect()
                });
        let groups = {
            let cache = restore_cache().lock_unpoisoned();
            let loaded: Vec<_> = dest_ids
                .iter()
                .map(|id| cache.destinations.get(id))
                .collect();
            group_rows(&loaded)
        };
        groups
            .into_iter()
            .map(|(group, subtitle)| RestoreGroupRow {
                group: group.code().to_string(),
                name: group.name(),
                subtitle,
                icon_name: group.icon_name().to_string(),
            })
            .collect()
    }
}

list_model!(
    RestoreGroupsModel,
    RestoreGroupRow,
    same: |a, b| a.group == b.group,
    begin_reset: begin_reset_restore_groups,
    end_reset: end_reset_restore_groups,
    index: index_restore_groups,
    changed: data_changed_restore_groups,
);
