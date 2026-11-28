// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! M3-S14 editor save path over D-Bus (`AC-M3-14-7`, `AC-M3-14-8`): `UpdateBackup` validates in
//! `core`, then persists `config.toml` and re-plans the schedule in the daemon.

mod common;

use common::revision::UpdateNow;

use common::load_config;
use common::m3::{
    M3Daemon, M3DaemonOpts, StdFakes, add_local_backup, assert_dbus_error_name, backup_summary,
    latest_variant, settle, spawn_m3_daemon,
};
use common::run_isolated_test;
use kestic_core::clock::FakeClock;
use kestic_core::model::{Backup, BackupId, Schedule, Weekday};
use std::fs;
use std::sync::Arc;
use time::macros::datetime;

type Rig = (M3Daemon, Backup, StdFakes);

fn rig(root: &std::path::Path, clock: &Arc<FakeClock>) -> Rig {
    let fakes = StdFakes::start();
    let variant = latest_variant();
    let sub = variant.sub("M3E");
    let bus = variant.bus_name("M3E");
    let daemon = spawn_m3_daemon(
        root,
        M3DaemonOpts::new(&sub, &bus, &variant.bin, Arc::clone(clock) as _),
    );
    let fx = add_local_backup(&daemon.client, &variant.bin, root, &sub, "Docs", |_| {});
    (daemon, fx.backup, fakes)
}

fn next_slot_unix(daemon: &M3Daemon, bid: &BackupId) -> Option<i64> {
    let state = daemon.client.get_state().expect("GetState");
    let summary = backup_summary(&state, bid);
    assert_eq!(
        state.overall.next_scheduled_at_unix, summary.next_scheduled_at_unix,
        "the overview shows the next slot of the only backup"
    );
    summary.next_scheduled_at_unix
}

/// AC-M3-14-7: a changed schedule saved over D-Bus (a) arrives at the daemon, (b) updates
/// `config.toml` and the daemon's schedule, (c) the next slot in the overview matches.
#[test]
fn ac_m3_14_7_saved_schedule_updates_config_schedule_and_next_slot() {
    run_isolated_test(
        "ac_m3_14_7_saved_schedule_updates_config_schedule_and_next_slot",
        "KESTIC_M3_14_7_INNER",
        |root| {
            // Tuesday 2026-12-01 10:00 Europe/Zurich.
            let clock = Arc::new(FakeClock::new(datetime!(2026-12-01 09:00:00 UTC)));
            let (daemon, mut backup, _fakes) = rig(root, &clock);
            let client = &daemon.client;
            let bid = backup.id.clone();
            assert_eq!(
                next_slot_unix(&daemon, &bid),
                Some(datetime!(2026-12-01 11:00:00 UTC).unix_timestamp()),
                "precondition: daily 12:00 -> today 12:00"
            );

            // (a) save "Mon/Wed/Fri 20:00" via D-Bus.
            let new_schedule = Schedule::SpecificWeekdays {
                weekdays: vec![Weekday::Monday, Weekday::Wednesday, Weekday::Friday],
                hour: 20,
                minute: 0,
            };
            backup.schedule = new_schedule.clone();
            client
                .update_backup_now(&backup)
                .expect("(a) UpdateBackup over D-Bus");

            // (b) config.toml and the daemon's own configuration carry the new schedule.
            let on_disk = load_config(&daemon.config_path).expect("load config.toml");
            assert_eq!(
                on_disk.backup(&bid).map(|b| b.schedule.clone()),
                Some(new_schedule.clone()),
                "(b) config.toml updated"
            );
            let state = client.get_state().expect("GetState");
            assert_eq!(
                backup_summary(&state, &bid).backup.schedule,
                new_schedule,
                "(b) daemon schedule updated"
            );
            // The old 12:00 slot no longer fires.
            clock.set(datetime!(2026-12-01 11:00:05 UTC));
            settle();
            let after_old_slot = client.get_state().expect("GetState");
            let summary = backup_summary(&after_old_slot, &bid);
            assert!(
                summary.last_run.is_none(),
                "(b) the replaced daily slot leaves no finished run"
            );
            assert!(
                summary.active_run.is_none(),
                "(b) the replaced daily slot starts no run"
            );
            assert_eq!(summary.pending_primary_code, None, "(b) nothing is pending");

            // (c) next slot = Wednesday 2026-12-02 20:00 Europe/Zurich = 19:00 UTC.
            assert_eq!(
                next_slot_unix(&daemon, &bid),
                Some(datetime!(2026-12-02 19:00:00 UTC).unix_timestamp()),
                "(c) overview next slot matches the new schedule"
            );
            daemon.stop();
        },
    );
}

/// AC-M3-14-8: `[Speichern]` = `UpdateBackup` (a); `core` validates (b) before anything is
/// written to `config.toml` (c); an invalid draft leaves the stored configuration unchanged (d).
#[test]
fn ac_m3_14_8_update_backup_validates_before_writing_config() {
    run_isolated_test(
        "ac_m3_14_8_update_backup_validates_before_writing_config",
        "KESTIC_M3_14_8_INNER",
        |root| {
            let clock = Arc::new(FakeClock::new(datetime!(2026-12-01 09:00:00 UTC)));
            let (daemon, backup, _fakes) = rig(root, &clock);
            let client = &daemon.client;
            let bid = backup.id.clone();
            let before = fs::read(&daemon.config_path).expect("read config.toml");

            // (b) invalid drafts are rejected by core validation: no weekday / "every 0 days".
            let invalid = [
                Schedule::SpecificWeekdays {
                    weekdays: Vec::new(),
                    hour: 20,
                    minute: 0,
                },
                Schedule::EveryNDays {
                    interval_days: 0,
                    start_date: "2026-12-01".to_string(),
                    hour: 9,
                    minute: 0,
                },
            ];
            for schedule in invalid {
                let mut draft = backup.clone();
                draft.schedule = schedule.clone();
                let err = client
                    .update_backup_now(&draft)
                    .expect_err("(b) core must reject the invalid schedule");
                assert_dbus_error_name(&err, "net.daester.Kestic.Error.InvalidInput");
                // (d) config.toml and the daemon's schedule stay unchanged.
                assert_eq!(
                    fs::read(&daemon.config_path).expect("read config.toml"),
                    before,
                    "(d) config.toml unchanged after rejected {schedule:?}"
                );
                let state = client.get_state().expect("GetState");
                assert_eq!(
                    backup_summary(&state, &bid).backup.schedule,
                    backup.schedule,
                    "(d) daemon schedule unchanged after rejected {schedule:?}"
                );
            }

            // (a)+(c) a valid draft is written to config.toml only after validation succeeded.
            let mut valid = backup;
            valid.schedule = Schedule::Daily {
                hour: 8,
                minute: 30,
            };
            client
                .update_backup_now(&valid)
                .expect("(a) valid UpdateBackup");
            let on_disk = load_config(&daemon.config_path).expect("load config.toml");
            assert_eq!(
                on_disk.backup(&bid).map(|b| b.schedule.clone()),
                Some(valid.schedule),
                "(c) validated change persisted"
            );
            assert_ne!(
                fs::read(&daemon.config_path).expect("read config.toml"),
                before,
                "(c) config.toml was rewritten"
            );
            daemon.stop();
        },
    );
}
