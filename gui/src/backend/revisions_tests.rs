// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

/// AC-M6-36-10 (d), (e): saves of this window move its own revision on, also over several saves,
/// so a later save of the same or another page does not conflict with them; a revision no own
/// save left stays as it is.
#[test]
fn ac_m6_36_10_d_e_own_saves_move_the_revision_on() {
    record_own("rev-a1", "rev-a2");
    record_own("rev-a2", "rev-a3");
    assert_eq!(latest_own("rev-a1"), "rev-a3", "(d) two saves in a row");
    assert_eq!(
        latest_own("rev-a2"),
        "rev-a3",
        "(e) a page opened after the first save"
    );
    assert_eq!(
        latest_own("rev-other"),
        "rev-other",
        "a foreign change is not followed"
    );
    record_own("", "rev-x");
    record_own("rev-y", "");
    assert_eq!(latest_own(""), "", "an empty revision stays empty");
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
