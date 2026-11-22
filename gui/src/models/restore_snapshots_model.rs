// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! CXX-Qt `RestoreSnapshotsModel` implementation: the snapshots of one backup in the restore
//! area, newest first, grouped by day, each with time, size and host.

use super::qobject;
use super::rows::{id_at, row_map, row_of};
use crate::backend::restore::restore_cache;
use crate::state_cache::{
    cached_daemon_now, format_bytes, format_datetime_display, format_day_group, format_time_short,
    i18n, to_local,
};
use crate::ui_texts::GuiText;
use cxx_qt::CxxQtType;
use cxx_qt_lib::{QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use kestic_core::snapshot::SnapshotSize;
use kestic_core::snapshot_ls::start_dir;
use kestic_core::sync::LockUnpoisoned;
use kestic_daemon::dbus_api::SnapshotSummaryDto;
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
});

#[derive(Default)]
pub struct RestoreSnapshotsModelRust {
    rows: Vec<RestoreSnapshotRow>,
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

/// Rows of one list of snapshots (newest first); `section` names the group, or `None` to group
/// by day.
fn push_rows(
    out: &mut Vec<RestoreSnapshotRow>,
    snaps: &[SnapshotSummaryDto],
    destination_id: &str,
    section: Option<&str>,
    now: OffsetDateTime,
) {
    for (i, s) in snaps.iter().enumerate() {
        let time = parse_time(s);
        let date_time = time.map_or_else(|| s.time.clone(), format_datetime_display);
        let (title, section_name) = match (section, time) {
            (Some(name), Some(t)) => (format_datetime_display(t), name.to_string()),
            (Some(name), None) => (s.time.clone(), name.to_string()),
            (None, Some(t)) => day_section(t, now),
            (None, None) => (s.time.clone(), String::new()),
        };
        out.push(RestoreSnapshotRow {
            snapshot_id: s.id.clone(),
            short_id: s.short_id.clone(),
            title,
            subtitle: snapshot_subtitle(s),
            section_name,
            previous_snapshot_id: snaps.get(i + 1).map_or_default(|p| p.id.clone()),
            destination_id: destination_id.to_string(),
            date_time,
            start_dir: start_dir(&s.paths),
        });
    }
}

impl RestoreSnapshotsModelRust {
    fn compute_rows(&self) -> Vec<RestoreSnapshotRow> {
        let now = cached_daemon_now();
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
        push_rows(
            &mut out,
            &dto.current_snapshots,
            &current_dest_id(&backup_id),
            None,
            now,
        );
        for prev in &dto.previous_destinations {
            let section = GuiText::EarlierSnapshotsOn.with_args(&[&prev.destination_name]);
            push_rows(
                &mut out,
                &prev.snapshots,
                &prev.destination_id,
                Some(&section),
                now,
            );
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
