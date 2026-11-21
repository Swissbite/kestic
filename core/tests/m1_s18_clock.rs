// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

mod common;

use kestic_core::clock::{Clock, FakeClock, SystemClock};
use kestic_core::model::{Backup, Destination, DestinationKind, Run, Source};
use kestic_test_support::workspace::{files_with_ext, workspace_root};
use std::fs;
use std::time::Duration;
use time::macros::datetime;

#[test]
fn ac_m1_18_1_fake_clock_drives_run_timestamps() {
    let clock = FakeClock::new(datetime!(2026-12-01 12:00:00 UTC));
    let dest = Destination::new(
        "Local",
        DestinationKind::Local {
            path: "/tmp/repo".into(),
        },
    )
    .expect("destination");
    let backup = Backup::new("Home", Source::folder("/home/alice"), dest.id).expect("backup");

    let run1 = Run::new_backup(&clock, &backup).expect("run 1");
    assert_eq!(run1.started_at, datetime!(2026-12-01 12:00:00 UTC));

    clock.advance(Duration::from_secs(2 * 3600));
    let run2 = Run::new_backup(&clock, &backup).expect("run 2");
    assert_eq!(run2.started_at, datetime!(2026-12-01 14:00:00 UTC));
}

#[test]
fn ac_m1_18_2_no_direct_system_clock_reads_outside_clock_module() {
    let workspace_root = workspace_root();
    let system_clock_file = workspace_root.join("core/src/clock.rs");

    let forbidden = [
        "SystemTime::now",
        "OffsetDateTime::now_utc",
        "OffsetDateTime::now_local",
        "UtcDateTime::now",
    ];

    let mut violations = Vec::new();
    for crate_src in ["core/src", "daemon/src"] {
        for path in files_with_ext(&workspace_root.join(crate_src), "rs") {
            if path == system_clock_file {
                continue;
            }
            let content = fs::read_to_string(&path).expect("read rust source");
            for (line_idx, line) in content.lines().enumerate() {
                for pattern in forbidden {
                    if line.contains(pattern) {
                        violations.push(format!(
                            "{}:{}: forbidden direct clock read `{pattern}`",
                            path.display(),
                            line_idx + 1
                        ));
                    }
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "Clock-Trait discipline violations (AC-M1-18-2):\n{}",
        violations.join("\n")
    );

    // Production clock uses the system clock.
    let now = SystemClock.now_utc();
    assert!(now.year() >= 2026);
}
