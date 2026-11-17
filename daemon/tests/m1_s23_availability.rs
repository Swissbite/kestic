// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

mod common;

use common::{
    RestServerGuard, fetch_dbusmenu_items, find_kestic_binary, restic_test_binaries,
    run_isolated_test, start_fake_notifications, start_fake_sni_watcher, wait_for_file,
    wait_for_sni_service,
};
use kestic_core::clock::FakeClock;
use kestic_core::config::{AppConfig, save_config};
use kestic_core::model::{Backup, Destination, DestinationKind, ResticProvider, Source};
use kestic_core::secrets::FileSecretStore;
use kestic_daemon::dbus_api::{
    KESTIC_BUS_NAME, KesticClient, REASON_DESTINATION_UNREACHABLE, REASON_SOURCE_UNAVAILABLE,
};
use kestic_daemon::service::{DaemonCore, DaemonOutcome, DaemonRunConfig, run_daemon};
use std::fs;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use time::macros::datetime;
use zbus::{Connection, block_on};

#[test]
fn ac_m1_23_3_daemon_core_availability_cache_respects_60s_fake_clock_ttl() {
    run_isolated_test(
        "ac_m1_23_3_daemon_core_availability_cache_respects_60s_fake_clock_ttl",
        "KESTIC_M1_S23_CORE_CACHE",
        |root| {
            let rest_data = root.join("rest-data");
            let mut server = RestServerGuard::start_with_auth(&rest_data);
            let port = server.port;

            let cfg_path = root.join("cfg").join("kestic").join("config.toml");
            let state_dir = root.join("state").join("kestic");
            let secret_file = root.join("secrets.hex");
            let src_dir = root.join("src");
            fs::create_dir_all(&src_dir).expect("mkdir src");

            let dest = Destination::new(
                "REST Cache Repo",
                DestinationKind::Rest {
                    url: server.url.clone(),
                    username: Some("alice".to_string()),
                },
            )
            .expect("dest");
            let backup =
                Backup::new("Docs", Source::folder(&src_dir), dest.id.clone()).expect("backup");
            let (_, restic_bin) = restic_test_binaries().pop().expect("restic binary");

            save_config(
                &cfg_path,
                &AppConfig {
                    restic_provider: ResticProvider::Custom { path: restic_bin },
                    destinations: vec![dest],
                    backups: vec![backup],
                    ..Default::default()
                },
            )
            .expect("save config");

            let clock = Arc::new(FakeClock::new(datetime!(2026-11-16 12:00:00 UTC)));
            let run_cfg = DaemonRunConfig {
                config_path: Some(cfg_path),
                state_dir: Some(state_dir),
                kestic_binary: Some(find_kestic_binary()),
                clock: clock.clone(),
                secret_store: Some(Arc::new(FileSecretStore::new(secret_file))),
                suppress_ui_spawn: true,
                enable_job_view: false,
                enable_portal_inhibit: false,
                ..Default::default()
            };

            let mut core = DaemonCore::new(&run_cfg).expect("DaemonCore::new");
            // Startup performed 1 probe (reachable).
            assert_eq!(core.availability_probe_count(), 1);
            let snap1 = core.refresh_availability_if_needed(false);
            assert!(snap1.destinations[0].reachable);
            assert_eq!(
                core.availability_probe_count(),
                1,
                "second check within 0 s must reuse cached result"
            );

            // Stop server and check within 60 s -> STILL cached, probe count unchanged!
            server.stop();
            clock.advance(Duration::from_secs(30));
            let snap2 = core.refresh_availability_if_needed(false);
            assert!(
                snap2.destinations[0].reachable,
                "check at +30 s must still return cached result without probing"
            );
            assert_eq!(core.availability_probe_count(), 1);

            // Advance past 60 s (> 60 s) -> probes again and detects unreachable!
            clock.advance(Duration::from_secs(31));
            let snap3 = core.refresh_availability_if_needed(false);
            assert_eq!(core.availability_probe_count(), 2);
            assert!(!snap3.destinations[0].reachable);
            assert!(!snap3.backups[0].available);
            assert_eq!(
                snap3.backups[0].blocked_reason.as_deref(),
                Some(REASON_DESTINATION_UNREACHABLE)
            );

            // Restart server on the same port: within 60 s stays cached unreachable; after >60 s recovers!
            let _restarted =
                RestServerGuard::start_on_port(&rest_data, port, Some(("alice", "secret")));
            clock.advance(Duration::from_secs(45));
            let snap4 = core.refresh_availability_if_needed(false);
            assert_eq!(core.availability_probe_count(), 2);
            assert!(!snap4.destinations[0].reachable);

            clock.advance(Duration::from_secs(20));
            let snap5 = core.refresh_availability_if_needed(false);
            assert_eq!(core.availability_probe_count(), 3);
            assert!(snap5.destinations[0].reachable);
            assert!(snap5.backups[0].available);
        },
    );
}

#[test]
fn ac_m1_23_1_23_2_23_3_23_4_dbus_and_tray_availability_updates_with_rest_server() {
    run_isolated_test(
        "ac_m1_23_1_23_2_23_3_23_4_dbus_and_tray_availability_updates_with_rest_server",
        "KESTIC_M1_S23_DBUS_TRAY",
        |root| {
            let watcher_conn = start_fake_sni_watcher();
            let _notif_guard = start_fake_notifications();
            let client_conn = block_on(Connection::session()).expect("session conn");

            let rest_data = root.join("rest-data");
            let mut server = RestServerGuard::start_with_auth(&rest_data);
            let port = server.port;

            let src_dir = root.join("src-avail");
            fs::create_dir_all(&src_dir).expect("mkdir src");
            let (_, restic_bin) = restic_test_binaries().pop().expect("restic binary");

            let clock = Arc::new(FakeClock::new(datetime!(2026-11-16 14:00:00 UTC)));
            let ready_file = root.join("daemon.ready");
            let run_cfg = DaemonRunConfig {
                bus_name: KESTIC_BUS_NAME.to_string(),
                assume_sni_available: false,
                ready_file: Some(ready_file.clone()),
                config_path: Some(root.join("cfg").join("kestic").join("config.toml")),
                state_dir: Some(root.join("state").join("kestic")),
                kestic_binary: Some(find_kestic_binary()),
                clock: clock.clone(),
                secret_store: Some(Arc::new(FileSecretStore::new(root.join("secrets.hex")))),
                suppress_ui_spawn: true,
                enable_job_view: false,
                enable_portal_inhibit: false,
                ..Default::default()
            };

            let handle = thread::spawn(move || run_daemon(run_cfg));
            wait_for_file(&ready_file, Duration::from_secs(5));
            let sni_svc = wait_for_sni_service(&watcher_conn, Duration::from_secs(5));

            let client = KesticClient::connect_default().expect("connect client");
            client
                .set_restic_provider(&ResticProvider::Custom { path: restic_bin })
                .expect("set provider");

            let dest = Destination::new(
                "NAS REST",
                DestinationKind::Rest {
                    url: server.url.clone(),
                    username: Some("alice".to_string()),
                },
            )
            .expect("dest");
            client
                .add_destination(&dest, "init", "rest-repo-pw", "secret")
                .expect("AddDestination REST");

            let backup = Backup::new("REST Backup", Source::folder(&src_dir), dest.id.clone())
                .expect("backup");
            let backup_id = client.add_backup(&backup).expect("AddBackup");

            // Initially reachable & enabled in tray menu.
            let avail_ok = client.check_availability(false).expect("CheckAvailability");
            assert!(avail_ok.destinations[0].reachable);
            assert!(avail_ok.backups[0].available);
            let menu_ok = fetch_dbusmenu_items(&client_conn, &sni_svc);
            let entry_ok = menu_ok
                .iter()
                .find(|m| m.label.starts_with("REST Backup"))
                .expect("tray item for REST Backup");
            assert!(entry_ok.enabled, "backup must be enabled when reachable");

            // AC-M1-23-1 & AC-M1-23-4: Stop rest-server, advance >60s, CheckAvailability(false)
            // marks destination unreachable, disables tray menu item, and blocks StartBackup.
            server.stop();
            clock.advance(Duration::from_secs(61));
            let avail_down = client.check_availability(false).expect("CheckAvailability");
            assert!(!avail_down.destinations[0].reachable);
            assert!(!avail_down.backups[0].available);
            assert_eq!(
                avail_down.backups[0].blocked_reason.as_deref(),
                Some(REASON_DESTINATION_UNREACHABLE)
            );

            let menu_down = fetch_dbusmenu_items(&client_conn, &sni_svc);
            let entry_down = menu_down
                .iter()
                .find(|m| m.label.starts_with("REST Backup"))
                .expect("tray item for REST Backup");
            assert!(
                !entry_down.enabled,
                "tray menu item must be disabled when destination is unreachable"
            );

            let err = client
                .start_backup(&backup_id)
                .expect_err("StartBackup must block when destination unreachable");
            match err {
                zbus::Error::MethodError(name, _, _) => {
                    assert_eq!(name.as_str(), "net.daester.Kestic.Error.ManualRunBlocked");
                }
                other => panic!("expected ManualRunBlocked error, got {other:?}"),
            }

            // AC-M1-23-3: Restart rest-server; within 60 s cache holds unreachable!
            let _restarted =
                RestServerGuard::start_on_port(&rest_data, port, Some(("alice", "secret")));
            clock.advance(Duration::from_secs(30));
            client
                .window_active(true, "overview")
                .expect("WindowActive");
            let avail_cached = client.check_availability(false).expect("CheckAvailability");
            assert!(
                !avail_cached.destinations[0].reachable,
                "cache within 60 s must not re-probe yet"
            );

            // Advance past 60 s (> 60 s) and call tray AboutToShow + GetLayout -> marks available again!
            clock.advance(Duration::from_secs(35));
            let menu_recovered = fetch_dbusmenu_items(&client_conn, &sni_svc);
            let entry_rec = menu_recovered
                .iter()
                .find(|m| m.label.starts_with("REST Backup"))
                .expect("tray item for REST Backup");
            assert!(
                entry_rec.enabled,
                "AboutToShow after >60 s must refresh availability and re-enable backup in tray"
            );

            let state_rec = client.get_state().expect("GetState");
            assert!(state_rec.destinations[0].availability.reachable);
            assert!(state_rec.backups[0].availability.available);

            // AC-M1-23-2 & AC-M1-23-4: Remove source folder, advance > 60 s, CheckAvailability(false)
            // marks source_unavailable and disables tray menu item.
            fs::remove_dir_all(&src_dir).expect("remove source dir");
            clock.advance(Duration::from_secs(61));
            let avail_nosrc = client.check_availability(false).expect("CheckAvailability");
            assert!(!avail_nosrc.backups[0].source_available);
            assert!(!avail_nosrc.backups[0].available);
            assert_eq!(
                avail_nosrc.backups[0].blocked_reason.as_deref(),
                Some(REASON_SOURCE_UNAVAILABLE)
            );

            let menu_nosrc = fetch_dbusmenu_items(&client_conn, &sni_svc);
            let entry_nosrc = menu_nosrc
                .iter()
                .find(|m| m.label.starts_with("REST Backup"))
                .expect("tray item");
            assert!(
                !entry_nosrc.enabled,
                "tray menu item must be disabled when source is unavailable"
            );

            client.confirm_quit().expect("confirm quit");
            assert_eq!(
                handle.join().expect("join daemon"),
                Ok(DaemonOutcome::RanAndStopped)
            );
        },
    );
}
