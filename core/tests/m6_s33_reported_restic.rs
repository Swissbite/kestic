// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M6-S33: the GUI builds the shown command from the restic the daemon reports, without running
//! `restic version` itself (`AC-M6-33-5`).

use kestic_core::provider::resolved_from_report;
use semver::Version;
use std::path::PathBuf;

/// AC-M6-33-5: (a) the reported path and version give the restic of the shown command; an
/// unreadable version gives none, so the GUI falls back to the short form.
#[test]
fn ac_m6_33_5_reported_restic_is_used_as_is() {
    let resolved = resolved_from_report("/usr/bin/restic", "0.18.0").expect("readable report");
    assert_eq!(resolved.binary, PathBuf::from("/usr/bin/restic"));
    assert_eq!(resolved.version, Version::new(0, 18, 0));
    assert!(
        resolved_from_report("/usr/bin/restic", "not a version").is_none(),
        "an unreadable version gives no restic"
    );
}
