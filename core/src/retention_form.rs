// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The fields of the retention editor: read from a [`RetentionPolicy`], written back to one,
//! compared with the saved policy and matched against the presets.

use crate::retention::{KeepCount, RetentionPolicy, RetentionPreset};
use serde::{Deserialize, Serialize};

/// One count rule of the editor (`--keep-daily 7`, or all with «unlimited»).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CountRule {
    pub on: bool,
    pub all: bool,
    pub num: u32,
}

/// The unit of a duration rule as restic writes it (`48h`, `30d`, `6m`, `5y`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DurationUnit {
    #[serde(rename = "h")]
    Hours,
    #[serde(rename = "d")]
    Days,
    #[serde(rename = "m")]
    Months,
    #[serde(rename = "y")]
    Years,
}

impl DurationUnit {
    fn letter(self) -> char {
        match self {
            Self::Hours => 'h',
            Self::Days => 'd',
            Self::Months => 'm',
            Self::Years => 'y',
        }
    }

    fn from_letter(letter: char) -> Option<Self> {
        match letter {
            'h' => Some(Self::Hours),
            'd' => Some(Self::Days),
            'm' => Some(Self::Months),
            'y' => Some(Self::Years),
            _ => None,
        }
    }
}

/// One duration rule of the editor (`--keep-within-daily 14d`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WithinRule {
    pub on: bool,
    pub num: u32,
    pub unit: DurationUnit,
}

/// All rules of the retention editor; a rule that is off keeps the value it offers when the
/// user turns it on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionForm {
    pub keep_last: CountRule,
    pub keep_hourly: CountRule,
    pub keep_daily: CountRule,
    pub keep_weekly: CountRule,
    pub keep_monthly: CountRule,
    pub keep_yearly: CountRule,
    pub keep_within: WithinRule,
    pub keep_within_hourly: WithinRule,
    pub keep_within_daily: WithinRule,
    pub keep_within_weekly: WithinRule,
    pub keep_within_monthly: WithinRule,
    pub keep_within_yearly: WithinRule,
}

/// What the editor shows for its current fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RetentionFormState {
    /// The policy the fields stand for.
    pub policy: RetentionPolicy,
    /// The preset the policy matches, or `custom`.
    pub preset: RetentionPreset,
    /// The policy differs from the saved one.
    pub dirty: bool,
    /// At least one rule is on.
    pub has_rule: bool,
}

const fn count_off(num: u32) -> CountRule {
    CountRule {
        on: false,
        all: false,
        num,
    }
}

const fn within_off(num: u32, unit: DurationUnit) -> WithinRule {
    WithinRule {
        on: false,
        num,
        unit,
    }
}

impl Default for RetentionForm {
    /// Every rule off, each with the value it offers when turned on.
    fn default() -> Self {
        Self {
            keep_last: count_off(10),
            keep_hourly: count_off(24),
            keep_daily: count_off(7),
            keep_weekly: count_off(4),
            keep_monthly: count_off(12),
            keep_yearly: count_off(5),
            keep_within: within_off(30, DurationUnit::Days),
            keep_within_hourly: within_off(48, DurationUnit::Hours),
            keep_within_daily: within_off(14, DurationUnit::Days),
            keep_within_weekly: within_off(6, DurationUnit::Months),
            keep_within_monthly: within_off(1, DurationUnit::Years),
            keep_within_yearly: within_off(5, DurationUnit::Years),
        }
    }
}

/// A count rule as the editor shows it; a count of 0 is a rule that is off.
fn count_rule(value: Option<KeepCount>, default: CountRule) -> CountRule {
    match value {
        Some(KeepCount::Unlimited) => CountRule {
            on: true,
            all: true,
            ..default
        },
        Some(KeepCount::Count(n)) if n > 0 => CountRule {
            on: true,
            all: false,
            num: n,
        },
        _ => default,
    }
}

/// A duration rule as the editor shows it: one number and one unit (`30d`). A duration the
/// editor cannot show that way (`1y6m`) is a rule that is off.
fn within_rule(value: Option<&str>, default: WithinRule) -> WithinRule {
    let Some(raw) = value.map(str::trim) else {
        return default;
    };
    let Some(letter) = raw.chars().last() else {
        return default;
    };
    let digits = &raw[..raw.len() - letter.len_utf8()];
    let unit = DurationUnit::from_letter(letter);
    let num = (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
        .then(|| digits.parse::<u32>().ok())
        .flatten();
    match (num, unit) {
        (Some(num), Some(unit)) => WithinRule {
            on: true,
            num: num.max(1),
            unit,
        },
        _ => default,
    }
}

fn count_value(rule: CountRule) -> Option<KeepCount> {
    match rule {
        CountRule { on: false, .. } => None,
        CountRule { all: true, .. } => Some(KeepCount::Unlimited),
        CountRule { num, .. } => Some(KeepCount::Count(num.max(1))),
    }
}

fn within_value(rule: WithinRule) -> Option<String> {
    rule.on
        .then(|| format!("{}{}", rule.num.max(1), rule.unit.letter()))
}

impl RetentionForm {
    /// The fields for `policy`.
    pub fn from_policy(policy: &RetentionPolicy) -> Self {
        let d = Self::default();
        Self {
            keep_last: count_rule(policy.keep_last, d.keep_last),
            keep_hourly: count_rule(policy.keep_hourly, d.keep_hourly),
            keep_daily: count_rule(policy.keep_daily, d.keep_daily),
            keep_weekly: count_rule(policy.keep_weekly, d.keep_weekly),
            keep_monthly: count_rule(policy.keep_monthly, d.keep_monthly),
            keep_yearly: count_rule(policy.keep_yearly, d.keep_yearly),
            keep_within: within_rule(policy.keep_within.as_deref(), d.keep_within),
            keep_within_hourly: within_rule(
                policy.keep_within_hourly.as_deref(),
                d.keep_within_hourly,
            ),
            keep_within_daily: within_rule(
                policy.keep_within_daily.as_deref(),
                d.keep_within_daily,
            ),
            keep_within_weekly: within_rule(
                policy.keep_within_weekly.as_deref(),
                d.keep_within_weekly,
            ),
            keep_within_monthly: within_rule(
                policy.keep_within_monthly.as_deref(),
                d.keep_within_monthly,
            ),
            keep_within_yearly: within_rule(
                policy.keep_within_yearly.as_deref(),
                d.keep_within_yearly,
            ),
        }
    }

    /// The policy the fields stand for; a count or duration below 1 counts as 1.
    pub fn to_policy(&self) -> RetentionPolicy {
        RetentionPolicy {
            keep_last: count_value(self.keep_last),
            keep_hourly: count_value(self.keep_hourly),
            keep_daily: count_value(self.keep_daily),
            keep_weekly: count_value(self.keep_weekly),
            keep_monthly: count_value(self.keep_monthly),
            keep_yearly: count_value(self.keep_yearly),
            keep_within: within_value(self.keep_within),
            keep_within_hourly: within_value(self.keep_within_hourly),
            keep_within_daily: within_value(self.keep_within_daily),
            keep_within_weekly: within_value(self.keep_within_weekly),
            keep_within_monthly: within_value(self.keep_within_monthly),
            keep_within_yearly: within_value(self.keep_within_yearly),
        }
    }

    /// Whether at least one rule is on.
    pub fn has_rule(&self) -> bool {
        [
            self.keep_last,
            self.keep_hourly,
            self.keep_daily,
            self.keep_weekly,
            self.keep_monthly,
            self.keep_yearly,
        ]
        .iter()
        .any(|r| r.on)
            || [
                self.keep_within,
                self.keep_within_hourly,
                self.keep_within_daily,
                self.keep_within_weekly,
                self.keep_within_monthly,
                self.keep_within_yearly,
            ]
            .iter()
            .any(|r| r.on)
    }

    /// The policy, its preset and whether it differs from `saved`.
    pub fn state(&self, saved: &RetentionPolicy) -> RetentionFormState {
        let policy = self.to_policy();
        RetentionFormState {
            preset: policy.preset(),
            dirty: policy != *saved,
            has_rule: self.has_rule(),
            policy,
        }
    }
}

#[cfg(test)]
#[path = "retention_form_tests.rs"]
mod tests;
