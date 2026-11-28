// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Unit tests of `status_texts` (kept in their own file, NFR-12 file size).

use super::*;
use kestic_core::model::*;
use time::macros::datetime;

fn sample_backup() -> Backup {
    let dest = Destination::new(
        "Vault",
        DestinationKind::Local {
            path: "/srv/vault".into(),
        },
    )
    .expect("dest");
    Backup::new("Documents", Source::folder("/home/u/docs"), dest.id).expect("backup")
}

/// A run of a deleted backup is titled as deleted, not with an internal ID (`dcde624u`).
#[test]
fn m3_acceptance_run_of_deleted_backup_is_titled_as_deleted() {
    let clock = kestic_core::clock::FakeClock::new(datetime!(2026-11-19 12:00:00 UTC));
    let run = Run::new_backup(&clock, &sample_backup()).expect("run");

    assert_eq!(
        format_run_kind_name(&run, None, "dcde624u"),
        "Deleted backup"
    );
    assert_eq!(
        format_run_kind_name(&run, Some("Documents"), "Vault"),
        "Documents"
    );
}

fn unreachable_destination() -> BackupAvailabilityDto {
    BackupAvailabilityDto {
        backup_id: "b1".to_string(),
        source_available: true,
        destination_available: false,
        available: false,
        blocked_reason: Some(BlockedReason::DestinationUnreachable),
        source_status: None,
    }
}

/// A manual-only backup with an unreachable destination never «waits» (it is never due); a
/// scheduled one does.
#[test]
fn m3_acceptance_manual_backup_with_unreachable_destination_does_not_wait() {
    let (manual, manual_icon, _) =
        unavailable_status(&Schedule::ManualOnly, &unreachable_destination());
    let (scheduled, _, _) = unavailable_status(
        &Schedule::Daily {
            hour: 12,
            minute: 0,
        },
        &unreachable_destination(),
    );

    assert_eq!(manual, "Manual only · Destination unreachable");
    assert_ne!(manual_icon, "chronometer");
    assert_eq!(scheduled, "Waiting: destination unreachable");
}

/// A successful maintenance run is shown as completed, not as «Backed up», because it backs
/// nothing up.
#[test]
fn m3_acceptance_successful_maintenance_is_completed_not_backed_up() {
    let clock = kestic_core::clock::FakeClock::new(datetime!(2026-11-19 12:00:00 UTC));
    let maint = Run::new_maintenance(&clock, sample_backup().destination_id).expect("run");
    let backup = Run::new_backup(&clock, &sample_backup()).expect("run");

    assert_eq!(format_run_status(&maint.kind, RunStatus::Ok).0, "Completed");
    assert_eq!(
        format_run_status(&backup.kind, RunStatus::Ok).0,
        "Backed up"
    );
}
