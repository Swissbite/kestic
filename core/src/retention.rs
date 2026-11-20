// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Scheduling, run conditions, notifications, retention policies, and recovery metadata (`M2-S05`, `M3-S01`).

use crate::model::{BackupId, DestinationId, ModelError, Weekday};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use time::OffsetDateTime;

/// Schedule definition (glossary: `Schedule`, `FR-03`, `P13`, `P35`, `M3-S01`, `§7.4`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Schedule {
    #[default]
    ManualOnly,
    Daily {
        hour: u8,
        minute: u8,
    },
    SpecificWeekdays {
        weekdays: Vec<Weekday>,
        hour: u8,
        minute: u8,
    },
    Weekly {
        weekday: u8,
        hour: u8,
        minute: u8,
    },
    EveryNDays {
        interval_days: u32,
        /// Start date in `YYYY-MM-DD` format (`§7.4`).
        start_date: String,
        hour: u8,
        minute: u8,
    },
}

impl Schedule {
    /// Default schedule for newly configured scheduled backups (`P35`: daily 12:00).
    pub fn default_scheduled() -> Self {
        Self::Daily {
            hour: 12,
            minute: 0,
        }
    }

    pub fn validate(&self) -> Result<(), ModelError> {
        match self {
            Self::ManualOnly => Ok(()),
            Self::Daily { hour, minute } => validate_hh_mm(*hour, *minute),
            Self::SpecificWeekdays {
                weekdays,
                hour,
                minute,
            } => {
                if weekdays.is_empty() {
                    return Err(ModelError::InvalidBackup(
                        "weekday schedule requires at least one weekday".to_string(),
                    ));
                }
                validate_hh_mm(*hour, *minute)
            }
            Self::Weekly {
                weekday,
                hour,
                minute,
            } => {
                Weekday::from_iso_number(*weekday)?;
                validate_hh_mm(*hour, *minute)
            }
            Self::EveryNDays {
                interval_days,
                start_date,
                hour,
                minute,
            } => {
                if !(2..=365).contains(interval_days) {
                    return Err(ModelError::InvalidBackup(format!(
                        "interval_days must be between 2 and 365, got {interval_days}"
                    )));
                }
                start_date.parse::<jiff::civil::Date>().map_err(|err| {
                    ModelError::InvalidBackup(format!(
                        "invalid schedule start_date '{start_date}': {err}"
                    ))
                })?;
                validate_hh_mm(*hour, *minute)
            }
        }
    }
}

/// Parses and validates a serialized [`Schedule`] JSON string (`U18`, `§7.4`).
pub fn parse_and_validate_schedule_json(raw: &str) -> Result<Schedule, ModelError> {
    let sched: Schedule = serde_json::from_str(raw)
        .map_err(|err| ModelError::InvalidBackup(format!("invalid schedule JSON: {err}")))?;
    sched.validate()?;
    Ok(sched)
}

fn validate_hh_mm(hour: u8, minute: u8) -> Result<(), ModelError> {
    if hour >= 24 || minute >= 60 {
        return Err(ModelError::InvalidBackup(format!(
            "invalid schedule time {hour:02}:{minute:02}"
        )));
    }
    Ok(())
}

fn default_true() -> bool {
    true
}

/// Conditions controlling when scheduled runs may start (glossary: `RunConditions`, `FR-13`, `P35`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunConditions {
    #[serde(alias = "skip_on_battery", default = "default_true")]
    pub require_ac_power: bool,
    #[serde(default = "default_true")]
    pub require_network: bool,
    #[serde(default)]
    pub skip_on_metered_network: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_networks: Vec<String>,
}

impl Default for RunConditions {
    fn default() -> Self {
        Self {
            require_ac_power: true,
            require_network: true,
            skip_on_metered_network: false,
            allowed_networks: Vec::new(),
        }
    }
}

/// Per-event notification toggles (`F3`, `§9.6`, `AC-M3-08-1`, `AC-M3-08-2`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotificationSettings {
    pub on_error: bool,
    pub on_warning: bool,
    pub on_overdue: bool,
    pub on_maintenance_failed: bool,
    pub on_restic_update: bool,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            on_error: true,
            on_warning: true,
            on_overdue: true,
            on_maintenance_failed: true,
            on_restic_update: true,
        }
    }
}

/// Default number of missed scheduled slots before a backup is marked overdue (`M3-S02`).
pub const DEFAULT_OVERDUE_THRESHOLD: u32 = 2;

/// Count specifier for retention rules (`--keep-daily 7` or `--keep-yearly unlimited`, `AC-M2-05-1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepCount {
    Count(u32),
    Unlimited,
}

impl From<u32> for KeepCount {
    fn from(n: u32) -> Self {
        Self::Count(n)
    }
}

impl fmt::Display for KeepCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Count(n) => write!(f, "{n}"),
            Self::Unlimited => f.write_str("unlimited"),
        }
    }
}

impl Serialize for KeepCount {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Count(n) => serializer.serialize_u32(*n),
            Self::Unlimited => serializer.serialize_str("unlimited"),
        }
    }
}

impl<'de> Deserialize<'de> for KeepCount {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct KeepCountVisitor;
        impl serde::de::Visitor<'_> for KeepCountVisitor {
            type Value = KeepCount;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a non-negative integer or \"unlimited\"")
            }

            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value, E> {
                let n = u32::try_from(v).map_err(|_| E::custom("keep count exceeds u32"))?;
                Ok(KeepCount::Count(n))
            }

            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value, E> {
                if v < 0 {
                    return Err(E::custom("keep count cannot be negative"));
                }
                let n = u32::try_from(v).map_err(|_| E::custom("keep count exceeds u32"))?;
                Ok(KeepCount::Count(n))
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                if v.eq_ignore_ascii_case("unlimited") {
                    Ok(KeepCount::Unlimited)
                } else {
                    Err(E::custom(format!("expected \"unlimited\", got '{v}'")))
                }
            }
        }
        deserializer.deserialize_any(KeepCountVisitor)
    }
}

/// Built-in retention presets (`M2-S05`, `M2-S06`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetentionPreset {
    Standard,
    Economical,
    Long,
    Custom,
}

/// Validates a restic `--keep-within*` duration (`y`, `m`, `d`, `h` chunks, e.g. `48h`, `30d`, `1y6m`).
pub fn validate_restic_duration(raw: &str) -> Result<String, ModelError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ModelError::InvalidBackup(
            "retention duration must not be empty".to_string(),
        ));
    }
    let bytes = trimmed.as_bytes();
    let mut i = 0;
    let mut total_value: u64 = 0;
    while i < bytes.len() {
        let num_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == num_start || i >= bytes.len() {
            return Err(ModelError::InvalidBackup(format!(
                "invalid retention duration '{raw}': expected <number><y|m|d|h>"
            )));
        }
        let num_str = &trimmed[num_start..i];
        let val: u64 = num_str.parse().map_err(|_| {
            ModelError::InvalidBackup(format!("invalid number in duration '{raw}'"))
        })?;
        total_value = total_value.saturating_add(val);
        let unit = bytes[i];
        if !matches!(unit, b'y' | b'm' | b'd' | b'h') {
            return Err(ModelError::InvalidBackup(format!(
                "invalid duration unit '{}' in '{raw}': allowed units are y, m, d, h",
                unit as char
            )));
        }
        i += 1;
    }
    if total_value == 0 {
        return Err(ModelError::InvalidBackup(format!(
            "retention duration '{raw}' must be greater than 0"
        )));
    }
    Ok(trimmed.to_string())
}

/// Snapshot retention configuration (glossary: `RetentionPolicy`, `M2-S05`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default = "RetentionPolicy::empty")]
pub struct RetentionPolicy {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_last: Option<KeepCount>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_hourly: Option<KeepCount>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_daily: Option<KeepCount>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_weekly: Option<KeepCount>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_monthly: Option<KeepCount>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_yearly: Option<KeepCount>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_within: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_within_hourly: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_within_daily: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_within_weekly: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_within_monthly: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_within_yearly: Option<String>,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self::standard()
    }
}

impl RetentionPolicy {
    /// Empty retention policy (invalid until at least one rule is set, `AC-M2-05-4`).
    pub fn empty() -> Self {
        Self {
            keep_last: None,
            keep_hourly: None,
            keep_daily: None,
            keep_weekly: None,
            keep_monthly: None,
            keep_yearly: None,
            keep_within: None,
            keep_within_hourly: None,
            keep_within_daily: None,
            keep_within_weekly: None,
            keep_within_monthly: None,
            keep_within_yearly: None,
        }
    }

    /// Standard preset: 7 daily, 4 weekly, 12 monthly (`AC-M2-05-3`).
    pub fn standard() -> Self {
        Self {
            keep_daily: Some(KeepCount::Count(7)),
            keep_weekly: Some(KeepCount::Count(4)),
            keep_monthly: Some(KeepCount::Count(12)),
            ..Self::empty()
        }
    }

    /// Economical preset ("Sparsam"): 7 daily, 4 weekly (`M2-S06`).
    pub fn economical() -> Self {
        Self {
            keep_daily: Some(KeepCount::Count(7)),
            keep_weekly: Some(KeepCount::Count(4)),
            ..Self::empty()
        }
    }

    /// Long preset ("Lang"): 7 daily, 4 weekly, 12 monthly, unlimited yearly (`M2-S06`).
    pub fn long() -> Self {
        Self {
            keep_daily: Some(KeepCount::Count(7)),
            keep_weekly: Some(KeepCount::Count(4)),
            keep_monthly: Some(KeepCount::Count(12)),
            keep_yearly: Some(KeepCount::Unlimited),
            ..Self::empty()
        }
    }

    /// Identifies which preset this policy matches, or [`RetentionPreset::Custom`].
    pub fn preset(&self) -> RetentionPreset {
        if *self == Self::standard() {
            RetentionPreset::Standard
        } else if *self == Self::economical() {
            RetentionPreset::Economical
        } else if *self == Self::long() {
            RetentionPreset::Long
        } else {
            RetentionPreset::Custom
        }
    }

    /// Validates all retention counts/durations and ensures at least one active rule (`AC-M2-05-4`).
    pub fn validate(&self) -> Result<(), ModelError> {
        let counts = [
            ("keep_last", self.keep_last),
            ("keep_hourly", self.keep_hourly),
            ("keep_daily", self.keep_daily),
            ("keep_weekly", self.keep_weekly),
            ("keep_monthly", self.keep_monthly),
            ("keep_yearly", self.keep_yearly),
        ];
        let mut active_rules = 0usize;
        for (name, opt) in counts {
            if let Some(kc) = opt {
                match kc {
                    KeepCount::Count(0) => {
                        return Err(ModelError::InvalidBackup(format!(
                            "{name} must be at least 1 (or unset)"
                        )));
                    }
                    KeepCount::Count(_) | KeepCount::Unlimited => active_rules += 1,
                }
            }
        }

        let durations = [
            ("keep_within", &self.keep_within),
            ("keep_within_hourly", &self.keep_within_hourly),
            ("keep_within_daily", &self.keep_within_daily),
            ("keep_within_weekly", &self.keep_within_weekly),
            ("keep_within_monthly", &self.keep_within_monthly),
            ("keep_within_yearly", &self.keep_within_yearly),
        ];
        for (_name, opt) in durations {
            if let Some(raw) = opt {
                if !raw.trim().is_empty() {
                    validate_restic_duration(raw)?;
                    active_rules += 1;
                }
            }
        }

        if active_rules == 0 {
            return Err(ModelError::InvalidBackup(
                "retention policy must specify at least one active keep rule".to_string(),
            ));
        }
        Ok(())
    }

    /// Converts active retention rules into restic `forget` CLI flags.
    /// Never emits `--unsafe-allow-remove-all` (`AC-M2-05-5`, `FR-05`).
    pub fn to_forget_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        let count_flags = [
            ("--keep-last", self.keep_last),
            ("--keep-hourly", self.keep_hourly),
            ("--keep-daily", self.keep_daily),
            ("--keep-weekly", self.keep_weekly),
            ("--keep-monthly", self.keep_monthly),
            ("--keep-yearly", self.keep_yearly),
        ];
        for (flag, opt) in count_flags {
            if let Some(val) = opt {
                args.push(flag.to_string());
                args.push(val.to_string());
            }
        }

        let duration_flags = [
            ("--keep-within", &self.keep_within),
            ("--keep-within-hourly", &self.keep_within_hourly),
            ("--keep-within-daily", &self.keep_within_daily),
            ("--keep-within-weekly", &self.keep_within_weekly),
            ("--keep-within-monthly", &self.keep_within_monthly),
            ("--keep-within-yearly", &self.keep_within_yearly),
        ];
        for (flag, opt) in duration_flags {
            if let Some(raw) = opt {
                let trimmed = raw.trim();
                if !trimmed.is_empty() {
                    args.push(flag.to_string());
                    args.push(trimmed.to_string());
                }
            }
        }
        args
    }
}

/// Recovery kit metadata associated with a repository key (glossary: `RecoveryKit`, M6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryKit {
    pub destination_id: DestinationId,
    pub key_id: String,
    pub created_at: OffsetDateTime,
}

/// Metadata snapshot reference (`kestic-meta:<backup-id>`, glossary: `MetaSnapshot`, M6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetaSnapshot {
    pub snapshot_id: String,
    pub backup_id: BackupId,
    pub created_at: OffsetDateTime,
}
