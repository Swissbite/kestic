// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Unit tests of `state_cache` (kept in their own file, NFR-12 file size).

use super::*;
use time::macros::datetime;

#[test]
fn utc_and_offset_same_instant_format_identically_in_local_time() {
    let ts_utc = datetime!(2026-11-16 23:30:00 UTC);
    let ts_plus1 = datetime!(2026-11-17 00:30:00 +01:00);
    let ts_minus5 = datetime!(2026-11-16 18:30:00 -05:00);
    assert_eq!(ts_utc.unix_timestamp(), ts_plus1.unix_timestamp());
    assert_eq!(ts_utc.unix_timestamp(), ts_minus5.unix_timestamp());

    let disp_utc = format_datetime_display(ts_utc);
    let disp_plus1 = format_datetime_display(ts_plus1);
    let disp_minus5 = format_datetime_display(ts_minus5);
    assert_eq!(disp_utc, disp_plus1);
    assert_eq!(disp_utc, disp_minus5);
    assert_eq!(format_time_short(ts_utc), &disp_plus1[11..16]);
    assert_eq!(format_time_short(ts_plus1), &disp_plus1[11..16]);
    assert_eq!(format_time_short(ts_minus5), &disp_plus1[11..16]);
    assert_eq!(
        format_day_group(ts_utc, datetime!(2026-11-17 12:00:00 UTC)),
        format_day_group(ts_plus1, datetime!(2026-11-17 12:00:00 UTC))
    );
}

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

/// M3 manual acceptance (2026-11-19): runs of a deleted backup were titled with an
/// internal ID (`dcde624u`) instead of a readable name.
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
        blocked_reason: Some(REASON_DESTINATION_UNREACHABLE.to_string()),
        reason_text: None,
        source_status: None,
    }
}

/// M3 manual acceptance (2026-11-19, decision): a manual-only backup with an unreachable
/// destination never «waits» (it is never due); a scheduled one does.
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

/// M3 manual acceptance (2026-11-19): a successful maintenance run was shown as
/// «Backed up», although it backs nothing up.
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

/// Decision 2026-11-20 (M3 acceptance, finding 9): a new backup on a local destination gets
/// no network run conditions; on a network destination it keeps the defaults (AC-M3-03-1).
#[test]
fn m3_acceptance_new_backup_has_network_conditions_only_on_network_destinations() {
    let local = DestinationKind::Local {
        path: "/srv/vault".into(),
    };
    let rest = DestinationKind::Rest {
        url: "https://nas.example.invalid/kestic".to_string(),
        username: None,
    };
    let dest_id = sample_backup().destination_id.as_str().to_string();

    let on_local =
        build_new_backup("Docs", "/home/u/docs", &dest_id, "", None, Some(&local)).expect("local");
    let on_rest =
        build_new_backup("Docs", "/home/u/docs", &dest_id, "", None, Some(&rest)).expect("rest");

    // (a) local: power yes, network no.
    assert!(on_local.run_conditions.require_ac_power);
    assert!(!on_local.run_conditions.require_network);
    // (b) network destination: power and network (AC-M3-03-1).
    assert!(on_rest.run_conditions.require_ac_power);
    assert!(on_rest.run_conditions.require_network);
}

/// Saving the run conditions of a backup on a local destination drops network conditions.
#[test]
fn m3_acceptance_saved_conditions_drop_network_for_local_destination() {
    let local = DestinationKind::Local {
        path: "/srv/vault".into(),
    };
    let edited = RunConditions {
        require_ac_power: true,
        require_network: true,
        skip_on_metered_network: true,
        allowed_networks: vec!["Heim".to_string()],
    };

    let updated =
        clone_updated_backup_conditions(&sample_backup(), edited, Some(&local)).expect("updated");

    assert!(!updated.run_conditions.require_network);
    assert!(!updated.run_conditions.skip_on_metered_network);
    assert!(updated.run_conditions.allowed_networks.is_empty());
}
