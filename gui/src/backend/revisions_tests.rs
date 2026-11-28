// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

/// AC-M6-36-10 (d), (e): a page that only missed saves of this window sends the daemon's current
/// revision, also over several saves; a page that missed a change made elsewhere keeps its own,
/// so the daemon refuses it.
#[test]
fn ac_m6_36_10_d_e_own_saves_lead_to_the_current_revision() {
    record_own("rev-a1", "rev-a2");
    record_own("rev-a2", "rev-a3");
    assert_eq!(
        sent_revision("rev-a1", "rev-a3"),
        "rev-a3",
        "(d) two saves in a row"
    );
    assert_eq!(
        sent_revision("rev-a2", "rev-a3"),
        "rev-a3",
        "(e) a page opened after a save"
    );
    assert_eq!(
        sent_revision("rev-a1", "rev-foreign"),
        "rev-a1",
        "a change made elsewhere"
    );
    assert_eq!(
        sent_revision("rev-a3", "rev-a3"),
        "rev-a3",
        "nothing missed"
    );
    record_own("", "rev-x");
    assert_eq!(
        sent_revision("", "rev-x"),
        "",
        "an empty revision stays empty"
    );
}

/// Revisions come back when a change is undone; own saves in a cycle still find the current one,
/// and a change undone elsewhere does not lead to a revision the daemon no longer holds.
#[test]
fn ac_m6_36_10_cycles_and_undone_changes() {
    record_own("rev-c1", "rev-c2");
    record_own("rev-c2", "rev-c3");
    record_own("rev-c3", "rev-c1");
    assert_eq!(
        sent_revision("rev-c1", "rev-c1"),
        "rev-c1",
        "back where it started"
    );
    assert_eq!(
        sent_revision("rev-c2", "rev-c1"),
        "rev-c1",
        "through the cycle"
    );
    record_own("rev-u1", "rev-u2");
    assert_eq!(
        sent_revision("rev-u1", "rev-u1"),
        "rev-u1",
        "undone elsewhere: the daemon holds the opened revision again"
    );
}

/// The daemon's `Conflict` is told apart from other refusals.
#[test]
fn conflict_is_recognised() {
    let conflict: Result<Result<Result<(), KesticDbusError>, String>, String> =
        Ok(Ok(Err(KesticDbusError::Conflict("changed".to_string()))));
    let other: Result<Result<Result<(), KesticDbusError>, String>, String> =
        Ok(Ok(Err(KesticDbusError::InvalidInput("bad".to_string()))));
    let refused: Result<Result<Result<(), KesticDbusError>, String>, String> =
        Ok(Err("input".to_string()));
    assert!(is_conflict(&conflict));
    assert!(!is_conflict(&other));
    assert!(!is_conflict(&refused));
}
