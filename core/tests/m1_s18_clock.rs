// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

use kestic_core::clock::{Clock, FakeClock, SystemClock};
use kestic_core::model::{Backup, Destination, DestinationKind, Run, Source};
use std::fs;
use std::path::{Path, PathBuf};
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
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().expect("workspace root");
    let system_clock_file = workspace_root.join("core/src/clock.rs");

    let forbidden = [
        "SystemTime::now",
        "OffsetDateTime::now_utc",
        "OffsetDateTime::now_local",
        "UtcDateTime::now",
    ];

    let mut violations = Vec::new();
    for crate_src in ["core/src", "daemon/src"] {
        let dir = workspace_root.join(crate_src);
        visit_rs_files(&dir, &mut |path| {
            if path == system_clock_file {
                return;
            }
            let content = fs::read_to_string(path).expect("read rust source");
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
        });
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

fn visit_rs_files(dir: &Path, f: &mut impl FnMut(&Path)) {
    for entry in fs::read_dir(dir).expect("read dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            visit_rs_files(&path, f);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            f(&path);
        }
    }
}
