// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! CXX-Qt `RestoreSnapshotsModel` implementation: the snapshots of one backup in the restore
//! area, newest first, grouped by day, each with time, size and host; or the snapshots of one
//! further group (unassigned, orphaned, not configured here) of all destinations.

use super::qobject;
use super::restore_backups_model::snapshot_count_text;
use super::restore_groups_model::SnapshotGroup;
use super::rows::{id_at, row_map, row_of};
use crate::backend::restore::restore_cache;
use crate::state_cache::{format_bytes, i18n, shared_state};
use crate::time_texts::{
    cached_daemon_now, format_datetime_display, format_day_group, format_time_short, to_local,
};
use crate::ui_texts::GuiText;
use cxx_qt::CxxQtType;
use cxx_qt_lib::{QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use kestic_core::snapshot::SnapshotSize;
use kestic_core::snapshot_ls::start_dir;
use kestic_core::sync::LockUnpoisoned;
use kestic_daemon::dbus_api::{DestinationSnapshotsDto, SnapshotSummaryDto};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

model_row!(RestoreSnapshotRow {
    snapshot_id: String => "snapshotId",
    short_id: String => "shortId",
    title: String => "title",
    subtitle: String => "subtitle",
    section_name: String => "sectionName",
    // The next older snapshot of the same list; empty for the oldest one.
    previous_snapshot_id: String => "previousSnapshotId",
    destination_id: String => "destinationId",
    // Date and time for the breadcrumb of the snapshot browser.
    date_time: String => "dateTime",
    // The folder the snapshot browser opens in.
    start_dir: String => "startDir",
    // Why Kestic does not manage the snapshot here; empty for a configured backup's snapshot.
    hint: String => "hint",
    // The snapshot's tags, for expert mode.
    tags: String => "tags",
});

#[derive(Default)]
pub struct RestoreSnapshotsModelRust {
    rows: Vec<RestoreSnapshotRow>,
    /// The further group listed instead of the loaded backup.
    group: Option<SnapshotGroup>,
}

/// Size of a snapshot: the newly written data («312 MB new»), the processed data when the
/// summary lacks it, or «unknown» without a summary.
pub(crate) fn snapshot_size_text(s: &SnapshotSummaryDto) -> String {
    match SnapshotSize::from_summary(s.data_added, s.total_bytes_processed) {
        SnapshotSize::NewData(bytes) => i18n("%1 new").subs(&[&format_bytes(bytes)]),
        SnapshotSize::Processed(bytes) => format_bytes(bytes),
        SnapshotSize::Unknown => i18n("Size unknown").to_string(),
    }
}

/// Second line of a snapshot: size and host.
pub(crate) fn snapshot_subtitle(s: &SnapshotSummaryDto) -> String {
    let size = snapshot_size_text(s);
    if s.hostname.is_empty() {
        size
    } else {
        GuiText::TwoParts.with_args(&[&size, &s.hostname])
    }
}

fn weekday_text(day: time::Weekday) -> String {
    match day {
        time::Weekday::Monday => GuiText::Mon,
        time::Weekday::Tuesday => GuiText::Tue,
        time::Weekday::Wednesday => GuiText::Wed,
        time::Weekday::Thursday => GuiText::Thu,
        time::Weekday::Friday => GuiText::Fri,
        time::Weekday::Saturday => GuiText::Sat,
        time::Weekday::Sunday => GuiText::Sun,
    }
    .text()
}

/// Title and group of a snapshot taken at `t`: «12:00» under «Today» or «Yesterday»,
/// «Mon 2026-11-09 12:00» under «This Week» (Monday to the day before yesterday), otherwise date and time
/// under «Earlier».
pub(crate) fn day_section(t: OffsetDateTime, now: OffsetDateTime) -> (String, String) {
    let date = to_local(t).date();
    let today = to_local(now).date();
    if date == today || today.previous_day() == Some(date) {
        return (format_time_short(t), format_day_group(t, now));
    }
    let monday = today - time::Duration::days(i64::from(today.weekday().number_days_from_monday()));
    if date >= monday && date < today {
        let title = format!(
            "{} {}",
            weekday_text(date.weekday()),
            format_datetime_display(t)
        );
        return (title, i18n("This Week").to_string());
    }
    (format_datetime_display(t), i18n("Earlier").to_string())
}

fn parse_time(s: &SnapshotSummaryDto) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(&s.time, &Rfc3339).ok()
}

/// Second line of a snapshot of a further group: host and backed-up paths.
pub(crate) fn host_paths_subtitle(s: &SnapshotSummaryDto) -> String {
    let paths = s.paths.join(", ");
    if s.hostname.is_empty() {
        paths
    } else {
        GuiText::TwoParts.with_args(&[&s.hostname, &paths])
    }
}

/// Where a list of snapshots goes: its destination, its group (`None`: grouped by day) and the
/// hint each of its rows carries (empty: none).
struct ListPart<'a> {
    destination_id: &'a str,
    section: Option<&'a str>,
    hint: &'a str,
}

/// Rows of one list of snapshots (newest first). Snapshots of a further group (with a hint)
/// show host and paths instead of size and host.
fn push_rows(
    out: &mut Vec<RestoreSnapshotRow>,
    snaps: &[SnapshotSummaryDto],
    part: &ListPart<'_>,
    now: OffsetDateTime,
) {
    for (i, s) in snaps.iter().enumerate() {
        let time = parse_time(s);
        let date_time = time.map_or_else(|| s.time.clone(), format_datetime_display);
        let (title, section_name) = match (part.section, time) {
            (Some(name), Some(t)) => (format_datetime_display(t), name.to_string()),
            (Some(name), None) => (s.time.clone(), name.to_string()),
            (None, Some(t)) => day_section(t, now),
            (None, None) => (s.time.clone(), String::new()),
        };
        let subtitle = if part.hint.is_empty() {
            snapshot_subtitle(s)
        } else {
            host_paths_subtitle(s)
        };
        out.push(RestoreSnapshotRow {
            snapshot_id: s.id.clone(),
            short_id: s.short_id.clone(),
            title,
            subtitle,
            section_name,
            previous_snapshot_id: snaps.get(i + 1).map_or_default(|p| p.id.clone()),
            destination_id: part.destination_id.to_string(),
            date_time,
            start_dir: start_dir(&s.paths),
            hint: part.hint.to_string(),
            tags: s.tags.join(", "),
        });
    }
}

/// One backup that is not configured here, as its group in the list.
struct BackupGroup<'a> {
    backup_id: &'a str,
    count: u64,
    latest_time: Option<&'a str>,
    latest_hostname: Option<&'a str>,
}

/// Group of the snapshots of one backup that is not configured here: `head`, and on a second
/// line the number of snapshots, when and where the newest was taken and the backup ID, which
/// keeps two such backups apart.
fn backup_section(head: &str, backup: &BackupGroup<'_>) -> String {
    let count_text = snapshot_count_text(backup.count);
    let (latest_time, host) = (backup.latest_time, backup.latest_hostname);
    let latest = latest_time
        .and_then(|t| OffsetDateTime::parse(t, &Rfc3339).ok())
        .map(format_datetime_display);
    let details = match (latest, host.filter(|h| !h.is_empty())) {
        (Some(t), Some(h)) => {
            GuiText::TwoParts.with_args(&[&count_text, &i18n("latest %1 from %2").subs(&[&t, h])])
        }
        (Some(t), None) => {
            GuiText::TwoParts.with_args(&[&count_text, &i18n("latest %1").subs(&[&t])])
        }
        (None, _) => count_text,
    };
    let id = i18n("ID %1").subs(&[backup.backup_id]);
    format!("{head}\n{}", GuiText::TwoParts.with_args(&[&details, &id]))
}

/// Rows of `group` in the destination `dto` called `dest_name`.
fn push_group_rows(
    out: &mut Vec<RestoreSnapshotRow>,
    group: SnapshotGroup,
    dto: &DestinationSnapshotsDto,
    dest_name: &str,
    now: OffsetDateTime,
) {
    let not_managed = i18n("Not managed by Kestic").to_string();
    match group {
        SnapshotGroup::Unassigned => {
            let head = i18n("Unassigned on '%1'").subs(&[dest_name]);
            let count = u64::try_from(dto.unassigned.len()).unwrap_or(u64::MAX);
            let section = format!("{head}\n{}", snapshot_count_text(count));
            let part = ListPart {
                destination_id: &dto.destination_id,
                section: Some(&section),
                hint: &not_managed,
            };
            push_rows(out, &dto.unassigned, &part, now);
        }
        SnapshotGroup::Orphaned => {
            let head = i18n("Backup without description on '%1'").subs(&[dest_name]);
            for o in &dto.orphaned {
                let section = backup_section(
                    &head,
                    &BackupGroup {
                        backup_id: &o.backup_id,
                        count: o.count,
                        latest_time: o.latest_time.as_deref(),
                        latest_hostname: o.latest_hostname.as_deref(),
                    },
                );
                let part = ListPart {
                    destination_id: &dto.destination_id,
                    section: Some(&section),
                    hint: &not_managed,
                };
                push_rows(out, &o.snapshots, &part, now);
            }
        }
        SnapshotGroup::Described => {
            let head = i18n("Backup on '%1'").subs(&[dest_name]);
            let not_configured = i18n("Backup not configured here").to_string();
            for d in &dto.described {
                let section = backup_section(
                    &head,
                    &BackupGroup {
                        backup_id: &d.backup_id,
                        count: d.count,
                        latest_time: d.latest_time.as_deref(),
                        latest_hostname: d.latest_hostname.as_deref(),
                    },
                );
                let part = ListPart {
                    destination_id: &dto.destination_id,
                    section: Some(&section),
                    hint: &not_configured,
                };
                push_rows(out, &d.snapshots, &part, now);
            }
        }
    }
}

/// The configured destinations as (ID, name), in the order of the configuration.
fn configured_destinations() -> Vec<(String, String)> {
    shared_state()
        .lock_unpoisoned()
        .state
        .as_ref()
        .map_or_else(Vec::new, |s| {
            s.config
                .destinations
                .iter()
                .map(|d| (d.id.as_str().to_string(), d.name.clone()))
                .collect()
        })
}

/// Rows of `group` over all destinations whose snapshots are loaded.
fn group_rows(group: SnapshotGroup, now: OffsetDateTime) -> Vec<RestoreSnapshotRow> {
    let destinations = configured_destinations();
    let cache = restore_cache().lock_unpoisoned();
    let mut out = Vec::new();
    for (id, name) in &destinations {
        if let Some(Ok(dto)) = cache.destinations.get(id)
            && dto.error_kind.is_none()
        {
            push_group_rows(&mut out, group, dto, name, now);
        }
    }
    out
}

impl RestoreSnapshotsModelRust {
    fn compute_rows(&self) -> Vec<RestoreSnapshotRow> {
        let now = cached_daemon_now();
        if let Some(group) = self.group {
            return group_rows(group, now);
        }
        let current_dest_id = |backup_id: &str| {
            crate::state_cache::shared_state()
                .lock_unpoisoned()
                .state
                .as_ref()
                .and_then(|s| {
                    s.backups
                        .iter()
                        .find(|b| b.backup.id.as_str() == backup_id)
                        .map(|b| b.backup.destination_id.as_str().to_string())
                })
                .unwrap_or_default()
        };
        // Copied out, so the restore cache is never held while the shared state is locked.
        let loaded = match &restore_cache().lock_unpoisoned().backup_snapshots {
            Some((backup_id, Ok(dto))) => Some((backup_id.clone(), dto.clone())),
            _ => None,
        };
        let Some((backup_id, dto)) = loaded else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let current = ListPart {
            destination_id: &current_dest_id(&backup_id),
            section: None,
            hint: "",
        };
        push_rows(&mut out, &dto.current_snapshots, &current, now);
        for prev in &dto.previous_destinations {
            let section = GuiText::EarlierSnapshotsOn.with_args(&[&prev.destination_name]);
            let part = ListPart {
                destination_id: &prev.destination_id,
                section: Some(&section),
                hint: "",
            };
            push_rows(&mut out, &prev.snapshots, &part, now);
        }
        out
    }
}

impl qobject::RestoreSnapshotsModel {
    pub fn row_of(&self, snapshot_id: &QString) -> i32 {
        let id = snapshot_id.to_string();
        row_of(&self.rows, |r| r.snapshot_id == id)
    }

    pub fn id_at(&self, row: i32) -> QString {
        id_at(&self.rows, row, |r| &r.snapshot_id)
    }

    pub fn row_map(&self, snapshot_id: &QString) -> QVariant {
        let id = snapshot_id.to_string();
        row_map(self.rows.iter().find(|r| r.snapshot_id == id))
    }

    pub fn set_group(self: std::pin::Pin<&mut Self>, group: &QString) {
        self.rust_mut().group = SnapshotGroup::parse(&group.to_string());
    }

    pub fn group_error(&self) -> QString {
        if self.group.is_none() {
            return QString::default();
        }
        QString::from(&destination_errors())
    }
}

/// Why destinations could not be listed, one line per destination: «'NAS': <reason>».
fn destination_errors() -> String {
    let destinations = configured_destinations();
    let cache = restore_cache().lock_unpoisoned();
    destinations
        .iter()
        .filter_map(|(id, name)| {
            let reason = match cache.destinations.get(id)? {
                Err(err) => err.clone(),
                Ok(dto) => super::restore_backups_model::destination_error_text(dto)?,
            };
            Some(i18n("'%1': %2").subs(&[name, &reason]))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

list_model!(
    RestoreSnapshotsModel,
    RestoreSnapshotRow,
    same: |a, b| a.snapshot_id == b.snapshot_id && a.section_name == b.section_name,
    begin_reset: begin_reset_restore_snapshots,
    end_reset: end_reset_restore_snapshots,
    index: index_restore_snapshots,
    changed: data_changed_restore_snapshots,
);

#[cfg(test)]
#[path = "restore_snapshots_model_tests.rs"]
mod tests;
