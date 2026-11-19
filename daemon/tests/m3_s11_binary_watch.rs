// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! M3-S11 package update: the daemon notices a replaced binary and restarts itself after a running
//! backup (`AC-M3-11-1`, `AC-M3-11-2`, `AC-M3-11-3`).

mod common;

use common::m3::{
    DropKillChild, backup_summary, latest_variant, wait_for_state, write_incompressible_dataset,
};
use common::{find_kestic_binary, run_isolated_test, start_fake_sni_watcher, wait_for_file};
use kestic_core::model::{
    Backup, BackupId, Destination, DestinationKind, ResticProvider, RunStatus, Schedule, Source,
};
use kestic_daemon::dbus_api::{KESTIC_OBJECT_PATH, KesticClient};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

/// AC-M3-11-1 (idle: rename of the binary restarts the daemon), AC-M3-11-2 (busy: the running
/// backup completes first, the restart follows), AC-M3-11-3 (negative: a sibling file in the same
/// directory does not restart the daemon). One story-level scenario so the large binary is copied
/// only once per replacement.
#[test]
fn ac_m3_11_1_to_11_3_binary_replacement_restarts_daemon_idle_or_after_backup() {
    run_isolated_test(
        "ac_m3_11_1_to_11_3_binary_replacement_restarts_daemon_idle_or_after_backup",
        "KESTIC_M3_S11_INNER",
        |root| {
            let _sni = start_fake_sni_watcher();
            let bin_dir = root.join("bin");
            fs::create_dir_all(&bin_dir).expect("mkdir bin");
            let real_bin = find_kestic_binary();
            let installed_bin = bin_dir.join("kestic");
            let sibling_file = bin_dir.join("other_helper");
            fs::copy(&real_bin, &installed_bin).expect("copy kestic binary");
            fs::set_permissions(&installed_bin, fs::Permissions::from_mode(0o755))
                .expect("chmod 755");
            fs::write(&sibling_file, b"v1").expect("write sibling");

            let cfg_path = root.join("cfg/kestic/config.toml");
            let state_dir = root.join("state/kestic");
            let ready_file = root.join("daemon.ready");
            let bus_name = "net.daester.Kestic.M3S11";
            let restic_bin = latest_variant().bin;

            let child = Command::new(&installed_bin)
                .args([
                    "daemon",
                    "--no-monitors",
                    "--no-scheduler",
                    "--no-job-view",
                    "--no-portal-inhibit",
                    "--suppress-ui-spawn",
                    "--bus-name",
                    bus_name,
                    "--config-path",
                    &cfg_path.to_string_lossy(),
                    "--state-dir",
                    &state_dir.to_string_lossy(),
                    "--ready-file",
                    &ready_file.to_string_lossy(),
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn installed kestic --daemon");

            let child_guard = DropKillChild(child);
            wait_for_file(&ready_file, Duration::from_secs(10));
            let pid_before = child_guard.0.id();

            // AC-M3-11-3: Rename sibling file in same directory -> NO restart!
            fs::remove_file(&ready_file).expect("rm ready");
            let sibling_new = bin_dir.join("other_helper.new");
            fs::write(&sibling_new, b"v2").expect("write sibling new");
            fs::rename(&sibling_new, &sibling_file).expect("rename sibling");
            thread::sleep(Duration::from_millis(800));
            assert!(
                !ready_file.exists(),
                "AC-M3-11-3: sibling file rename must NOT restart daemon"
            );

            // AC-M3-11-1: Rename kestic binary while idle -> restarts itself via exec() in same PID!
            let bin_new = bin_dir.join("kestic.new");
            fs::copy(&real_bin, &bin_new).expect("copy new bin");
            fs::set_permissions(&bin_new, fs::Permissions::from_mode(0o755)).expect("chmod 755");
            fs::rename(&bin_new, &installed_bin).expect("rename kestic binary");
            wait_for_file(&ready_file, Duration::from_secs(10));
            assert_eq!(child_guard.0.id(), pid_before);

            // AC-M3-11-2: Busy backup delays restart until completion, then restarts!
            let client = KesticClient::connect(bus_name, KESTIC_OBJECT_PATH).expect("connect");
            client
                .set_restic_provider(&ResticProvider::Custom { path: restic_bin })
                .expect("set restic");
            let repo_dir = root.join("repo");
            let src_dir = root.join("src");
            fs::create_dir_all(&repo_dir).expect("mkdir repo");
            write_incompressible_dataset(&src_dir, 96, 4 * 1024 * 1024);

            let dest =
                Destination::new("LocalS11", DestinationKind::Local { path: repo_dir }).expect("d");
            client
                .add_destination(&dest, "init", "pw-s11", "")
                .expect("init");
            let mut bk =
                Backup::new("BusyBk", Source::folder(&src_dir), dest.id.clone()).expect("bk");
            bk.schedule = Schedule::ManualOnly;
            let bid_str = client.add_backup(&bk).expect("add");
            let bid = BackupId::parse(&bid_str).expect("bid");

            fs::remove_file(&ready_file).expect("rm ready before busy backup");
            let _rid = client.start_backup(&bid_str).expect("start busy bk");
            wait_for_state(&client, Duration::from_secs(5), "busy running", |s| {
                backup_summary(s, &bid).active_run.is_some()
            });

            // Replace binary mid-backup -> must NOT interrupt running backup!
            fs::copy(&real_bin, &bin_new).expect("copy bin v3");
            fs::set_permissions(&bin_new, fs::Permissions::from_mode(0o755)).expect("chmod 755");
            fs::rename(&bin_new, &installed_bin).expect("rename v3");
            thread::sleep(Duration::from_millis(650));

            wait_for_file(&ready_file, Duration::from_secs(20));
            let client_after =
                KesticClient::connect(bus_name, KESTIC_OBJECT_PATH).expect("reconnect");
            let s_after = client_after.get_state().expect("GetState after exec");
            let last = backup_summary(&s_after, &bid)
                .last_run
                .as_ref()
                .expect("last run");
            assert_eq!(
                last.status,
                RunStatus::Ok,
                "AC-M3-11-2: busy backup must complete Ok before daemon self-exec restart"
            );
            client_after.confirm_quit().expect("quit");
        },
    );
}
