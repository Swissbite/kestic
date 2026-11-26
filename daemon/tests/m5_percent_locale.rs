// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Percentages in the daemon's texts (tray, notifications, #203): the daemon formats them like
//! the GUI, with the number from the translator's locale and the percent pattern of the catalog.

use kestic_daemon::i18n::{
    CallbackSubsTranslator, PassthroughTranslator, format_percent, set_daemon_translator, tr,
};
use kestic_i18n::codes::{PendingReasonCode, RunKindCode, RunProgressPhase};
use kestic_i18n::messages::{ActiveRunInfo, active_run_message, substitute_placeholders};
use kestic_test_support::po::{catalog, translations};
use kestic_test_support::workspace::workspace_root;
use std::sync::Arc;

/// #203: with the German catalog (a) a percentage reads «25 %» with a no-break space, in the
/// tray text of a backup and a restore and in the battery condition, and (b) its number comes
/// from the translator's locale; (c) without a catalog it reads «25%».
#[test]
fn m5_203_daemon_percent_follows_the_language() {
    let backup = active_run_message(&ActiveRunInfo {
        name: "Docs",
        kind: RunKindCode::Backup,
        phase: RunProgressPhase::ScanFinished,
        percent_done: 25.0,
    });
    let restore = active_run_message(&ActiveRunInfo {
        name: "Docs",
        kind: RunKindCode::Restore,
        phase: RunProgressPhase::ScanFinished,
        percent_done: 25.0,
    });
    let battery = PendingReasonCode::OnBattery.condition_message(Some(40), &[]);

    // (c)
    set_daemon_translator(Arc::new(PassthroughTranslator));
    assert_eq!(tr(&backup), "Docs: backing up · 25%", "(c) backup");
    assert_eq!(tr(&battery), "AC power (battery 40%)", "(c) battery");

    let german = Arc::new(translations(&catalog(&workspace_root())));
    let lookup = Arc::clone(&german);
    let lookup_subs = Arc::clone(&german);
    set_daemon_translator(Arc::new(CallbackSubsTranslator {
        translate_fn: move |msgid: &str| lookup.get(msgid).cloned().unwrap_or(msgid.to_string()),
        translate_subs_fn: move |msgid: &str, args: &[&str]| {
            let text = lookup_subs.get(msgid).cloned().unwrap_or(msgid.to_string());
            let args: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();
            substitute_placeholders(&text, &args)
        },
        translate_plural_fn: |singular: &str, _: &str, _: u64, _: &[&str]| singular.to_string(),
        // Arabic-Indic digits show that the number comes from the locale.
        format_count_fn: |n: i64| {
            n.to_string()
                .chars()
                .map(|c| char::from_u32(0x0660 + c.to_digit(10).unwrap_or(0)).unwrap_or(c))
                .collect()
        },
    }));
    // (a)
    assert_eq!(format_percent(7), "\u{667}\u{a0}%", "(a) format_percent");
    assert_eq!(
        tr(&backup),
        "Docs: sichert · \u{662}\u{665}\u{a0}%",
        "(a) backup"
    );
    assert_eq!(
        tr(&restore),
        "Docs: stellt wieder her · \u{662}\u{665}\u{a0}%",
        "(a) restore"
    );
    assert_eq!(
        tr(&battery),
        "Netzstrom (Akku \u{664}\u{660}\u{a0}%)",
        "(a) battery"
    );
    // (b)
    assert!(
        !tr(&backup).contains("25"),
        "(b) the number is not the plain digits"
    );
    set_daemon_translator(Arc::new(PassthroughTranslator));
}
