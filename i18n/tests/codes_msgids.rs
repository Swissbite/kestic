// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Every language-independent code (`docs/spec/dbus-api.md` §Sprachunabhängige Codes) maps to
//! a non-empty English msgid with a German translation in `po/de/kestic.po`, and every message
//! fills exactly the placeholders of its msgid.

use kestic_i18n::codes::{
    BlockedReason, BlockingKind, ConnectionNoteCode, ConnectionTestCode, ErrorKind,
    OverallStatusKind, PendingReasonCode, RecoveryAction, ResticInstallCode, ResticInstallOutcome,
    ResticInstallPhase, ResticStatusCode, RestoreNoteCode, RestoreOutcome, RestoreTargetCode,
    RunKindCode, RunProgressPhase, ServiceErrorCode, SkipReason, SourceStatus,
};
use kestic_i18n::messages::{
    ActiveRunInfo, Message, OverallCounts, ResticStatusParams, UiText, active_run_message,
    completed_message, connection_test_text, dns_failed_message, nothing_restored_message,
    overall_headline, overall_subtitle, unmet_conditions_message,
};
use kestic_test_support::po::catalog;
use kestic_test_support::workspace::workspace_root;
use std::collections::{BTreeSet, HashMap};

/// msgid -> German translation of the singular catalog entries.
fn german_catalog() -> HashMap<String, String> {
    catalog(&workspace_root())
        .into_iter()
        .filter(|e| !e.plural)
        .map(|e| (e.msgid().to_string(), e.german()[0].clone()))
        .collect()
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
    push("completed".into(), completed_message());
    for phase in ResticInstallPhase::ALL {
        push(format!("{phase:?}"), phase.message());
    }
    for outcome in ResticInstallOutcome::ALL {
        push(format!("{outcome:?}"), outcome.message("0.20.0"));
    }
    for code in ResticInstallCode::ALL {
        push(format!("install/{code:?}"), code.message());
    }
    push("dns".into(), dns_failed_message());
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
            match reason.text(kind, None) {
                UiText::Message(m) => push(format!("{reason:?}/{kind:?}"), m),
                UiText::Raw(raw) => panic!("{reason:?}/{kind:?}: raw text {raw:?} without detail"),
            }
        }
    }
    for code in ResticStatusCode::ALL {
        push(
            format!("{code:?}"),
            code.message(&ResticStatusParams {
                binary_path: Some("/p"),
                found_version: Some("0.1"),
                detail: Some("d"),
                min_version: "0.18.0",
            }),
        );
    }
    for kind in BlockingKind::ALL {
        push(format!("{kind:?}"), kind.message(None));
    }
    // Codes whose detail is the information itself; every other code must have a msgid.
    let raw_codes = [
        ConnectionTestCode::InvalidInput,
        ConnectionTestCode::Internal,
        ConnectionTestCode::CommandFailed,
        ConnectionTestCode::DaemonUnreachable,
    ];
    for code in ConnectionTestCode::ALL {
        match connection_test_text(false, Some(code), "x") {
            UiText::Message(m) => push(format!("{code:?}"), m),
            UiText::Raw(_) => assert!(raw_codes.contains(&code), "{code:?} must be translated"),
        }
    }
    for note in ConnectionNoteCode::ALL {
        push(format!("{note:?}"), note.message("x"));
    }
    if let UiText::Message(m) = connection_test_text(true, None, "") {
        push("reachable".into(), m);
    }
    for code in ServiceErrorCode::ALL {
        push(format!("{code:?}"), code.message());
    }
    for outcome in RestoreOutcome::ALL {
        push(format!("{outcome:?}"), outcome.message());
    }
    for note in RestoreNoteCode::ALL {
        push(format!("{note:?}"), note.message());
    }
    for code in RestoreTargetCode::ALL {
        if let Some(m) = code.message("/t") {
            push(format!("{code:?}"), m);
        }
    }
    push("nothing_restored".into(), nothing_restored_message());
    out.extend(overall_messages());
    out
}

/// Headline, subtitle and active-run messages of the overall status.
fn overall_messages() -> Vec<(String, Message)> {
    let mut out: Vec<(String, Message)> = Vec::new();
    let mut push = |label: String, m: Message| out.push((label, m));
    for (name, kind, phase) in [
        ("run", RunKindCode::Backup, RunProgressPhase::ScanFinished),
        ("scan", RunKindCode::Backup, RunProgressPhase::Scanning),
        (
            "maint",
            RunKindCode::Maintenance,
            RunProgressPhase::Scanning,
        ),
        ("restore", RunKindCode::Restore, RunProgressPhase::Scanning),
        (
            "restoring",
            RunKindCode::Restore,
            RunProgressPhase::ScanFinished,
        ),
    ] {
        let run = ActiveRunInfo {
            name,
            kind,
            phase,
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

/// Every code variant has a non-empty msgid with a German translation, and msgid, translation
/// and arguments agree on the placeholders.
#[test]
fn every_code_maps_to_a_translated_msgid() {
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
