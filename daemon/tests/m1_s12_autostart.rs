// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

mod common;

use common::{
    find_kestic_binary, restic_test_binaries, run_isolated_test, start_fake_sni_watcher,
    wait_for_file, wait_for_sni_service,
};
use kestic_core::config::load_config;
use kestic_core::model::{Backup, Destination, DestinationKind, ResticProvider, Source};
use kestic_daemon::dbus_api::KesticClient;
use std::fs;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn ac_m1_12_1_12_2_12_3_daemon_autostart_lifecycle() {
    run_isolated_test(
        "ac_m1_12_1_12_2_12_3_daemon_autostart_lifecycle",
        "KESTIC_M1_S12_AUTOSTART",
        |root| {
            let kestic_bin = find_kestic_binary();
            let (_, restic_bin) = restic_test_binaries().pop().expect("restic binary");
            let cfg_path = root.join("cfg").join("kestic").join("config.toml");

            // AC-M1-12-1: No config.toml + `kestic daemon --autostart` -> exits 0 immediately without claiming bus name.
            assert!(!cfg_path.exists());
            let start_t = Instant::now();
            let autostart_empty = Command::new(&kestic_bin)
                .args([
                    "daemon",
                    "--autostart",
                    "--no-job-view",
                    "--no-portal-inhibit",
                    "--suppress-ui-spawn",
                ])
                .output()
                .expect("run kestic daemon --autostart without config");
            assert!(
                autostart_empty.status.success(),
                "autostart without config must exit 0, got {:?}",
                autostart_empty.status.code()
            );
            assert!(
                start_t.elapsed() < Duration::from_secs(3),
                "autostart without config must exit immediately"
            );

            let client = KesticClient::connect_default().expect("connect client");
            assert!(
                !client.is_bus_owned().expect("is_bus_owned"),
                "autostart without config must not claim bus name"
            );

            // AC-M1-12-2: No config.toml + `kestic daemon` (no `--autostart`) -> claims bus name & accepts CRUD.
            let ready_no_auto = root.join("daemon-manual.ready");
            let manual_child = Command::new(&kestic_bin)
                .args([
                    "daemon",
                    "--no-job-view",
                    "--no-portal-inhibit",
                    "--suppress-ui-spawn",
                    "--ready-file",
                ])
                .arg(&ready_no_auto)
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .expect("spawn kestic daemon without --autostart");

            wait_for_file(&ready_no_auto, Duration::from_secs(5));
            assert!(client.is_bus_owned().expect("is_bus_owned"));

            let init_state = client.get_state().expect("GetState");
            assert!(!init_state.has_config_file);

            client
                .set_restic_provider(&ResticProvider::Custom { path: restic_bin })
                .expect("SetResticProvider");

            let repo_dir = root.join("repo-autostart");
            let src_dir = root.join("src-autostart");
            fs::create_dir_all(&repo_dir).expect("mkdir repo");
            fs::create_dir_all(&src_dir).expect("mkdir src");

            let dest =
                Destination::new("Autostart Repo", DestinationKind::Local { path: repo_dir })
                    .expect("dest");
            client
                .add_destination(&dest, "init", "autostart-pw", "")
                .expect("AddDestination");
            let backup = Backup::new("Docs", Source::folder(&src_dir), dest.id).expect("backup");
            client.add_backup(&backup).expect("AddBackup");

            assert!(cfg_path.is_file(), "daemon must persist config.toml");
            assert_eq!(
                load_config(&cfg_path).expect("load config").backups.len(),
                1
            );

            client.confirm_quit().expect("ConfirmQuit manual daemon");
            let manual_out = manual_child.wait_with_output().expect("wait manual daemon");
            assert!(
                manual_out.status.success(),
                "manual daemon exited with error: {}",
                String::from_utf8_lossy(&manual_out.stderr)
            );
            while client.is_bus_owned().expect("is_bus_owned") {
                std::thread::sleep(std::time::Duration::from_millis(15));
            }

            // AC-M1-12-3: Valid config.toml + `kestic daemon --autostart` -> stays running, owns bus name, registers SNI tray.
            let watcher_conn = start_fake_sni_watcher();
            let ready_auto = root.join("daemon-autostart.ready");
            let mut auto_child = Command::new(&kestic_bin)
                .args([
                    "daemon",
                    "--autostart",
                    "--require-sni",
                    "--no-job-view",
                    "--no-portal-inhibit",
                    "--suppress-ui-spawn",
                    "--ready-file",
                ])
                .arg(&ready_auto)
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .expect("spawn kestic daemon --autostart with valid config");

            wait_for_file(&ready_auto, Duration::from_secs(5));
            assert!(
                auto_child
                    .try_wait()
                    .expect("try_wait autostart daemon")
                    .is_none(),
                "autostart daemon with config.toml must stay running"
            );
            assert!(client.is_bus_owned().expect("is_bus_owned"));

            let sni_svc = wait_for_sni_service(&watcher_conn, Duration::from_secs(5));
            assert!(
                sni_svc.starts_with("org.kde.StatusNotifierItem-"),
                "expected SNI registration, got {sni_svc}"
            );

            let state_after = client.get_state().expect("GetState");
            assert!(state_after.has_config_file);
            assert_eq!(state_after.backups.len(), 1);

            client.confirm_quit().expect("ConfirmQuit autostart daemon");
            let auto_out = auto_child
                .wait_with_output()
                .expect("wait autostart daemon");
            assert!(
                auto_out.status.success(),
                "autostart daemon exited with error: {}",
                String::from_utf8_lossy(&auto_out.stderr)
            );
        },
    );
}
