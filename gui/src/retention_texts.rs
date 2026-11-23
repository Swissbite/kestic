// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Texts of retention rules and of the retention preview (`forget --dry-run`).

use crate::state_cache::{format_datetime_display, i18n, i18np};
use crate::ui_texts::GuiText;
use kestic_core::backup::ForgetPreviewResult;
use kestic_core::model::{ForgetReason, RetentionPeriod};
use serde::Serialize;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// A rule «keep … within `duration`»: per `period`, or all snapshots without one.
pub fn within_text(period: Option<RetentionPeriod>, duration: &str) -> String {
    let text = match period {
        None => i18n("all within %1"),
        Some(RetentionPeriod::Hourly) => i18n("hourly within %1"),
        Some(RetentionPeriod::Daily) => i18n("daily within %1"),
        Some(RetentionPeriod::Weekly) => i18n("weekly within %1"),
        Some(RetentionPeriod::Monthly) => i18n("monthly within %1"),
        Some(RetentionPeriod::Yearly) => i18n("yearly within %1"),
    };
    text.subs(&[duration])
}

fn period_reason_text(period: RetentionPeriod) -> String {
    match period {
        RetentionPeriod::Hourly => i18n("hourly snapshot"),
        RetentionPeriod::Daily => i18n("daily snapshot"),
        RetentionPeriod::Weekly => i18n("weekly snapshot"),
        RetentionPeriod::Monthly => i18n("monthly snapshot"),
        RetentionPeriod::Yearly => i18n("yearly snapshot"),
    }
    .to_string()
}

/// Why the preview keeps a snapshot, in the GUI language; a reason Kestic does not know is
/// shown as restic writes it.
pub fn forget_reason_text(raw: &str) -> String {
    let Some(reason) = ForgetReason::parse(raw) else {
        return raw.trim().to_string();
    };
    match reason {
        ForgetReason::Last => i18n("latest snapshot").to_string(),
        ForgetReason::Within(duration) => within_text(None, &duration),
        ForgetReason::Period {
            period,
            within,
            oldest,
        } => {
            let text = match within {
                Some(duration) => within_text(Some(period), &duration),
                None => period_reason_text(period),
            };
            if oldest {
                i18n("%1 (oldest)").subs(&[&text])
            } else {
                text
            }
        }
    }
}

/// One snapshot of the retention preview as the page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RetentionPreviewRow {
    pub short_id: String,
    /// `"keep"` or `"remove"`.
    pub action: String,
    /// Local date and time like everywhere else («2026-11-22 21:13»).
    pub time_text: String,
    /// The reasons for keeping it, separated by commas; empty for a snapshot to remove.
    pub reasons_text: String,
}

/// The retention preview as the page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RetentionPreviewView {
    pub keep_count: usize,
    pub remove_count: usize,
    /// «3 snapshots kept · 2 snapshots would be removed»
    pub summary: String,
    pub snapshots: Vec<RetentionPreviewRow>,
}

fn summary_text(keep: usize, remove: usize) -> String {
    if remove == 0 {
        return i18n("No snapshots would be removed (%1 kept).").subs(&[&keep.to_string()]);
    }
    GuiText::TwoParts.with_args(&[
        &i18np("%1 snapshot kept", "%1 snapshots kept", keep, &[]),
        &i18np(
            "%1 snapshot would be removed",
            "%1 snapshots would be removed",
            remove,
            &[],
        ),
    ])
}

pub fn retention_preview_view(result: &ForgetPreviewResult) -> RetentionPreviewView {
    let snapshots = result
        .snapshots
        .iter()
        .map(|s| RetentionPreviewRow {
            short_id: s.short_id.clone(),
            action: s.action.clone(),
            time_text: OffsetDateTime::parse(&s.time, &Rfc3339)
                .map_or_else(|_| s.time.clone(), format_datetime_display),
            reasons_text: s
                .reasons
                .iter()
                .map(|r| forget_reason_text(r))
                .collect::<Vec<_>>()
                .join(", "),
        })
        .collect();
    RetentionPreviewView {
        keep_count: result.keep_count,
        remove_count: result.remove_count,
        summary: summary_text(result.keep_count, result.remove_count),
        snapshots,
    }
}

#[cfg(test)]
#[path = "retention_texts_tests.rs"]
mod tests;
