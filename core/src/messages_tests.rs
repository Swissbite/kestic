// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Unit tests for the code → msgid mapping (`messages.rs`).

use super::*;

/// AC (review M3, GUI-Sprache, Option A+): the error kinds map to the texts the GUI showed
/// before; `unknown` depends on exit code and warning.
#[test]
fn m3_review3_error_kind_codes_map_to_gui_texts() {
    assert_eq!(
        ErrorKind::WrongPassword.message(Some(12), false).english(),
        "Stored password does not match"
    );
    assert_eq!(
        ErrorKind::RepoMissing.message(Some(10), false).english(),
        "Repository is missing"
    );
    assert_eq!(
        ErrorKind::Locked.message(Some(11), false).english(),
        "Repository is locked"
    );
    assert_eq!(
        ErrorKind::Unknown.message(Some(42), false),
        Message::with_args("Unknown error (exit %1)", &["42"])
    );
    assert_eq!(
        ErrorKind::Unknown.message(Some(1), false).english(),
        "Backup command failed"
    );
    assert_eq!(
        ErrorKind::Unknown.message(None, true).english(),
        "Backed up with warning"
    );
    assert_eq!(
        ErrorKind::IncompleteSnapshot
            .message(Some(3), true)
            .english(),
        "Snapshot incomplete: some files could not be read"
    );
}

#[test]
fn restic_failure_summary_keeps_dns_and_exit_code_detail() {
    assert_eq!(
        ResticFailureKind::DnsResolutionFailed.default_summary(),
        "Destination hostname could not be resolved (DNS)"
    );
    assert_eq!(
        ResticFailureKind::Unknown { exit_code: 99 }.default_summary(),
        "Unknown error (exit 99)"
    );
    assert_eq!(
        ResticFailureKind::WrongPassword.summary_message().msgid,
        "Stored password does not match"
    );
}

#[test]
fn pending_reason_codes_map_to_waiting_lines() {
    assert_eq!(
        PendingReasonCode::OnBattery.waiting_message("").english(),
        "Waiting: AC power"
    );
    assert_eq!(
        PendingReasonCode::Backoff
            .waiting_message("12:00")
            .english(),
        "Waiting: next attempt at 12:00"
    );
    assert_eq!(
        PendingReasonCode::Backoff.waiting_message("").english(),
        "Waiting: next attempt at –"
    );
    assert_eq!(
        PendingReasonCode::OnBattery
            .condition_message(Some(40), &[])
            .english(),
        "AC power (battery 40%)"
    );
    assert_eq!(
        PendingReasonCode::WrongNetwork
            .condition_message(None, &["Home".to_string(), "Office".to_string()])
            .english(),
        "allowed network (Home, Office)"
    );
}

#[test]
fn source_blocked_and_skip_codes_map_to_texts() {
    assert_eq!(
        SourceStatus::message(Some(SourceStatus::DriveNotMounted)).english(),
        "Source unavailable: drive not mounted"
    );
    assert_eq!(
        SourceStatus::message(Some(SourceStatus::PathMissing)).english(),
        "Source unavailable"
    );
    assert_eq!(
        BlockedReason::waiting_message(None, None).english(),
        "Waiting: unavailable"
    );
    assert_eq!(
        BlockedReason::SourceUnavailable
            .manual_block_message("Docs")
            .english(),
        "Cannot back up 'Docs': source is unavailable."
    );
    assert_eq!(
        SkipReason::Error.text(Some(ErrorKind::SecretStoreLocked), Some("raw")),
        UiText::Message(Message::new("Password store is locked"))
    );
    assert_eq!(
        SkipReason::Error.text(Some(ErrorKind::Unknown), Some("raw detail")),
        UiText::Raw("raw detail".to_string())
    );
    assert_eq!(
        SkipReason::PermanentError.text(Some(ErrorKind::WrongPassword), None),
        UiText::Message(Message::new("permanent error, fix required"))
    );
}

#[test]
fn restic_blocking_and_connection_texts() {
    assert_eq!(
        ResticStatusCode::TooOld
            .message(None, Some("0.17.3"), None)
            .english(),
        "restic 0.17.3 is too old; Kestic needs at least 0.18.0"
    );
    assert_eq!(
        restic_status_text(None, Some("0.18.0"), None, None, None),
        UiText::Raw("restic 0.18.0".to_string())
    );
    assert_eq!(
        BlockingKind::ResticUnavailable.message(None).english(),
        "restic is unavailable or too old."
    );
    assert_eq!(
        connection_test_text(false, Some(ConnectionTestCode::DuplicateLocation), "NAS"),
        UiText::Message(Message::with_args(
            "This location is already used by destination '%1'.",
            &["NAS"]
        ))
    );
    assert_eq!(
        connection_test_text(false, Some(ConnectionTestCode::WrongPassword), "raw"),
        UiText::Raw("raw".to_string())
    );
    assert_eq!(
        connection_test_text(true, None, ""),
        UiText::Message(Message::new("Reachable"))
    );
}

fn counts(kind: OverallStatusKind) -> OverallCounts {
    OverallCounts {
        kind,
        failed: 0,
        overdue: 0,
        warning: 0,
        waiting: 0,
        backups: 3,
        paused: false,
    }
}

#[test]
fn overall_headline_follows_the_priority_order() {
    let run = ActiveRunInfo {
        name: "Z",
        is_maintenance: false,
        percent_done: 45.6,
    };
    let maint = ActiveRunInfo {
        name: "NAS",
        is_maintenance: true,
        percent_done: 0.0,
    };
    let mut c = counts(OverallStatusKind::Error);
    c.failed = 2;
    c.overdue = 1;
    assert_eq!(
        overall_headline(&c, Some(&run), None).english(),
        "2 backups failed"
    );
    assert_eq!(
        overall_subtitle(&c, Some(&run)).map(|m| m.english()),
        Some("Currently running: Z (46%)".to_string())
    );
    assert_eq!(
        overall_subtitle(&c, Some(&maint)).map(|m| m.english()),
        Some("Maintaining NAS".to_string())
    );
    assert_eq!(
        overall_headline(&c, None, Some(Message::new("blocked"))).english(),
        "blocked"
    );
    let running = counts(OverallStatusKind::Running);
    assert_eq!(
        overall_headline(&running, Some(&run), None).english(),
        "Z: backing up · 46%"
    );
    assert_eq!(overall_subtitle(&running, Some(&run)), None);
    let mut waiting = counts(OverallStatusKind::Pending);
    waiting.waiting = 2;
    assert_eq!(
        overall_headline(&waiting, None, None).english(),
        "2 of 3 backups waiting"
    );
    waiting.paused = true;
    assert_eq!(
        overall_headline(&waiting, None, None).english(),
        "Schedules paused · 2 backups waiting"
    );
    assert_eq!(
        overall_headline(&counts(OverallStatusKind::Paused), None, None).english(),
        "Schedules paused"
    );
    let mut empty = counts(OverallStatusKind::NeverRun);
    empty.backups = 0;
    assert_eq!(
        overall_headline(&empty, None, None).english(),
        "No backups configured yet"
    );
    assert_eq!(
        overall_headline(&counts(OverallStatusKind::Interrupted), None, None).english(),
        "Last run interrupted"
    );
    assert_eq!(
        overall_headline(&counts(OverallStatusKind::Ok), None, None).english(),
        "All data backed up"
    );
}

#[test]
fn placeholders_are_substituted_once() {
    let m = Message::with_args("%1 · %2 (%1%) %3", &["a%2", "b"]);
    assert_eq!(m.english(), "a%2 · b (a%2%) %3");
}
