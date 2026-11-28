// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! One-line summaries of a backup's settings (retention, exclude rules, schedule, run
//! conditions, maintenance) and the sizes, durations and times they use.

use crate::i18n::{i18n, i18np, tr};
use crate::retention_texts::within_text;
use crate::time_texts::{format_datetime_display, format_time_short};
use crate::ui_texts::GuiText;
use kestic_core::model::{
    ExcludeRules, KeepCount, MaintenanceEntrySchedule, MaintenanceSchedule, NthWeekday,
    RetentionPeriod, RetentionPolicy, RetentionPreset, RunConditions, Schedule, Source, SourceKind,
    Weekday,
};
use kestic_core::scheduler::{TimezoneSpec, format_local_slot_hint, next_backup_slots};
use kestic_i18n::messages::pause_message;
use time::OffsetDateTime;

fn format_keep_count_part(kc: KeepCount, unit_singular_plural: &str) -> String {
    match kc {
        KeepCount::Unlimited => i18n("all %1").subs(&[unit_singular_plural]),
        KeepCount::Count(n) => format!("{n} {unit_singular_plural}"),
    }
}

pub fn format_retention_summary(policy: &RetentionPolicy) -> String {
    match policy.preset() {
        RetentionPreset::Standard => {
            return GuiText::PresetStandard.text();
        }
        RetentionPreset::Economical => {
            return GuiText::PresetEconomical.text();
        }
        RetentionPreset::Long => {
            return GuiText::PresetLong.text();
        }
        RetentionPreset::Custom => {}
    }

    let mut parts: Vec<String> = Vec::new();
    if let Some(kc) = policy.keep_last {
        parts.push(match kc {
            KeepCount::Unlimited => i18n("all snapshots").to_string(),
            KeepCount::Count(n) => i18n("Last %1").subs(&[&n.to_string()]),
        });
    }
    if let Some(kc) = policy.keep_hourly {
        parts.push(format_keep_count_part(kc, &i18n("hourly").to_string()));
    }
    if let Some(kc) = policy.keep_daily {
        parts.push(format_keep_count_part(kc, &i18n("daily").to_string()));
    }
    if let Some(kc) = policy.keep_weekly {
        parts.push(format_keep_count_part(kc, &i18n("weekly").to_string()));
    }
    if let Some(kc) = policy.keep_monthly {
        parts.push(format_keep_count_part(kc, &i18n("monthly").to_string()));
    }
    if let Some(kc) = policy.keep_yearly {
        parts.push(format_keep_count_part(kc, &i18n("yearly").to_string()));
    }
    let within_rules = [
        (None, &policy.keep_within),
        (Some(RetentionPeriod::Hourly), &policy.keep_within_hourly),
        (Some(RetentionPeriod::Daily), &policy.keep_within_daily),
        (Some(RetentionPeriod::Weekly), &policy.keep_within_weekly),
        (Some(RetentionPeriod::Monthly), &policy.keep_within_monthly),
        (Some(RetentionPeriod::Yearly), &policy.keep_within_yearly),
    ];
    for (period, duration) in within_rules {
        if let Some(duration) = duration {
            parts.push(within_text(period, duration));
        }
    }
    if parts.is_empty() {
        i18n("No rules configured").to_string()
    } else {
        i18n("Custom (%1)").subs(&[&parts.join(", ")])
    }
}

pub fn format_exclude_summary(excludes: &ExcludeRules) -> String {
    let mut parts: Vec<String> = Vec::new();
    let folder_count = excludes
        .folders
        .iter()
        .filter(|f| !f.trim().is_empty())
        .count();
    if folder_count > 0 {
        parts.push(i18np("%1 folder", "%1 folders", folder_count, &[]));
    }
    if excludes.exclude_caches {
        parts.push(i18n("caches").to_string());
    }
    if excludes.exclude_trash {
        parts.push(i18n("trash").to_string());
    }
    if let Some(ref limit) = excludes.exclude_larger_than
        && !limit.trim().is_empty()
    {
        parts.push(i18n("files over %1").subs(&[limit.trim()]));
    }
    let expert_count = excludes.expert_rule_count();
    if expert_count > 0 {
        parts.push(i18np(
            "%1 expert rule",
            "%1 expert rules",
            expert_count,
            &[],
        ));
    }
    if !excludes.exclude_if_present.is_empty() {
        parts.push(i18n("marker: %1").subs(&[&excludes.exclude_if_present.join(", ")]));
    }
    if parts.is_empty() {
        GuiText::None.text()
    } else {
        parts.join(" · ")
    }
}

pub fn format_source_kind_label(source: &Source) -> String {
    match source.kind {
        SourceKind::Drive => {
            if let Some(lbl) = source
                .drive_label
                .as_deref()
                .filter(|s| !s.trim().is_empty())
            {
                GuiText::DriveWithLabel.with_args(&[lbl.trim()])
            } else {
                GuiText::Drive.text()
            }
        }
        SourceKind::Folder => GuiText::Folder.text(),
    }
}

pub fn format_relative_time(ts: OffsetDateTime, now: OffsetDateTime) -> String {
    let diff = (now - ts).whole_seconds();
    if diff < 60 {
        return i18n("just now").to_string();
    }
    let mins = diff / 60;
    if mins < 60 {
        return i18np("%1 minute ago", "%1 minutes ago", mins, &[]);
    }
    let hours = mins / 60;
    if hours < 24 {
        return i18np("%1 hour ago", "%1 hours ago", hours, &[]);
    }
    i18np("%1 day ago", "%1 days ago", hours / 24, &[])
}

pub fn format_duration_ms(ms: u64) -> String {
    let total_secs = ms / 1000;
    if total_secs < 60 {
        return i18n("%1 s").subs(&[&total_secs.to_string()]);
    }
    let mins = total_secs / 60;
    let rem_secs = total_secs % 60;
    if mins < 60 {
        if rem_secs == 0 {
            return i18n("%1 min").subs(&[&mins.to_string()]);
        }
        return i18n("%1 min %2 s").subs(&[&mins.to_string(), &rem_secs.to_string()]);
    }
    let hours = mins / 60;
    let rem_mins = mins % 60;
    i18n("%1 h %2 min").subs(&[&hours.to_string(), &rem_mins.to_string()])
}

/// A size in decimal units (B, KB, MB, GB) with the decimal separator of the locale; the one
/// formatting path for sizes in models and QML.
pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1000.0;
    const MB: f64 = 1_000_000.0;
    const GB: f64 = 1_000_000_000.0;
    let b = bytes as f64;
    let decimal = crate::qt_bridge::ffi::format_decimal;
    if b >= GB {
        format!("{} GB", decimal(b / GB, 1))
    } else if b >= MB {
        format!("{} MB", decimal(b / MB, 1))
    } else if b >= KB {
        format!("{} KB", decimal(b / KB, 0))
    } else {
        format!("{bytes} B")
    }
}

fn weekday_short_label(w: Weekday) -> String {
    match w {
        Weekday::Monday => GuiText::Mon.text(),
        Weekday::Tuesday => GuiText::Tue.text(),
        Weekday::Wednesday => GuiText::Wed.text(),
        Weekday::Thursday => GuiText::Thu.text(),
        Weekday::Friday => GuiText::Fri.text(),
        Weekday::Saturday => GuiText::Sat.text(),
        Weekday::Sunday => GuiText::Sun.text(),
    }
}

fn weekday_full_label(w: Weekday) -> String {
    match w {
        Weekday::Monday => GuiText::Monday.text(),
        Weekday::Tuesday => GuiText::Tuesday.text(),
        Weekday::Wednesday => GuiText::Wednesday.text(),
        Weekday::Thursday => GuiText::Thursday.text(),
        Weekday::Friday => GuiText::Friday.text(),
        Weekday::Saturday => GuiText::Saturday.text(),
        Weekday::Sunday => GuiText::Sunday.text(),
    }
}

fn nth_weekday_label(nth: NthWeekday) -> String {
    match nth {
        NthWeekday::First => i18n("first").to_string(),
        NthWeekday::Second => i18n("second").to_string(),
        NthWeekday::Third => i18n("third").to_string(),
        NthWeekday::Fourth => i18n("fourth").to_string(),
        NthWeekday::Last => i18n("last").to_string(),
    }
}

fn weekday_plural_label(weekday: Weekday) -> String {
    match weekday {
        Weekday::Monday => i18n("Mondays").to_string(),
        Weekday::Tuesday => i18n("Tuesdays").to_string(),
        Weekday::Wednesday => i18n("Wednesdays").to_string(),
        Weekday::Thursday => i18n("Thursdays").to_string(),
        Weekday::Friday => i18n("Fridays").to_string(),
        Weekday::Saturday => i18n("Saturdays").to_string(),
        Weekday::Sunday => i18n("Sundays").to_string(),
    }
}

pub fn format_schedule_summary(schedule: &Schedule) -> String {
    match schedule {
        Schedule::ManualOnly => GuiText::ManualOnly.text(),
        Schedule::Daily { hour, minute } => {
            let hhmm = format!("{hour:02}:{minute:02}");
            i18n("Daily at %1").subs(&[&hhmm])
        }
        Schedule::SpecificWeekdays {
            weekdays,
            hour,
            minute,
        } => {
            let mut sorted = weekdays.clone();
            sorted.sort_by_key(|w| w.iso_number());
            sorted.dedup();
            let days: Vec<String> = sorted.into_iter().map(weekday_short_label).collect();
            let hhmm = format!("{hour:02}:{minute:02}");
            if days.is_empty() {
                i18n("On weekdays at %1").subs(&[&hhmm])
            } else {
                i18n("On %1 at %2").subs(&[&days.join(", "), &hhmm])
            }
        }
        Schedule::Weekly {
            weekday,
            hour,
            minute,
        } => {
            let day = Weekday::from_iso_number(*weekday)
                .map_or_else(|_| weekday.to_string(), weekday_full_label);
            let hhmm = format!("{hour:02}:{minute:02}");
            i18n("Weekly on %1 at %2").subs(&[&day, &hhmm])
        }
        Schedule::EveryNDays {
            interval_days,
            start_date,
            hour,
            minute,
        } => {
            let hhmm = format!("{hour:02}:{minute:02}");
            i18np(
                "Every %1 day from %2 at %3",
                "Every %1 days from %2 at %3",
                *interval_days,
                &[start_date, &hhmm],
            )
        }
    }
}

pub fn format_conditions_summary(conditions: &RunConditions) -> String {
    let mut parts: Vec<String> = Vec::new();
    if conditions.require_ac_power {
        parts.push(i18n("AC power only").to_string());
    }
    if conditions.require_network {
        parts.push(i18n("Network required").to_string());
    }
    if conditions.skip_on_metered_network {
        parts.push(i18n("Not on metered connection").to_string());
    }
    if !conditions.allowed_networks.is_empty() {
        parts.push(i18n("Networks: %1").subs(&[&conditions.allowed_networks.join(", ")]));
    }
    if parts.is_empty() {
        i18n("No restrictions").to_string()
    } else {
        parts.join(" · ")
    }
}

pub fn format_maintenance_entry(entry: &MaintenanceEntrySchedule) -> String {
    match entry {
        MaintenanceEntrySchedule::ManualOnly => GuiText::ManualOnly.text(),
        MaintenanceEntrySchedule::Daily { hour, minute } => {
            let hhmm = format!("{hour:02}:{minute:02}");
            i18n("Daily at %1").subs(&[&hhmm])
        }
        MaintenanceEntrySchedule::Weekly {
            weekday,
            hour,
            minute,
        } => {
            let hhmm = format!("{hour:02}:{minute:02}");
            i18n("%1 at %2").subs(&[&weekday_plural_label(*weekday), &hhmm])
        }
        MaintenanceEntrySchedule::Monthly {
            nth,
            weekday,
            hour,
            minute,
        } => {
            let hhmm = format!("{hour:02}:{minute:02}");
            i18n("%1 %2 of the month at %3").subs(&[
                &nth_weekday_label(*nth),
                &weekday_full_label(*weekday),
                &hhmm,
            ])
        }
    }
}

pub fn format_maintenance_summary(maint: &MaintenanceSchedule) -> String {
    i18n("Clean up: %1 · Verify: %2").subs(&[
        &format_maintenance_entry(&maint.prune),
        &format_maintenance_entry(&maint.check),
    ])
}

pub fn format_slot_timestamp(unix_opt: Option<i64>, now: OffsetDateTime) -> String {
    let Some(ts) = unix_opt else {
        return String::new();
    };
    let Ok(dt) = OffsetDateTime::from_unix_timestamp(ts) else {
        return String::new();
    };
    format_local_slot_hint(dt, now, &TimezoneSpec::System)
}

pub fn format_next_scheduled_line(
    next_unix: Option<i64>,
    schedule: &Schedule,
    now: OffsetDateTime,
) -> String {
    let hint = format_slot_timestamp(next_unix, now);
    if !hint.is_empty() {
        return i18n("Next: %1").subs(&[&hint]);
    }
    if matches!(schedule, Schedule::ManualOnly) {
        return i18n("Schedule: Manual only").to_string();
    }
    String::new()
}

pub fn preview_next_schedule_slots(schedule: &Schedule, now: OffsetDateTime) -> Vec<String> {
    next_backup_slots(schedule, now, 3, &TimezoneSpec::System)
        .into_iter()
        .map(format_datetime_display)
        .collect()
}

pub fn format_pause_line(
    since_unix: Option<i64>,
    until_unix: Option<i64>,
    now: OffsetDateTime,
) -> String {
    let since_str = since_unix
        .and_then(|s| OffsetDateTime::from_unix_timestamp(s).ok())
        .map_or_else(|| "–".to_string(), format_time_short);
    let until_str = format_slot_timestamp(until_unix, now);
    let until = (!until_str.is_empty()).then_some(until_str.as_str());
    tr(&pause_message(&since_str, until))
}

#[cfg(test)]
#[path = "summary_texts_tests.rs"]
mod tests;
