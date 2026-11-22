// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Tests of the code → msgid mapping (`kestic_i18n::messages`).

use kestic_i18n::codes::{
    BlockedReason, BlockingKind, ConnectionTestCode, ErrorKind, OverallStatusKind,
    PendingReasonCode, RecoveryAction, ResticStatusCode, RunKindCode, RunProgressPhase,
    ServiceErrorCode, SkipReason, SourceStatus,
};
use kestic_i18n::messages::{
    ActiveRunInfo, Message, OverallCounts, ResticStatusParams, UiText, active_run_message,
    connection_test_text, overall_headline, overall_subtitle, restic_status_text, rounded_percent,
};

/// `unknown` depends on exit code and warning; all other kinds ignore both
/// (see `every_error_kind_maps_to_its_msgid`).
#[test]
fn unknown_error_kind_message_depends_on_exit_code_and_warning() {
    assert_eq!(
        ErrorKind::Unknown.message(Some(42), false),
        Message::with_args("Unknown error (code %1)", &["42"])
    );
    assert_eq!(
        ErrorKind::Unknown.message(Some(1), false).english(),
        "Backup command failed"
    );
    assert_eq!(
        ErrorKind::Unknown.message(None, true).english(),
        "Backed up with warning"
    );
}

#[test]
fn pending_reason_codes_map_to_waiting_lines() {
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
        UiText::Message(Message::new("Permanent error, needs fixing"))
    );
}

#[test]
fn restic_blocking_and_connection_texts() {
    assert_eq!(
        ResticStatusCode::TooOld
            .message(&ResticStatusParams {
                found_version: Some("0.17.3"),
                min_version: "0.18.0",
                ..ResticStatusParams::default()
            })
            .english(),
        "restic 0.17.3 is too old; Kestic needs at least 0.18.0"
    );
    assert_eq!(
        restic_status_text(None, Some("0.18.0"), &ResticStatusParams::default()),
        UiText::Raw("restic 0.18.0".to_string())
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
        UiText::Message(Message::new("Saved password does not match"))
    );
    assert_eq!(
        connection_test_text(false, Some(ConnectionTestCode::CommandFailed), "raw"),
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
        kind: RunKindCode::Backup,
        phase: RunProgressPhase::ScanFinished,
        percent_done: 45.6,
    };
    let maint = ActiveRunInfo {
        name: "NAS",
        kind: RunKindCode::Maintenance,
        phase: RunProgressPhase::Scanning,
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

/// Checks that `table` lists every variant of `all` exactly once and that each variant maps to
/// its msgid (`None` = shown as raw detail).
fn assert_table<T>(all: &[T], table: &[(T, Option<&str>)], text: impl Fn(T) -> UiText)
where
    T: Copy + std::fmt::Debug + PartialEq,
{
    assert_eq!(table.len(), all.len(), "table must list every variant once");
    for variant in all {
        let (_, expected) = table
            .iter()
            .find(|(v, _)| v == variant)
            .unwrap_or_else(|| panic!("{variant:?} missing in table"));
        let actual = match text(*variant) {
            UiText::Message(m) => Some(m.msgid),
            UiText::Raw(_) => None,
        };
        assert_eq!(actual, *expected, "{variant:?}");
    }
}

fn msg(m: Message) -> UiText {
    UiText::Message(m)
}

#[test]
fn every_error_kind_maps_to_its_msgid() {
    use ErrorKind as E;
    let table = [
        (E::WrongPassword, Some("Saved password does not match")),
        (E::RepoMissing, Some("Repository is missing")),
        (E::Locked, Some("Repository is locked")),
        (E::DestinationUnreachable, Some("Destination unreachable")),
        (E::SourceUnavailable, Some("Source unavailable")),
        (E::AuthFailed, Some("Login to the destination failed")),
        (
            E::IncompleteSnapshot,
            Some("Snapshot incomplete: some files could not be read"),
        ),
        (
            E::RetentionPending,
            Some("Backed up, retention not applied yet"),
        ),
        (E::Cancelled, Some("Cancelled by user")),
        (E::SecretStoreLocked, Some("Password store is locked")),
        (
            E::ResticUnavailable,
            Some("restic is unavailable or too old."),
        ),
        (
            E::PathNotInSnapshot,
            Some("The chosen folder does not exist in the snapshot"),
        ),
        (E::Unknown, Some("Backup command failed")),
    ];
    assert_table(&ErrorKind::ALL, &table, |k| msg(k.message(None, false)));
    // `text`: only `unknown` with a detail shows the detail raw.
    let raw_unknown = table.map(|(k, m)| (k, if k == E::Unknown { None } else { m }));
    assert_table(&ErrorKind::ALL, &raw_unknown, |k| k.text(Some("raw")));
}

#[test]
fn every_pending_reason_maps_to_its_msgids() {
    use PendingReasonCode as P;
    let waiting = [
        (P::SourceUnavailable, Some("Waiting: source unavailable")),
        (
            P::DestinationUnreachable,
            Some("Waiting: destination unreachable"),
        ),
        (P::OnBattery, Some("Waiting: AC power")),
        (P::NoNetwork, Some("Waiting: network")),
        (P::Metered, Some("Waiting: unmetered connection")),
        (P::WrongNetwork, Some("Waiting: allowed network")),
        (P::Queue, Some("Waiting: another run on this destination")),
        (P::Backoff, Some("Waiting: next attempt at %1")),
    ];
    assert_table(&P::ALL, &waiting, |c| msg(c.waiting_message("")));
    let condition = [
        (P::SourceUnavailable, Some("source available")),
        (P::DestinationUnreachable, Some("destination reachable")),
        (P::OnBattery, Some("AC power")),
        (P::NoNetwork, Some("network connection")),
        (P::Metered, Some("unmetered connection")),
        (P::WrongNetwork, Some("allowed network (%1)")),
        (P::Queue, Some("another run on this destination")),
        (P::Backoff, Some("waiting time before next attempt")),
    ];
    assert_table(&P::ALL, &condition, |c| msg(c.condition_message(None, &[])));
}

#[test]
fn every_source_blocked_and_skip_code_maps_to_its_msgid() {
    use SourceStatus as S;
    let source = [
        (S::PathMissing, Some("Source unavailable")),
        (
            S::DriveNotMounted,
            Some("Source unavailable: drive not mounted"),
        ),
        (
            S::DriveMismatch,
            Some("Source unavailable: different drive"),
        ),
        (S::DriveEmpty, Some("Source unavailable: folder is empty")),
    ];
    assert_table(&S::ALL, &source, |s| msg(SourceStatus::message(Some(s))));

    use BlockedReason as B;
    let reason = [
        (B::SourceUnavailable, Some("Source unavailable")),
        (B::DestinationUnreachable, Some("Destination unreachable")),
        (B::ConditionNotMet, Some("Run conditions not met")),
    ];
    assert_table(&B::ALL, &reason, |r| msg(r.message(None)));
    let manual = [
        (
            B::SourceUnavailable,
            Some("Cannot back up '%1': source is unavailable."),
        ),
        (
            B::DestinationUnreachable,
            Some("Cannot back up '%1': destination is unreachable."),
        ),
        (
            B::ConditionNotMet,
            Some("Cannot back up '%1': run conditions are not met."),
        ),
    ];
    assert_table(&B::ALL, &manual, |r| msg(r.manual_block_message("B")));

    use SkipReason as K;
    let skip = [
        (K::PermanentError, Some("Permanent error, needs fixing")),
        (K::SourceUnavailable, Some("Source unavailable")),
        (K::DestinationUnreachable, Some("Destination unreachable")),
        (K::ConditionNotMet, Some("Run conditions not met")),
        (K::Error, Some("Backup command failed")),
    ];
    assert_table(&K::ALL, &skip, |r| r.text(None, None));
}

#[test]
fn every_restic_blocking_connection_and_action_code_maps_to_its_msgid() {
    use ResticStatusCode as R;
    let restic = [
        (R::NotFound, Some("restic not found; at least %1 required")),
        (
            R::TooOld,
            Some("restic %1 is too old; Kestic needs at least %2"),
        ),
        (
            R::InvalidCustomPath,
            Some("Custom restic path is invalid (%1): %2"),
        ),
        (
            R::UnreadableVersion,
            Some("Cannot read the restic version (%1)"),
        ),
    ];
    assert_table(&R::ALL, &restic, |c| {
        msg(c.message(&ResticStatusParams::default()))
    });

    let blocking = [
        (
            BlockingKind::SecretStoreLocked,
            Some(
                "The system password store (Secret Service / KWallet) is locked. Kestic cannot access destinations.",
            ),
        ),
        (
            BlockingKind::ResticUnavailable,
            Some("restic is unavailable or too old."),
        ),
    ];
    assert_table(&BlockingKind::ALL, &blocking, |k| msg(k.message(None)));

    use ConnectionTestCode as C;
    let connection = [
        (C::InvalidInput, None),
        (
            C::DuplicateLocation,
            Some("This location is already used by destination '%1'."),
        ),
        (C::DestinationUnreachable, Some("Destination unreachable")),
        (
            C::RepositoryExists,
            Some(
                "This folder already contains a restic repository. Choose 'Connect existing repository' instead.",
            ),
        ),
        (
            C::NotWritable,
            Some("Kestic cannot write to the folder: %1"),
        ),
        (
            C::MissingPassword,
            Some("No repository password entered or saved."),
        ),
        (C::SecretStoreLocked, Some("Password store is locked")),
        (C::Internal, None),
        (
            C::ResticUnavailable,
            Some("restic is unavailable or too old."),
        ),
        (C::WrongPassword, Some("Saved password does not match")),
        (C::AuthFailed, Some("Login to the destination failed")),
        (C::RepositoryMissing, Some("Repository is missing")),
        (C::CommandFailed, None),
        (C::DaemonUnreachable, None),
    ];
    assert_table(&C::ALL, &connection, |c| c.text("x"));

    use RecoveryAction as A;
    let actions = [
        (A::UpdateStoredPassword, Some("Update Saved Password…")),
        (A::AddToExcludes, Some("Add to Exclude Rules…")),
        (A::CheckResticSource, Some("Configure restic Source…")),
        (A::UnlockRepository, Some("Release Lock…")),
    ];
    assert_table(&A::ALL, &actions, |a| msg(a.label_message()));
}

#[test]
fn every_service_error_code_maps_to_its_msgid() {
    use ServiceErrorCode as S;
    let table = [
        (S::InvalidInput, Some("Invalid input.")),
        (S::NotFound, Some("The item no longer exists.")),
        (
            S::InUse,
            Some("Remove or reassign all backups on this destination first."),
        ),
        (
            S::ManualRunBlocked,
            Some("Backup could not start because source or destination is unavailable."),
        ),
        (S::DestinationUnreachable, Some("Destination unreachable")),
        (
            S::WrongPassword,
            Some("Wrong password: the repository rejected this password."),
        ),
        (S::AuthFailed, Some("Login to the destination failed")),
        (S::SecretStoreLocked, Some("Password store is locked")),
        (
            S::ResticUnavailable,
            Some("restic is unavailable or too old."),
        ),
        (
            S::AlreadyRunning,
            Some("Another run is already active on this destination."),
        ),
        (
            S::LockInUse,
            Some("Cannot release the lock: another program still uses the repository."),
        ),
        (
            S::DuplicateLocation,
            Some("This location is already used by another destination."),
        ),
        (
            S::PreviewRequired,
            Some("Check the preview again before restoring; it is missing or outdated."),
        ),
        (
            S::Internal,
            Some("Internal error in the background service."),
        ),
        (
            S::DaemonUnreachable,
            Some("Background service unavailable."),
        ),
    ];
    assert_table(&S::ALL, &table, |c| msg(c.message()));
}

/// While restic still finds files, headline, tray, progress view and subtitle name no
/// percentage (`docs/spec/ui/hauptbereiche.md`, restic-integration «Fortschritt beim Sichern»).
#[test]
fn active_run_while_scanning_shows_no_percentage() {
    let scanning = ActiveRunInfo {
        name: "Z",
        kind: RunKindCode::Backup,
        phase: RunProgressPhase::Scanning,
        percent_done: 99.0,
    };
    let finished = ActiveRunInfo {
        phase: RunProgressPhase::ScanFinished,
        ..scanning
    };
    let mut c = counts(OverallStatusKind::Error);
    c.failed = 1;
    // (a) headline / tray / progress view text while scanning
    assert_eq!(
        active_run_message(&scanning).english(),
        "Z: backing up · looking through files"
    );
    // (b) subtitle next to failed backups while scanning
    assert_eq!(
        overall_subtitle(&c, Some(&scanning)).map(|m| m.english()),
        Some("Currently running: Z".to_string())
    );
    // (c) after the scan the percentage is back
    assert_eq!(
        active_run_message(&finished).english(),
        "Z: backing up · 99%"
    );
}

#[test]
fn rounded_percent_stays_between_zero_and_hundred() {
    assert_eq!(rounded_percent(42.5), 43);
    assert_eq!(rounded_percent(99.4), 99);
    assert_eq!(rounded_percent(99.5), 100);
    assert_eq!(rounded_percent(-3.0), 0);
    assert_eq!(rounded_percent(250.0), 100);
    assert_eq!(rounded_percent(f64::NAN), 0);
}
