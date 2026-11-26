// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Plural-aware daemon texts: notifications with a count use the singular for one.

use kestic_daemon::i18n::{
    CallbackSubsTranslator, PassthroughTranslator, i18np, set_daemon_translator,
};
use std::sync::Arc;

/// Without a catalog `i18np` picks (a) the singular for 1 and (b) the plural
/// otherwise, with `%1` as the count and (c) further placeholders from the arguments; (d) an
/// installed translator receives both forms and the count.
#[test]
fn finding2_daemon_i18np_uses_singular_for_one() {
    assert_eq!(
        i18np(
            "%1 scheduled backup missed",
            "%1 scheduled backups missed",
            1,
            &[]
        ),
        "1 scheduled backup missed",
        "(a)"
    );
    assert_eq!(
        i18np(
            "%1 scheduled backup missed",
            "%1 scheduled backups missed",
            3,
            &[]
        ),
        "3 scheduled backups missed",
        "(b)"
    );
    assert_eq!(
        i18np("%2: %1 snapshot", "%2: %1 snapshots", 2, &["NAS"]),
        "NAS: 2 snapshots",
        "(c)"
    );

    set_daemon_translator(Arc::new(CallbackSubsTranslator {
        translate_fn: |msgid: &str| msgid.to_string(),
        translate_subs_fn: |msgid: &str, _args: &[&str]| msgid.to_string(),
        translate_plural_fn: |singular: &str, plural: &str, n: u64, _args: &[&str]| {
            format!("{singular}|{plural}|{n}")
        },
        format_count_fn: |n: i64| n.to_string(),
    }));
    assert_eq!(
        i18np(
            "%1 scheduled backup missed",
            "%1 scheduled backups missed",
            1,
            &[]
        ),
        "%1 scheduled backup missed|%1 scheduled backups missed|1",
        "(d)"
    );
    set_daemon_translator(Arc::new(PassthroughTranslator));
}
