// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Variant A+ (decision 2026-11-20, `docs/spec/dbus-api.md` §Sprachunabhängige Codes): every
//! language-independent code maps to a non-empty English msgid with a German translation in
//! `po/de/kestic.po`, and every message fills exactly the placeholders of its msgid.

use kestic_core::codes::{
    BlockedReason, BlockingKind, ConnectionTestCode, ErrorKind, OverallStatusKind,
    PendingReasonCode, ResticStatusCode, SkipReason, SourceStatus,
};
use kestic_core::messages::{
    ActiveRunInfo, Message, OverallCounts, UiText, active_run_message, connection_test_text,
    overall_headline, overall_subtitle, unmet_conditions_message,
};
use kestic_core::restic::{RecoveryAction, ResticFailureKind};
use std::collections::{BTreeSet, HashMap};
use std::path::Path;

/// Decodes one PO string literal line (`"..."`).
fn po_literal(line: &str) -> String {
    let inner = line
        .trim()
        .strip_prefix('"')
        .and_then(|l| l.strip_suffix('"'))
        .unwrap_or_else(|| panic!("malformed PO string line: {line}"));
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// msgid → msgstr of the German catalog (singular entries; plural entries are skipped).
fn german_catalog() -> HashMap<String, String> {
    let po = Path::new(env!("CARGO_MANIFEST_DIR")).join("../po/de/kestic.po");
    let text = std::fs::read_to_string(&po).expect("read po/de/kestic.po");
    let mut catalog = HashMap::new();
    let (mut msgid, mut msgstr) = (None::<String>, None::<String>);
    let mut current: Option<&mut String> = None;
    let mut flush = |id: &mut Option<String>, s: &mut Option<String>| {
        if let (Some(i), Some(t)) = (id.take(), s.take()) {
            catalog.insert(i, t);
        }
    };
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("msgid ") {
            flush(&mut msgid, &mut msgstr);
            msgid = Some(po_literal(rest));
            current = msgid.as_mut();
        } else if let Some(rest) = line.strip_prefix("msgstr ") {
            msgstr = Some(po_literal(rest));
            current = msgstr.as_mut();
        } else if line.starts_with('"') {
            if let Some(buf) = current.as_mut() {
                buf.push_str(&po_literal(line));
            }
        } else if line.starts_with("msgid_plural") || line.starts_with("msgstr[") {
            msgid = None;
            msgstr = None;
            current = None;
        }
    }
    flush(&mut msgid, &mut msgstr);
    catalog
}

/// Distinct `%1`..`%9` placeholders of a template.
fn placeholders(template: &str) -> BTreeSet<u32> {
    let chars: Vec<char> = template.chars().collect();
    chars
        .windows(2)
        .filter(|w| w[0] == '%')
        .filter_map(|w| w[1].to_digit(10).filter(|d| *d > 0))
        .collect()
}

/// Every message the code → msgid mapping can produce (all variants, all branches).
fn all_messages() -> Vec<(String, Message)> {
    let mut out: Vec<(String, Message)> = Vec::new();
    let mut push = |label: String, m: Message| out.push((label, m));
    for kind in ErrorKind::ALL {
        for (exit, warning) in [
            (None, false),
            (Some(1), false),
            (Some(42), false),
            (None, true),
        ] {
            push(
                format!("{kind:?}/{exit:?}/{warning}"),
                kind.message(exit, warning),
            );
        }
    }
    for kind in [
        ResticFailureKind::None,
        ResticFailureKind::Cancelled,
        ResticFailureKind::IncompleteSnapshot {
            unreadable_items: Vec::new(),
        },
        ResticFailureKind::DestinationUnreachable,
        ResticFailureKind::DnsResolutionFailed,
        ResticFailureKind::BackendAuthFailed,
        ResticFailureKind::GenericFailure,
        ResticFailureKind::RepositoryMissing,
        ResticFailureKind::RepositoryLocked,
        ResticFailureKind::WrongPassword,
        ResticFailureKind::Unknown { exit_code: 99 },
    ] {
        push(format!("{kind:?}"), kind.summary_message());
    }
    for action in RecoveryAction::ALL {
        push(format!("{action:?}"), action.label_message());
    }
    let allowed = vec!["Home".to_string()];
    for code in PendingReasonCode::ALL {
        push(format!("{code:?}/wait"), code.waiting_message("12:00"));
        push(format!("{code:?}/wait-"), code.waiting_message(""));
        push(
            format!("{code:?}/cond"),
            code.condition_message(Some(40), &allowed),
        );
        push(
            format!("{code:?}/cond-"),
            code.condition_message(None, &allowed),
        );
    }
    push("unmet".into(), unmet_conditions_message("x"));
    for status in SourceStatus::ALL.map(Some).into_iter().chain([None]) {
        push(format!("{status:?}"), SourceStatus::message(status));
    }
    for reason in BlockedReason::ALL {
        push(format!("{reason:?}"), reason.message(None));
        push(
            format!("{reason:?}/manual"),
            reason.manual_block_message("B"),
        );
        push(
            format!("{reason:?}/wait"),
            BlockedReason::waiting_message(Some(reason), None),
        );
    }
    push(
        "blocked/none".into(),
        BlockedReason::waiting_message(None, None),
    );
    for reason in SkipReason::ALL {
        for kind in ErrorKind::ALL.map(Some).into_iter().chain([None]) {
            if let UiText::Message(m) = reason.text(kind, None) {
                push(format!("{reason:?}/{kind:?}"), m);
            }
        }
    }
    for code in ResticStatusCode::ALL {
        push(
            format!("{code:?}"),
            code.message(Some("/p"), Some("0.1"), Some("d")),
        );
    }
    for kind in BlockingKind::ALL {
        push(format!("{kind:?}"), kind.message(None));
    }
    for code in ConnectionTestCode::ALL {
        if let UiText::Message(m) = connection_test_text(false, Some(code), "x") {
            push(format!("{code:?}"), m);
        }
    }
    if let UiText::Message(m) = connection_test_text(true, None, "") {
        push("reachable".into(), m);
    }
    for (name, is_maintenance) in [("run", false), ("maint", true)] {
        let run = ActiveRunInfo {
            name,
            is_maintenance,
            percent_done: 50.0,
        };
        push(format!("active/{name}"), active_run_message(&run));
        let failing = OverallCounts {
            kind: OverallStatusKind::Error,
            failed: 1,
            overdue: 0,
            warning: 0,
            waiting: 0,
            backups: 2,
            paused: false,
        };
        if let Some(m) = overall_subtitle(&failing, Some(&run)) {
            push(format!("subtitle/{name}"), m);
        }
    }
    for kind in [
        OverallStatusKind::Error,
        OverallStatusKind::Overdue,
        OverallStatusKind::Warning,
        OverallStatusKind::Pending,
        OverallStatusKind::Paused,
        OverallStatusKind::Interrupted,
        OverallStatusKind::Cancelled,
        OverallStatusKind::NeverRun,
        OverallStatusKind::Ok,
    ] {
        for n in [1, 3] {
            for paused in [false, true] {
                for backups in [0, 4] {
                    let c = OverallCounts {
                        kind,
                        failed: usize::from(kind == OverallStatusKind::Error) * n,
                        overdue: usize::from(kind == OverallStatusKind::Overdue) * n,
                        warning: usize::from(kind == OverallStatusKind::Warning) * n,
                        waiting: usize::from(kind == OverallStatusKind::Pending) * n,
                        backups,
                        paused,
                    };
                    push(
                        format!("headline/{kind:?}/{n}/{paused}/{backups}"),
                        overall_headline(&c, None, None),
                    );
                }
            }
        }
    }
    out
}

/// A+ (2026-11-20): every code variant has a non-empty msgid with a German translation, and
/// msgid, translation and arguments agree on the placeholders.
#[test]
fn a_plus_every_code_maps_to_a_translated_msgid() {
    let catalog = german_catalog();
    let messages = all_messages();
    assert!(
        messages.len() > 100,
        "expected all branches, got {}",
        messages.len()
    );
    for (label, m) in &messages {
        assert!(!m.msgid.trim().is_empty(), "{label}: empty msgid");
        let translation = catalog
            .get(m.msgid)
            .unwrap_or_else(|| panic!("{label}: msgid {:?} missing in po/de/kestic.po", m.msgid));
        assert!(
            !translation.trim().is_empty(),
            "{label}: msgid {:?} has no German translation",
            m.msgid
        );
        let wanted = placeholders(m.msgid);
        assert_eq!(
            placeholders(translation),
            wanted,
            "{label}: translation of {:?} must use the same placeholders",
            m.msgid
        );
        let filled: BTreeSet<u32> = (1..=u32::try_from(m.args.len()).expect("few args")).collect();
        assert_eq!(filled, wanted, "{label}: arguments must fill {:?}", m.msgid);
    }
}
