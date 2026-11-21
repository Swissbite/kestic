// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! M3-S03 run conditions over the UPower / NetworkManager D-Bus fakes (`AC-M3-03-2` … `AC-M3-03-7`).

mod common;

use common::fakes::FakeNetworkManagerHandle;
use common::m3::{
    FakeSetup, LocalBackupFixture, M3Daemon, M3DaemonOpts, ResticVariant, StdFakes,
    add_local_backup, assert_dbus_error_name, backup_summary, executed_runs, latest_variant,
    notifications_for, restic_variants, settle, spawn_m3_daemon, wait_for_notifications,
    wait_for_state,
};
use common::run_isolated_test;
use kestic_core::clock::FakeClock;
use kestic_core::model::{Backup, BackupId, RunStatus};
use kestic_daemon::dbus_api::DaemonStateDto;
use kestic_i18n::codes::BlockedReason;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use time::macros::datetime;

/// 2026-12-01 11:59:30 Europe/Zurich, 30 s before the daily 12:00 slot.
const BEFORE_SLOT: time::OffsetDateTime = datetime!(2026-12-01 10:59:30 UTC);
/// 2026-12-01 12:00:05 Europe/Zurich, just after the daily 12:00 slot.
const SLOT_1: time::OffsetDateTime = datetime!(2026-12-01 11:00:05 UTC);
/// 2026-12-02 12:00:05 Europe/Zurich.
const SLOT_2: time::OffsetDateTime = datetime!(2026-12-02 11:00:05 UTC);

struct Rig {
    daemon: M3Daemon,
    clock: Arc<FakeClock>,
    fixture: LocalBackupFixture,
}

impl Rig {
    fn bid(&self) -> &BackupId {
        &self.fixture.backup.id
    }

    fn start(
        root: &Path,
        variant: &ResticVariant,
        base: &str,
        configure: impl FnOnce(&mut Backup),
    ) -> Self {
        let clock = Arc::new(FakeClock::new(BEFORE_SLOT));
        let sub = variant.sub(base);
        let bus = variant.bus_name(base);
        let daemon = spawn_m3_daemon(
            root,
            M3DaemonOpts::new(&sub, &bus, &variant.bin, Arc::clone(&clock) as _),
        );
        let fixture = add_local_backup(&daemon.client, &variant.bin, root, &sub, "Docs", configure);
        Self {
            daemon,
            clock,
            fixture,
        }
    }

    fn wait_pending(&self, code: &str) -> DaemonStateDto {
        wait_for_state(&self.daemon.client, Duration::from_secs(10), code, |s| {
            backup_summary(s, self.bid())
                .pending_primary_code
                .map(|c| c.as_str())
                == Some(code)
        })
    }

    fn wait_ok_run(&self) -> DaemonStateDto {
        wait_for_state(
            &self.daemon.client,
            Duration::from_secs(20),
            "run Ok",
            |s| {
                backup_summary(s, self.bid())
                    .last_run
                    .as_ref()
                    .is_some_and(|r| r.status == RunStatus::Ok)
            },
        )
    }
}

/// AC-M3-03-2: on battery the due backup is `ausstehend` (run condition); once AC power returns it
/// starts in the same slot without waiting for the next one.
#[test]
fn ac_m3_03_2_on_battery_pending_then_starts_on_ac_power() {
    run_isolated_test(
        "ac_m3_03_2_on_battery_pending_then_starts_on_ac_power",
        "KESTIC_M3_03_2_INNER",
        |root| {
            let fakes = StdFakes::start_with_power(true, 60.0);
            for variant in restic_variants() {
                fakes.upower.set_on_battery(true, 60.0);
                let rig = Rig::start(root, &variant, "M3C2", |_| {});

                rig.clock.set(SLOT_1);
                let pending = rig.wait_pending("on_battery");
                // (a) pending because of the run condition, and no restic run was executed.
                assert_eq!(
                    backup_summary(&pending, rig.bid())
                        .pending_primary_code
                        .map(|c| c.as_str()),
                    Some("on_battery"),
                    "{}: (a) pending reason is the run condition",
                    variant.label
                );
                assert_eq!(
                    executed_runs(&pending, rig.bid()),
                    0,
                    "{}: (a) no run may execute on battery",
                    variant.label
                );

                // (b) AC power returns -> starts without advancing the clock to the next slot.
                fakes.upower.set_on_battery(false, 100.0);
                let done = rig.wait_ok_run();
                let last = backup_summary(&done, rig.bid())
                    .last_run
                    .clone()
                    .expect("run");
                assert_eq!(
                    last.started_at, SLOT_1,
                    "{}: (b) run started in the same slot (clock unchanged)",
                    variant.label
                );
                assert_eq!(executed_runs(&done, rig.bid()), 1, "{}: (b)", variant.label);
                assert_eq!(
                    backup_summary(&done, rig.bid()).pending_primary_code,
                    None,
                    "{}: (b) pending reason cleared",
                    variant.label
                );
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M3-03-3: with "not on metered connection" the backup is `ausstehend` while NetworkManager
/// reports `Metered=yes` and starts after `Metered=no`.
#[test]
fn ac_m3_03_3_metered_connection_pending_then_starts_when_unmetered() {
    run_isolated_test(
        "ac_m3_03_3_metered_connection_pending_then_starts_when_unmetered",
        "KESTIC_M3_03_3_INNER",
        |root| {
            let fakes = StdFakes::start_with(FakeSetup {
                metered: true,
                ..FakeSetup::default()
            });
            for variant in restic_variants() {
                fakes.nm.update(true, true, Some("Heim"));
                let rig = Rig::start(root, &variant, "M3C3", |b| {
                    b.run_conditions.skip_on_metered_network = true;
                });

                rig.clock.set(SLOT_1);
                let pending = rig.wait_pending("metered");
                // (a) pending on a metered connection, nothing executed.
                assert_eq!(
                    backup_summary(&pending, rig.bid())
                        .pending_primary_code
                        .map(|c| c.as_str()),
                    Some("metered"),
                    "{}: (a)",
                    variant.label
                );
                assert_eq!(
                    executed_runs(&pending, rig.bid()),
                    0,
                    "{}: (a)",
                    variant.label
                );

                // (b) Metered=no directly -> starts.
                fakes.nm.update(true, false, Some("Heim"));
                let done = rig.wait_ok_run();
                assert_eq!(executed_runs(&done, rig.bid()), 1, "{}: (b)", variant.label);
                assert_eq!(
                    backup_summary(&done, rig.bid())
                        .last_run
                        .as_ref()
                        .map(|r| r.started_at),
                    Some(SLOT_1),
                    "{}: (b) same slot",
                    variant.label
                );
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M3-03-4: "only in network Heim" with active connection "Cafe" -> `ausstehend`; switching to
/// "Heim" starts the run.
#[test]
fn ac_m3_03_4_allowed_network_pending_then_starts_on_home_network() {
    run_isolated_test(
        "ac_m3_03_4_allowed_network_pending_then_starts_on_home_network",
        "KESTIC_M3_03_4_INNER",
        |root| {
            let fakes = StdFakes::start_with(FakeSetup {
                active_network: Some("Cafe"),
                known_networks: &["Heim", "Cafe"],
                ..FakeSetup::default()
            });
            for variant in restic_variants() {
                fakes.nm.update(true, false, Some("Cafe"));
                let rig = Rig::start(root, &variant, "M3C4", |b| {
                    b.run_conditions.allowed_networks = vec!["Heim".to_string()];
                });

                rig.clock.set(SLOT_1);
                let pending = rig.wait_pending("wrong_network");
                assert_eq!(
                    backup_summary(&pending, rig.bid())
                        .pending_primary_code
                        .map(|c| c.as_str()),
                    Some("wrong_network"),
                    "{}: (a)",
                    variant.label
                );
                assert_eq!(
                    executed_runs(&pending, rig.bid()),
                    0,
                    "{}: (a)",
                    variant.label
                );

                fakes.nm.update(true, false, Some("Heim"));
                let done = rig.wait_ok_run();
                assert_eq!(executed_runs(&done, rig.bid()), 1, "{}: (b)", variant.label);
                assert_eq!(
                    backup_summary(&done, rig.bid())
                        .last_run
                        .as_ref()
                        .map(|r| r.started_at),
                    Some(SLOT_1),
                    "{}: (b) same slot",
                    variant.label
                );
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M3-03-5: "only with network" and NetworkManager without connection -> `ausstehend`.
#[test]
fn ac_m3_03_5_no_network_connection_is_pending() {
    run_isolated_test(
        "ac_m3_03_5_no_network_connection_is_pending",
        "KESTIC_M3_03_5_INNER",
        |root| {
            let _fakes = StdFakes::start_with(FakeSetup {
                connected: false,
                active_network: None,
                ..FakeSetup::default()
            });
            let variant = latest_variant();
            let rig = Rig::start(root, &variant, "M3C5", |_| {});

            rig.clock.set(SLOT_1);
            let pending = rig.wait_pending("no_network");
            assert_eq!(
                backup_summary(&pending, rig.bid())
                    .pending_primary_code
                    .map(|c| c.as_str()),
                Some("no_network"),
                "(a) pending without network connection"
            );
            settle();
            let after = rig.daemon.client.get_state().expect("GetState");
            assert_eq!(
                executed_runs(&after, rig.bid()),
                0,
                "(a) no run executes without network"
            );
            rig.daemon.stop();
        },
    );
}

/// AC-M3-03-6: run condition unmet over two slots -> (b) no "overdue" notification after the first
/// missed slot, (a) exactly one after the second.
#[test]
fn ac_m3_03_6_unmet_condition_over_two_slots_notifies_overdue_exactly_once() {
    run_isolated_test(
        "ac_m3_03_6_unmet_condition_over_two_slots_notifies_overdue_exactly_once",
        "KESTIC_M3_03_6_INNER",
        |root| {
            let fakes = StdFakes::start_with_power(true, 40.0);
            let variant = latest_variant();
            let rig = Rig::start(root, &variant, "M3C6", |_| {});

            rig.clock.set(SLOT_1);
            let first = rig.wait_pending("on_battery");
            settle();
            // (b) before the second missed slot: not overdue, no notification.
            assert!(
                !backup_summary(&first, rig.bid()).is_overdue,
                "(b) one missed slot is not overdue"
            );
            assert_eq!(
                notifications_for(&fakes.notif, "BackupOverdue").len(),
                0,
                "(b) no overdue notification before the second missed slot"
            );

            rig.clock.set(SLOT_2);
            wait_for_state(
                &rig.daemon.client,
                Duration::from_secs(10),
                "overdue",
                |s| backup_summary(s, rig.bid()).is_overdue,
            );
            wait_for_notifications(&fakes.notif, "BackupOverdue", 1, Duration::from_secs(5));
            settle();
            assert_eq!(
                notifications_for(&fakes.notif, "BackupOverdue").len(),
                1,
                "(a) exactly one overdue notification: {:?}",
                fakes.notif.snapshot()
            );
            rig.daemon.stop();
        },
    );
}

/// AC-M3-03-7: manual start on battery is blocked and shows the unmet condition (a), runs after
/// confirmation (b), stays `ausstehend` without confirmation (c), and the override applies to this
/// run only (d).
#[test]
fn ac_m3_03_7_manual_override_applies_to_one_run_only() {
    run_isolated_test(
        "ac_m3_03_7_manual_override_applies_to_one_run_only",
        "KESTIC_M3_03_7_INNER",
        |root| {
            let fakes = StdFakes::start_with_power(true, 55.0);
            for variant in restic_variants() {
                fakes.upower.set_on_battery(true, 55.0);
                let rig = Rig::start(root, &variant, "M3C7", |_| {});
                let client = &rig.daemon.client;
                rig.clock.set(SLOT_1);
                rig.wait_pending("on_battery");

                // (a) the manual start reports the unmet run condition ("start anyway" is offered).
                let err = client
                    .start_backup(&rig.fixture.backup_id_str)
                    .expect_err("manual start must be blocked on battery");
                assert_dbus_error_name(&err, "net.daester.Kestic.Error.ManualRunBlocked");
                assert!(
                    err.to_string()
                        .contains(BlockedReason::ConditionNotMet.as_str()),
                    "{}: (a) reason `{}` in {err}",
                    variant.label,
                    BlockedReason::ConditionNotMet.as_str()
                );
                let blocked = client.get_state().expect("GetState");
                let summary = backup_summary(&blocked, rig.bid());
                assert!(
                    summary.has_unmet_run_condition,
                    "{}: (a) state exposes the unmet run condition",
                    variant.label
                );
                assert_eq!(
                    summary
                        .unmet_run_conditions
                        .iter()
                        .map(|c| c.as_str())
                        .collect::<Vec<_>>(),
                    vec!["on_battery"],
                    "{}: (a) state names the unmet condition",
                    variant.label
                );
                assert_eq!(
                    summary.battery_percent,
                    Some(55),
                    "{}: (a) state carries the battery level for the condition text",
                    variant.label
                );

                // (c) without confirmation the backup stays `ausstehend`, no run executes.
                settle();
                let still = client.get_state().expect("GetState");
                assert_eq!(
                    backup_summary(&still, rig.bid())
                        .pending_primary_code
                        .map(|c| c.as_str()),
                    Some("on_battery"),
                    "{}: (c) still pending",
                    variant.label
                );
                assert_eq!(
                    executed_runs(&still, rig.bid()),
                    0,
                    "{}: (c)",
                    variant.label
                );

                // (b) after confirmation the backup runs.
                client
                    .start_backup_override(&rig.fixture.backup_id_str)
                    .expect("StartBackupOverride");
                let done = rig.wait_ok_run();
                assert_eq!(executed_runs(&done, rig.bid()), 1, "{}: (b)", variant.label);

                // (d) the next slot on battery is blocked again: the override was for one run only.
                rig.clock.set(SLOT_2);
                rig.wait_pending("on_battery");
                settle();
                let next = client.get_state().expect("GetState");
                assert_eq!(
                    executed_runs(&next, rig.bid()),
                    1,
                    "{}: (d) no further run after the override run",
                    variant.label
                );
                rig.daemon.stop();
            }
        },
    );
}

/// «Allowed Wi-Fi networks» offers only saved Wi-Fi profiles, not the wired profile or the
/// loopback device (FR-13).
#[test]
fn m3_acceptance_known_networks_lists_only_wifi_profiles() {
    run_isolated_test(
        "m3_acceptance_known_networks_lists_only_wifi_profiles",
        "KESTIC_M3_ACC_KNOWN_NETS_INNER",
        |_root| {
            let _nm = FakeNetworkManagerHandle::start(true, false, Some("Heim"), &["Heim", "Cafe"]);

            let names = kestic_daemon::monitors::list_known_networks(
                &kestic_daemon::monitors::MonitorBusConfig {
                    use_session_bus: true,
                    ..Default::default()
                },
            );

            assert_eq!(names, vec!["Cafe".to_string(), "Heim".to_string()]);
            assert!(!names.iter().any(|n| n == common::fakes::FAKE_NM_WIRED_ID));
            assert!(
                !names
                    .iter()
                    .any(|n| n == common::fakes::FAKE_NM_LOOPBACK_ID)
            );
        },
    );
}
