// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Percentages in messages (#203): a message carries the percentage as a number, and each side
//! formats it with the catalog's percent pattern, so German gets «25 %» and English «25%».

use kestic_i18n::codes::{PendingReasonCode, RunKindCode, RunProgressPhase};
use kestic_i18n::messages::{
    ActiveRunInfo, MessageArg, PERCENT_PATTERN, active_run_message, english_percent,
};

fn backing_up(percent_done: f64) -> ActiveRunInfo<'static> {
    ActiveRunInfo {
        name: "Docs",
        kind: RunKindCode::Backup,
        phase: RunProgressPhase::ScanFinished,
        percent_done,
    }
}

/// #203: (a) the percentage of an active run and of the battery is a [`MessageArg::Percent`],
/// not text with a percent sign in the msgid, (b) the side that shows the message formats it,
/// (c) the English text keeps «25%».
#[test]
fn m5_203_percent_is_formatted_by_the_side_that_shows_it() {
    let run = active_run_message(&backing_up(24.6));
    let battery = PendingReasonCode::OnBattery.condition_message(Some(40), &[]);
    // (a)
    assert_eq!(run.args.last(), Some(&MessageArg::Percent(25)), "(a) run");
    assert_eq!(battery.args, vec![MessageArg::Percent(40)], "(a) battery");
    assert_eq!(run.msgid, "%1: backing up · %2", "(a) run msgid");
    assert_eq!(battery.msgid, "AC power (battery %1)", "(a) battery msgid");
    // (b)
    assert_eq!(
        run.rendered_args(|p| format!("{p}\u{a0}%")),
        vec!["Docs".to_string(), "25\u{a0}%".to_string()],
        "(b)"
    );
    // (c)
    assert_eq!(PERCENT_PATTERN, "%1%", "(c) pattern");
    assert_eq!(english_percent(25), "25%", "(c) percent");
    assert_eq!(run.english(), "Docs: backing up · 25%", "(c) run");
    assert_eq!(battery.english(), "AC power (battery 40%)", "(c) battery");
}
