// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! M3-S08 monitoring: notifications per event (F3), tray status and progress without GUI, and
//! "Back up all" with skipped backups (`AC-M3-08-1`, `-2`, `-4`, `-6`).

mod common;

use common::fakes::{FakeNetworkManagerHandle, FakeUPowerHandle};
use common::m3::{
    LocalBackupFixture, M3Daemon, M3DaemonOpts, ResticVariant, ResticWrapper, add_local_backup,
    backup_summary, latest_variant, notifications_for, restic_variants, settle, spawn_m3_daemon,
    wait_for_notifications, wait_for_state,
};
use common::{
    NotificationCapture, click_dbusmenu_item, fetch_dbusmenu_items, read_sni_string_prop,
    read_sni_tooltip, run_isolated_test, start_fake_notifications, start_fake_sni_watcher,
};
use kestic_core::clock::FakeClock;
use kestic_core::history::RunHistory;
use kestic_core::model::{NotificationSettings, RunStatus, Schedule};
use kestic_core::restic::SECRET_KIND_REPO;
use kestic_core::secrets::{FileSecretStore, SecretStore, SecretValue};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use time::OffsetDateTime;
use time::macros::datetime;

/// 2026-12-01 11:59:30 Europe/Zurich.
const BEFORE_SLOT: OffsetDateTime = datetime!(2026-12-01 10:59:30 UTC);

struct Rig {
    daemon: M3Daemon,
    clock: Arc<FakeClock>,
    wrapper: ResticWrapper,
    root: std::path::PathBuf,
    sub: String,
}

fn start_rig(root: &Path, variant: &ResticVariant, base: &str, scheduler: bool) -> Rig {
    let sub = variant.sub(base);
    let wrapper = ResticWrapper::create(&root.join(&sub).join("wrapper"), &variant.bin);
    let clock = Arc::new(FakeClock::new(BEFORE_SLOT));
    let bus = variant.bus_name(base);
    let mut opts = M3DaemonOpts::new(&sub, &bus, &wrapper.path, Arc::clone(&clock) as _);
    opts.enable_scheduler = scheduler;
    let daemon = spawn_m3_daemon(root, opts);
    Rig {
        daemon,
        clock,
        wrapper,
        root: root.to_path_buf(),
        sub,
    }
}

impl Rig {
    /// Manual-only backup without run conditions (so it starts regardless of power/network).
    fn add_manual(&self, name: &str) -> LocalBackupFixture {
        add_local_backup(&self.daemon.client, &self.root, &self.sub, name, |b| {
            b.schedule = Schedule::ManualOnly;
            b.run_conditions.require_ac_power = false;
            b.run_conditions.require_network = false;
        })
    }

    fn run_to(&self, fx: &LocalBackupFixture, status: RunStatus) {
        self.daemon
            .client
            .start_backup(&fx.backup_id_str)
            .expect("manual start");
        wait_for_state(
            &self.daemon.client,
            Duration::from_secs(30),
            "run end",
            |s| {
                let b = backup_summary(s, &fx.backup.id);
                b.active_run.is_none() && b.last_run.as_ref().is_some_and(|r| r.status == status)
            },
        );
    }
}

fn start_fakes(
    on_battery: bool,
) -> (
    Arc<NotificationCapture>,
    zbus::Connection,
    zbus::Connection,
    FakeUPowerHandle,
    FakeNetworkManagerHandle,
) {
    let (nc, notif) = start_fake_notifications();
    (
        notif,
        nc,
        start_fake_sni_watcher(),
        FakeUPowerHandle::start(on_battery, 70.0),
        FakeNetworkManagerHandle::start(true, false, Some("Heim"), &["Heim"]),
    )
}

/// Short wait (one scheduler tick plus async notification dispatch) before asserting an exact count.
fn short_settle() {
    std::thread::sleep(Duration::from_millis(1200));
}

fn make_unreadable_file(src_dir: &Path) -> std::path::PathBuf {
    let bad = src_dir.join("unreadable.secret");
    fs::write(&bad, b"cannot read me").expect("write bad file");
    fs::set_permissions(&bad, fs::Permissions::from_mode(0o000)).expect("chmod 000");
    bad
}

fn wait_tooltip(
    conn: &zbus::Connection,
    service: &str,
    label: &str,
    pred: impl Fn(&str) -> bool,
) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let (_title, description) = read_sni_tooltip(conn, service);
        if pred(&description) {
            return description;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for tray tooltip ({label}); last: {description:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// AC-M3-08-1 (FR-12, F3): under default settings (a) `Fehler`, `Warnung` and "überfällig" each
/// produce exactly one notification, (b) `ok` produces none but a tray state and a history entry.
#[test]
fn ac_m3_08_1_default_settings_notify_error_warning_overdue_once_and_ok_never() {
    run_isolated_test(
        "ac_m3_08_1_default_settings_notify_error_warning_overdue_once_and_ok_never",
        "KESTIC_M3_08_1_INNER",
        |root| {
            let (notif, _nc, sni, _upower, _nm) = start_fakes(true);
            for variant in restic_variants() {
                notif.clear();
                let rig = start_rig(root, &variant, "M3M1", true);
                let client = &rig.daemon.client;
                let sni_service =
                    common::wait_for_sni_service_at(&sni, variant.index, Duration::from_secs(5));

                // (b) ok: no notification, but tray state and history entry.
                let ok_fx = rig.add_manual("OkBk");
                rig.run_to(&ok_fx, RunStatus::Ok);
                short_settle();
                assert_eq!(
                    notif.snapshot().len(),
                    0,
                    "{}: (b) an ok run raises no notification: {:?}",
                    variant.label,
                    notif.snapshot()
                );
                wait_tooltip(client.connection(), &sni_service, "all backed up", |d| {
                    d == "All data backed up"
                });
                let state = client.get_state().expect("GetState");
                assert!(
                    state.recent_runs.iter().any(|r| {
                        r.backup_id() == Some(&ok_fx.backup.id) && r.status == RunStatus::Ok
                    }),
                    "{}: (b) the ok run is in the history",
                    variant.label
                );

                // (a) Warning (Exit 3, unreadable file) -> exactly one notification.
                let warn_fx = rig.add_manual("WarnBk");
                let bad = make_unreadable_file(&warn_fx.src_dir);
                rig.run_to(&warn_fx, RunStatus::Warning);
                wait_for_notifications(&notif, "BackupWarning", 1, Duration::from_secs(5));
                short_settle();
                assert_eq!(
                    notifications_for(&notif, "BackupWarning").len(),
                    1,
                    "{}: (a) exactly one warning notification",
                    variant.label
                );
                let _ = fs::set_permissions(&bad, fs::Permissions::from_mode(0o644));

                // (a) Error (Exit 12, wrong password) -> exactly one notification.
                let err_fx = rig.add_manual("ErrBk");
                FileSecretStore::new(root.join("secrets.hex"))
                    .set_secret(
                        err_fx.dest.id.as_str(),
                        SECRET_KIND_REPO,
                        &SecretValue::from_str_secret("wrong-password"),
                    )
                    .expect("wrong password");
                rig.run_to(&err_fx, RunStatus::Error);
                wait_for_notifications(&notif, "BackupFailed", 1, Duration::from_secs(5));
                short_settle();
                assert_eq!(
                    notifications_for(&notif, "BackupFailed").len(),
                    1,
                    "{}: (a) exactly one error notification",
                    variant.label
                );

                // (a) overdue: scheduled backup on battery misses two slots -> exactly one notification.
                let late_id = add_local_backup(client, root, &rig.sub, "LateBk", |_| {})
                    .backup
                    .id;
                rig.clock.set(datetime!(2026-12-01 11:00:05 UTC));
                wait_for_state(client, Duration::from_secs(10), "late pending", |s| {
                    backup_summary(s, &late_id).pending_primary_code.as_deref()
                        == Some("on_battery")
                });
                rig.clock.set(datetime!(2026-12-02 11:00:05 UTC));
                wait_for_notifications(&notif, "BackupOverdue", 1, Duration::from_secs(10));
                settle();
                assert_eq!(
                    notifications_for(&notif, "BackupOverdue").len(),
                    1,
                    "{}: (a) exactly one overdue notification",
                    variant.label
                );
                assert_eq!(
                    notif.snapshot().len(),
                    3,
                    "{}: (a)/(b) only warning, error and overdue notified: {:?}",
                    variant.label,
                    notif.snapshot()
                );
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M3-08-2 (F3): with the event kind "Warnung" switched off, a run ending with Exit 3
/// (a) raises no notification and (b) still shows `Warnung` in history and tray.
#[test]
fn ac_m3_08_2_disabled_warning_notification_still_shows_warning_in_history_and_tray() {
    run_isolated_test(
        "ac_m3_08_2_disabled_warning_notification_still_shows_warning_in_history_and_tray",
        "KESTIC_M3_08_2_INNER",
        |root| {
            let (notif, _nc, sni, _upower, _nm) = start_fakes(false);
            for variant in restic_variants() {
                notif.clear();
                let rig = start_rig(root, &variant, "M3M2", true);
                let client = &rig.daemon.client;
                let sni_service =
                    common::wait_for_sni_service_at(&sni, variant.index, Duration::from_secs(5));
                client
                    .update_settings(
                        2,
                        &NotificationSettings {
                            on_warning: false,
                            ..Default::default()
                        },
                    )
                    .expect("UpdateSettings");
                let fx = rig.add_manual("WarnBk");
                let bad = make_unreadable_file(&fx.src_dir);
                rig.run_to(&fx, RunStatus::Warning);
                settle();
                let _ = fs::set_permissions(&bad, fs::Permissions::from_mode(0o644));

                let last = backup_summary(&client.get_state().expect("GetState"), &fx.backup.id)
                    .last_run
                    .clone()
                    .expect("last run");
                assert_eq!(
                    last.exit_code,
                    Some(3),
                    "{}: run ended with Exit 3",
                    variant.label
                );
                // (a) no notification at all.
                assert_eq!(
                    notif.snapshot().len(),
                    0,
                    "{}: (a) warning notification is suppressed: {:?}",
                    variant.label,
                    notif.snapshot()
                );
                // (b) history and tray still show the warning.
                let state = client.get_state().expect("GetState");
                assert!(
                    state
                        .recent_runs
                        .iter()
                        .any(|r| r.id == last.id && r.status == RunStatus::Warning),
                    "{}: (b) warning run in the history",
                    variant.label
                );
                wait_tooltip(client.connection(), &sni_service, "warning in tray", |d| {
                    d.contains("finished with warning")
                });
                assert_eq!(
                    read_sni_string_prop(client.connection(), &sni_service, "Status"),
                    "Active",
                    "{}: (b) tray is not passive while a warning is shown",
                    variant.label
                );
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M3-08-4 (NFR-11, NFR-04): with no GUI running, a scheduled run (a) shows status and progress
/// in the tray and (b) is in the history afterwards.
#[test]
fn ac_m3_08_4_scheduled_run_without_gui_shows_tray_progress_and_lands_in_history() {
    run_isolated_test(
        "ac_m3_08_4_scheduled_run_without_gui_shows_tray_progress_and_lands_in_history",
        "KESTIC_M3_08_4_INNER",
        |root| {
            let (_notif, _nc, sni, _upower, _nm) = start_fakes(false);
            for variant in restic_variants() {
                let rig = start_rig(root, &variant, "M3M4", true);
                let client = &rig.daemon.client;
                let sni_service =
                    common::wait_for_sni_service_at(&sni, variant.index, Duration::from_secs(5));
                let fx = add_local_backup(client, root, &rig.sub, "Docs", |_| {});
                rig.wrapper.hold("backup");

                // No GUI is registered and no window is active; the scheduler starts the run.
                rig.clock.set(datetime!(2026-12-01 11:00:05 UTC));
                let running = wait_for_state(client, Duration::from_secs(15), "running", |s| {
                    backup_summary(s, &fx.backup.id).active_run.is_some()
                });
                let run_id = backup_summary(&running, &fx.backup.id)
                    .active_run
                    .as_ref()
                    .map(|r| r.id.clone())
                    .expect("active run");
                // (a) tray status and progress while the run is active.
                let tooltip = wait_tooltip(client.connection(), &sni_service, "progress", |d| {
                    d.contains("backing up") && d.contains("25%")
                });
                assert!(
                    tooltip.contains("Docs"),
                    "{}: (a) tray names the running backup: {tooltip:?}",
                    variant.label
                );
                assert_eq!(
                    read_sni_string_prop(client.connection(), &sni_service, "Status"),
                    "Active",
                    "{}: (a) SNI status while running",
                    variant.label
                );
                let menu = fetch_dbusmenu_items(client.connection(), &sni_service);
                assert!(
                    menu.iter()
                        .any(|e| e.label.contains("backing up") && e.label.contains("25%")),
                    "{}: (a) tray menu shows the progress line: {menu:?}",
                    variant.label
                );

                // (b) after the run it is in the history (D-Bus and persisted).
                rig.wrapper.release("backup");
                wait_for_state(client, Duration::from_secs(30), "run Ok", |s| {
                    backup_summary(s, &fx.backup.id)
                        .last_run
                        .as_ref()
                        .is_some_and(|r| r.id == run_id && r.status == RunStatus::Ok)
                });
                let detail = client
                    .get_run_detail(run_id.as_str())
                    .expect("GetRunDetail");
                assert_eq!(detail.run.status, RunStatus::Ok, "{}: (b)", variant.label);
                let persisted = RunHistory::open(&rig.daemon.state_dir)
                    .expect("open history")
                    .get_run(&run_id)
                    .expect("query run")
                    .expect("run persisted");
                assert_eq!(
                    persisted.status,
                    RunStatus::Ok,
                    "{}: (b) persisted",
                    variant.label
                );
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M3-08-6 (FR-13b): "Back up all" with one valid backup, one with a missing source and one
/// with a permanent error (Exit 12) -> (a) only the valid one starts, (b) a message (also in the
/// history) names the two skipped ones with the reason.
#[test]
fn ac_m3_08_6_back_up_all_starts_only_valid_backup_and_reports_skipped_with_reason() {
    run_isolated_test(
        "ac_m3_08_6_back_up_all_starts_only_valid_backup_and_reports_skipped_with_reason",
        "KESTIC_M3_08_6_INNER",
        |root| {
            let (notif, _nc, sni, _upower, _nm) = start_fakes(false);
            let variant = latest_variant();
            // Scheduler on: the skipped entries of "Back up all" must survive scheduler ticks.
            let rig = start_rig(root, &variant, "M3M6", true);
            let client = &rig.daemon.client;
            let sni_service = common::wait_for_sni_service(&sni, Duration::from_secs(5));

            let ok_fx = rig.add_manual("GoodBk");
            let missing_fx = rig.add_manual("MissingSrc");
            let err_fx = rig.add_manual("ErrBk");
            fs::remove_dir_all(&missing_fx.src_dir).expect("remove source");
            FileSecretStore::new(root.join("secrets.hex"))
                .set_secret(
                    err_fx.dest.id.as_str(),
                    SECRET_KIND_REPO,
                    &SecretValue::from_str_secret("wrong-password"),
                )
                .expect("wrong password");
            rig.run_to(&err_fx, RunStatus::Error);
            notif.clear();

            let res = client.start_all_backups().expect("StartAllBackups");
            // (a) only the valid backup starts.
            assert_eq!(
                res.started_run_ids.len(),
                1,
                "(a) only GoodBk starts: {res:?}"
            );
            wait_for_state(client, Duration::from_secs(30), "GoodBk done", |s| {
                backup_summary(s, &ok_fx.backup.id)
                    .last_run
                    .as_ref()
                    .is_some_and(|r| r.status == RunStatus::Ok)
            });
            // (b) the two skipped backups are named with their reason.
            let skipped_ids: Vec<&str> = res.skipped.iter().map(|s| s.backup_id.as_str()).collect();
            assert_eq!(res.skipped.len(), 2, "(b) two skipped: {res:?}");
            assert!(
                skipped_ids.contains(&missing_fx.backup_id_str.as_str())
                    && skipped_ids.contains(&err_fx.backup_id_str.as_str()),
                "(b) skipped ids: {res:?}"
            );
            let reason_of = |id: &str| {
                res.skipped
                    .iter()
                    .find(|s| s.backup_id == id)
                    .map(|s| (s.reason.clone(), s.reason_text.clone()))
                    .expect("skipped entry")
            };
            assert_eq!(
                reason_of(&missing_fx.backup_id_str).0,
                "source_unavailable",
                "(b) reason of the missing source"
            );
            assert_eq!(
                reason_of(&err_fx.backup_id_str).0,
                "permanent_error",
                "(b) reason of the permanent error"
            );
            // (b) "also in the history": still listed after more than one scheduler tick.
            std::thread::sleep(Duration::from_millis(2500));
            let history = client.get_state().expect("GetState").recent_runs;
            assert!(
                history
                    .iter()
                    .any(|r| r.backup_id() == Some(&missing_fx.backup.id)
                        && r.summary_message
                            .as_deref()
                            .is_some_and(|m| m.contains("Skipped"))),
                "(b) history lists the skipped backup with the missing source: {history:?}"
            );
            assert!(
                history.iter().any(|r| r
                    .summary_message
                    .as_deref()
                    .is_some_and(|m| m.contains("ErrBk") && m.contains("Skipped"))),
                "(b) history lists the skipped backup with the permanent error: {history:?}"
            );

            // The tray entry "Back up all" produces the same message as a notification.
            let items = fetch_dbusmenu_items(client.connection(), &sni_service);
            let start_all = items
                .iter()
                .find(|m| {
                    m.label.to_lowercase().contains("back up all") || m.label == "All Backups"
                })
                .unwrap_or_else(|| panic!("tray has a 'Back up all' entry: {items:?}"));
            click_dbusmenu_item(client.connection(), &sni_service, start_all.id);
            let deadline = Instant::now() + Duration::from_secs(5);
            while !notif
                .snapshot()
                .iter()
                .any(|n| n.body.contains("MissingSrc") && n.body.contains("ErrBk"))
            {
                assert!(
                    Instant::now() < deadline,
                    "(b) tray 'Back up all' must notify the skipped backups: {:?}",
                    notif.snapshot()
                );
                std::thread::sleep(Duration::from_millis(50));
            }
            let body = notif
                .snapshot()
                .into_iter()
                .find(|n| n.body.contains("MissingSrc"))
                .map(|n| n.body)
                .expect("skipped notification");
            assert!(
                body.contains(&reason_of(&missing_fx.backup_id_str).1)
                    && body.contains(&reason_of(&err_fx.backup_id_str).1),
                "(b) notification names the reasons: {body:?}"
            );
            rig.daemon.stop();
        },
    );
}
