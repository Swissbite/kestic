// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Integration tests for M5-S06b: installing the restic managed by Kestic through the daemon
//! (`StartResticInstall`, `GetResticInstallStatus`, `CancelResticInstall`) from a local release
//! mock signed with a test key (feature `test-trust-anchor` of `kestic-core`).

mod common;

use common::m3::{M3Rig, assert_dbus_error_name, latest_variant, restic_variants, wait_for_state};
use common::release::{serve, trust};
use common::{SYSTEM_PROGRAMS_DIR, run_isolated_test, run_isolated_test_with_system_programs};
use kestic_core::model::ResticProvider;
use kestic_core::restic_install::{TEMP_PREFIX, app_managed_binary, kestic_data_dir};
use kestic_daemon::dbus_api::{KESTIC_OBJECT_PATH, KesticClient, ResticInstallStatusDto};
use kestic_i18n::codes::ResticSourceCode;
use kestic_i18n::codes::{
    ResticInstallCode, ResticInstallOutcome, ResticInstallPhase, ServiceErrorCode,
};
use kestic_test_support::config::load_config;
use kestic_test_support::release_mock::{
    LATEST_PATH, ReleaseFiles, Route, TestSigningKey, asset_path, restic_stand_in_archive,
};
use kestic_test_support::tools::{free_loopback_port, write_executable};
use kestic_test_support::wait::{wait_for_value, wait_until};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use time::OffsetDateTime;
use time::macros::datetime;

const START: OffsetDateTime = datetime!(2026-12-01 10:00:00 UTC);
/// Newer than every real restic the tests run, so it is never older than the installed one.
const RELEASE: &str = "0.99.0";
const INSTALL_TIMEOUT: Duration = Duration::from_secs(30);

/// Kestic's data folder of this isolated test (`$XDG_DATA_HOME/kestic`).
fn data_dir() -> PathBuf {
    kestic_data_dir(
        std::env::var_os("HOME").as_deref(),
        std::env::var_os("XDG_DATA_HOME").as_deref(),
    )
    .expect("data dir")
}

fn version_script(version: &str) -> String {
    format!("#!/bin/sh\necho '{{\"message_type\":\"version\",\"version\":\"{version}\"}}'\n")
}

/// Puts a restic stand-in reporting `version` in place as the app-managed binary; returns its
/// bytes and inode.
fn app_managed_stand_in(version: &str) -> (Vec<u8>, u64) {
    let dir = data_dir();
    fs::create_dir_all(&dir).expect("data dir");
    let binary = app_managed_binary(&dir);
    let _ = fs::remove_file(&binary);
    write_executable(&binary, &version_script(version));
    binary_identity(&binary)
}

fn binary_identity(binary: &Path) -> (Vec<u8>, u64) {
    (
        fs::read(binary).expect("read binary"),
        fs::metadata(binary).expect("binary").ino(),
    )
}

fn reported_version(binary: &Path) -> String {
    let out = Command::new(binary)
        .args(["version", "--json"])
        .output()
        .expect("run version --json");
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).expect("version JSON");
    json["version"].as_str().expect("version").to_string()
}

/// Names in the data folder left by an installation.
fn leftovers() -> Vec<String> {
    let Ok(entries) = fs::read_dir(data_dir()) else {
        return Vec::new();
    };
    entries
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(TEMP_PREFIX))
        .collect()
}

fn status(client: &KesticClient, id: &str) -> ResticInstallStatusDto {
    client
        .get_restic_install_status(id)
        .expect("GetResticInstallStatus")
}

/// Polls `GetResticInstallStatus` until `pred` holds.
fn wait_install(
    client: &KesticClient,
    id: &str,
    timeout: Duration,
    what: &str,
    pred: impl Fn(&ResticInstallStatusDto) -> bool,
) -> ResticInstallStatusDto {
    wait_for_value(timeout, what, || status(client, id), |st| pred(st))
}

fn wait_finished(client: &KesticClient, id: &str) -> ResticInstallStatusDto {
    wait_install(client, id, INSTALL_TIMEOUT, "the end", |st| {
        st.state != "running"
    })
}

/// A daemon with the wrapper of the latest restic as its restic source, no scheduler.
fn quiet_rig(root: &Path, base: &str) -> M3Rig {
    M3Rig::start(root, &latest_variant(), base, START, |o| {
        o.enable_monitors = false;
        o.enable_scheduler = false;
    })
}

/// AC-M5-06-6: without a reachable release server the installation ends with an
/// understandable error; the restic source and the app-managed binary stay as they were.
#[test]
fn ac_m5_06_6_unreachable_release_server_keeps_setting_and_binary() {
    run_isolated_test(
        "ac_m5_06_6_unreachable_release_server_keeps_setting_and_binary",
        "KESTIC_M5_06_6_INNER",
        |root| {
            let rig = quiet_rig(root, "S06B6");
            let client = &rig.daemon.client;
            let key = TestSigningKey::generate("kestic-test-release");
            trust(&format!("http://127.0.0.1:{}", free_loopback_port()), &key);
            let old = app_managed_stand_in("0.18.0");
            let config_before = load_config(&rig.daemon.config_path).expect("config");
            let restic_before = client.get_state().expect("state").restic_status;

            let id = client.start_restic_install().expect("StartResticInstall");
            let end = wait_finished(client, &id);

            // (a) an error with the code for a failed download and its text
            assert_eq!(end.state, "error");
            assert_eq!(end.error_kind, Some(ResticInstallCode::Network));
            assert!(
                end.error_detail.as_deref().is_some_and(|d| !d.is_empty()),
                "{end:?}"
            );
            assert_eq!(
                ResticInstallCode::Network.message().english(),
                "Could not download restic. Check the internet connection and try again."
            );
            // (b) the restic source is unchanged
            let config_after = load_config(&rig.daemon.config_path).expect("config");
            assert_eq!(config_after.restic_provider, config_before.restic_provider);
            assert_eq!(
                client.get_state().expect("state").restic_status,
                restic_before
            );
            // (c) the app-managed binary is unchanged and nothing is left behind
            let binary = app_managed_binary(&data_dir());
            assert_eq!(binary_identity(&binary), old);
            assert_eq!(leftovers(), Vec::<String>::new());
            rig.daemon.stop();
        },
    );
}

/// AC-M5-06-8: with a backup running on the app-managed restic, the new version is downloaded
/// and checked at once, but replaces the binary only after the run has ended.
#[test]
fn ac_m5_06_8_install_waits_for_running_backup() {
    run_isolated_test(
        "ac_m5_06_8_install_waits_for_running_backup",
        "KESTIC_M5_06_8_INNER",
        |root| {
            let key = TestSigningKey::generate("kestic-test-release");
            let files = ReleaseFiles::signed(RELEASE, &key);
            for variant in restic_variants() {
                let rig = M3Rig::start(root, &variant, "S06B8", START, |o| {
                    o.enable_monitors = false;
                    o.enable_scheduler = false;
                });
                let client = &rig.daemon.client;
                let fx = rig.add_backup("Alpha", |_| {});
                // The wrapper (it logs and can hold restic calls) is the app-managed restic.
                let dir = data_dir();
                fs::create_dir_all(&dir).expect("data dir");
                let binary = app_managed_binary(&dir);
                let _ = fs::remove_file(&binary);
                fs::copy(&rig.wrapper.path, &binary).expect("copy wrapper");
                let source = client
                    .set_restic_provider(&ResticProvider::AppManaged)
                    .expect("SetResticProvider");
                assert!(source.valid, "{}: {source:?}", variant.label);
                let old_inode = fs::metadata(&binary).expect("binary").ino();

                rig.wrapper.hold("backup");
                client.start_backup(&fx.backup_id_str).expect("StartBackup");
                rig.wrapper
                    .wait_for('S', "backup", 1, Duration::from_secs(20));
                let mock = serve(&files, &key, Vec::new());

                let id = client.start_restic_install().expect("StartResticInstall");
                let waiting =
                    wait_install(client, &id, INSTALL_TIMEOUT, "waiting_for_runs", |st| {
                        st.phase == ResticInstallPhase::WaitingForRuns || st.state != "running"
                    });
                // (a) downloaded and checked while the backup runs
                assert_eq!(waiting.state, "running", "{}: {waiting:?}", variant.label);
                assert_eq!(waiting.phase, ResticInstallPhase::WaitingForRuns);
                assert_eq!(waiting.version.as_deref(), Some(RELEASE));
                let requests = mock.requests();
                for path in [
                    LATEST_PATH.to_string(),
                    asset_path(RELEASE, "SHA256SUMS"),
                    asset_path(RELEASE, "SHA256SUMS.asc"),
                    asset_path(RELEASE, &files.archive_name("amd64")),
                ] {
                    assert!(
                        requests.contains(&path),
                        "{}: {path} in {requests:?}",
                        variant.label
                    );
                }
                // (b) not replaced while the backup runs
                // quiet window: the install keeps waiting for the run
                std::thread::sleep(Duration::from_millis(800));
                assert_eq!(
                    status(client, &id).phase,
                    ResticInstallPhase::WaitingForRuns
                );
                assert_eq!(fs::metadata(&binary).expect("binary").ino(), old_inode);
                assert_eq!(rig.wrapper.of('E', "backup").len(), 0);

                // (b) nor between the restic processes of that run: the backup ends, the
                // run's `forget` follows and is held
                rig.wrapper.hold("forget");
                rig.wrapper.release("backup");
                rig.wrapper
                    .wait_for('S', "forget", 1, Duration::from_secs(20));
                assert_eq!(rig.wrapper.of('E', "backup").len(), 1);
                // quiet window: the install keeps waiting between the restic processes
                std::thread::sleep(Duration::from_millis(500));
                assert_eq!(
                    status(client, &id).phase,
                    ResticInstallPhase::WaitingForRuns
                );
                assert_eq!(fs::metadata(&binary).expect("binary").ino(), old_inode);

                rig.wrapper.release("forget");
                let end = wait_finished(client, &id);
                assert_eq!(end.state, "done", "{}: {end:?}", variant.label);
                assert_eq!(end.outcome, Some(ResticInstallOutcome::Installed));
                // (b) replaced only after the whole run had ended
                let state = client.get_state().expect("state");
                let summary = state
                    .backups
                    .iter()
                    .find(|b| b.backup.id == fx.backup.id)
                    .expect("backup");
                assert_eq!(summary.active_run, None, "{}", variant.label);
                assert_eq!(rig.wrapper.of('E', "backup").len(), 1);
                assert_ne!(fs::metadata(&binary).expect("binary").ino(), old_inode);
                assert_eq!(reported_version(&binary), RELEASE);
                assert_eq!(leftovers(), Vec::<String>::new());
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M5-06-8: an installation that waits for a running backup can still be cancelled; the
/// binary stays unchanged, also after the backup has ended.
#[test]
fn ac_m5_06_8_cancel_while_waiting_for_runs_keeps_binary() {
    run_isolated_test(
        "ac_m5_06_8_cancel_while_waiting_for_runs_keeps_binary",
        "KESTIC_M5_06_8C_INNER",
        |root| {
            let rig = quiet_rig(root, "S06B8C");
            let client = &rig.daemon.client;
            let fx = rig.add_backup("Alpha", |_| {});
            let old = app_managed_stand_in("0.18.0");
            let key = TestSigningKey::generate("kestic-test-release");
            let _mock = serve(&ReleaseFiles::signed(RELEASE, &key), &key, Vec::new());

            rig.wrapper.hold("backup");
            client.start_backup(&fx.backup_id_str).expect("StartBackup");
            rig.wrapper
                .wait_for('S', "backup", 1, Duration::from_secs(20));
            let id = client.start_restic_install().expect("StartResticInstall");
            wait_install(client, &id, INSTALL_TIMEOUT, "waiting_for_runs", |st| {
                st.phase == ResticInstallPhase::WaitingForRuns
            });
            assert!(
                !leftovers().is_empty(),
                "the checked binary waits in its temporary file"
            );

            client
                .cancel_restic_install(&id)
                .expect("CancelResticInstall");
            // (a) cancelled at once, while the backup still runs
            let end = wait_install(client, &id, Duration::from_secs(2), "cancelled", |st| {
                st.state == "cancelled"
            });
            assert_eq!(end.state, "cancelled");
            // (b) the temporary file goes, the binary stays
            wait_until("temporary file removed", || leftovers().is_empty());
            let binary = app_managed_binary(&data_dir());
            assert_eq!(binary_identity(&binary), old);

            rig.wrapper.release("backup");
            wait_for_state(client, Duration::from_secs(30), "backup ended", |s| {
                s.backups.iter().all(|b| b.active_run.is_none())
            });
            // quiet window: nothing replaces the binary after the run either
            std::thread::sleep(Duration::from_millis(300));
            // (c) still unchanged after the run
            assert_eq!(binary_identity(&binary), old);
            rig.daemon.stop();
        },
    );
}

/// AC-M5-06-12 (a), (c): with the source «Automatisch» and a system restic 0.16.4 (too old), the
/// daemon starts and reports without asking the release server; only `StartResticInstall`
/// does.
#[test]
fn ac_m5_06_12_no_release_access_before_start() {
    run_isolated_test_with_system_programs(
        "ac_m5_06_12_no_release_access_before_start",
        "KESTIC_M5_06_12_INNER",
        |root| {
            let key = TestSigningKey::generate("kestic-test-release");
            let mock = serve(&ReleaseFiles::signed(RELEASE, &key), &key, Vec::new());
            write_executable(
                &root.join(SYSTEM_PROGRAMS_DIR).join("restic"),
                "#!/bin/sh\nif [ \"$1\" = version ] && [ -z \"$2\" ]; then\n  \
                 echo 'restic 0.16.4 compiled with go1.22.2 on linux/amd64'\n  exit 0\nfi\nexit 1\n",
            );
            let variant = latest_variant();
            let sub = variant.sub("S06B12");
            let (config_path, _) = common::m3::daemon_paths(root, &sub);
            let config = kestic_core::config::AppConfig {
                restic_provider: ResticProvider::Automatic,
                ..Default::default()
            };
            fs::create_dir_all(config_path.parent().expect("cfg dir")).expect("cfg dir");
            kestic_core::config::save_config(&config_path, &config).expect("save config");

            let rig = M3Rig::start_on(
                root,
                &variant,
                &sub,
                &variant.bus_name("S06B12"),
                START,
                |o| {
                    o.enable_monitors = false;
                    o.enable_scheduler = false;
                    o.set_restic_provider = false;
                },
            );
            let client = &rig.daemon.client;
            let state = client.get_state().expect("state");
            assert!(!state.restic_status.valid, "{:?}", state.restic_status);
            assert_eq!(state.restic_status.found_version.as_deref(), Some("0.16.4"));
            assert_eq!(state.restic_status.source, ResticSourceCode::Automatic);
            // quiet window: the daemon asks the release server nothing by itself
            std::thread::sleep(Duration::from_secs(2));
            // (a) nothing was asked before the start
            assert_eq!(mock.requests(), Vec::<String>::new());

            // (c) the start downloads
            let id = client.start_restic_install().expect("StartResticInstall");
            let end = wait_finished(client, &id);
            assert_eq!(end.state, "done", "{end:?}");
            assert_eq!(end.outcome, Some(ResticInstallOutcome::Installed));
            assert_eq!(
                mock.requests().first().map(String::as_str),
                Some(LATEST_PATH)
            );
            assert_eq!(reported_version(&app_managed_binary(&data_dir())), RELEASE);
            rig.daemon.stop();
        },
    );
}

/// M5-S06: one installation at a time; a cancel during a stalled download ends the installation at once
/// (its worker can no longer replace the binary); finished installations stay readable for ten
/// minutes after their end; unknown ids are `NotFound`.
#[test]
fn restic_install_cancel_already_running_and_expiry() {
    run_isolated_test(
        "restic_install_cancel_already_running_and_expiry",
        "KESTIC_M5_06B_LIFECYCLE_INNER",
        |root| {
            let rig = quiet_rig(root, "S06BL");
            let client = &rig.daemon.client;
            let key = TestSigningKey::generate("kestic-test-release");
            let files = ReleaseFiles::signed(RELEASE, &key);
            let archive = asset_path(RELEASE, &files.archive_name("amd64"));
            let mock = serve(
                &files,
                &key,
                vec![(archive.clone(), Route::StallAfterHeaders)],
            );

            let not_found = client
                .get_restic_install_status("unknown")
                .expect_err("unknown id");
            // Only the code is compared and printed: an error may carry secret details.
            assert_eq!(not_found.code(), ServiceErrorCode::NotFound);
            let err = client
                .cancel_restic_install("unknown")
                .expect_err("unknown id");
            assert_dbus_error_name(&err, "net.daester.Kestic.Error.NotFound");

            let first = client.start_restic_install().expect("StartResticInstall");
            wait_until("archive requested", || mock.requests().contains(&archive));
            let downloading = status(client, &first);
            assert_eq!(downloading.phase, ResticInstallPhase::Downloading);
            // a second start while one runs
            let err = client.start_restic_install().expect_err("second start");
            assert_dbus_error_name(&err, "net.daester.Kestic.Error.AlreadyRunning");

            client.cancel_restic_install(&first).expect("cancel");
            let end = wait_install(client, &first, Duration::from_secs(2), "cancelled", |st| {
                st.state == "cancelled"
            });
            assert_eq!(end.state, "cancelled");
            assert_eq!(leftovers(), Vec::<String>::new());
            assert!(!app_managed_binary(&data_dir()).exists());

            // a new installation may start once the first has ended
            let second = client.start_restic_install().expect("start after cancel");
            assert_ne!(second, first);
            client
                .cancel_restic_install(&second)
                .expect("cancel second");

            // kept ten minutes after its end, then gone
            rig.clock.advance(Duration::from_secs(599));
            assert_eq!(status(client, &first).state, "cancelled");
            rig.clock.advance(Duration::from_secs(2));
            let gone = client
                .get_restic_install_status(&first)
                .expect_err("expired");
            assert_eq!(gone.code(), ServiceErrorCode::NotFound);
            rig.daemon.stop();
        },
    );
}

/// M5-S06: `already_current` and the errors after a download (another version, an older release) keep
/// the binary and leave nothing behind.
#[test]
fn restic_install_outcomes_and_errors_after_download() {
    run_isolated_test(
        "restic_install_outcomes_and_errors_after_download",
        "KESTIC_M5_06B_OUTCOMES_INNER",
        |root| {
            let rig = quiet_rig(root, "S06BO");
            let client = &rig.daemon.client;
            let key = TestSigningKey::generate("kestic-test-release");
            let binary = app_managed_binary(&data_dir());

            // the same version is installed: nothing but the release information is fetched
            let current = app_managed_stand_in(RELEASE);
            let mock = serve(&ReleaseFiles::signed(RELEASE, &key), &key, Vec::new());
            let id = client.start_restic_install().expect("start");
            let end = wait_finished(client, &id);
            assert_eq!(end.state, "done", "{end:?}");
            assert_eq!(end.outcome, Some(ResticInstallOutcome::AlreadyCurrent));
            assert_eq!(mock.requests(), vec![LATEST_PATH.to_string()]);
            assert_eq!(binary_identity(&binary), current);

            // the archive holds another version than its release
            let old = app_managed_stand_in("0.18.0");
            let mut files = ReleaseFiles::signed(RELEASE, &key);
            for arch in ["amd64", "arm64"] {
                files
                    .assets
                    .insert(files.archive_name(arch), restic_stand_in_archive("0.98.0"));
            }
            files.resign(&key);
            let _mismatch = serve(&files, &key, Vec::new());
            let id = client.start_restic_install().expect("start");
            let end = wait_finished(client, &id);
            assert_eq!(end.state, "error");
            assert_eq!(end.error_kind, Some(ResticInstallCode::VersionMismatch));
            assert_eq!(binary_identity(&binary), old);
            assert_eq!(leftovers(), Vec::<String>::new());

            // the release is older than the installed restic
            let newer = app_managed_stand_in("1.0.0");
            let _older = serve(&ReleaseFiles::signed(RELEASE, &key), &key, Vec::new());
            let id = client.start_restic_install().expect("start");
            let end = wait_finished(client, &id);
            assert_eq!(end.state, "error");
            assert_eq!(end.error_kind, Some(ResticInstallCode::TooOld));
            let detail = end.error_detail.unwrap_or_default();
            assert!(detail.contains(RELEASE), "{detail}");
            assert!(detail.contains("1.0.0"), "{detail}");
            assert_eq!(binary_identity(&binary), newer);
            rig.daemon.stop();
        },
    );
}

/// M5-S06: what an interrupted installation left in the data folder is removed when the daemon
/// starts; everything else stays.
#[test]
fn leftovers_removed_at_daemon_start() {
    run_isolated_test(
        "leftovers_removed_at_daemon_start",
        "KESTIC_M5_06B_LEFTOVERS_INNER",
        |root| {
            let dir = data_dir();
            fs::create_dir_all(dir.join(".restic-download-0a1b.gnupg")).expect("gnupg dir");
            fs::write(dir.join(".restic-download-0a1b.gnupg/keyring.pgp"), b"k").expect("key");
            fs::write(dir.join(".restic-download-2c3d"), b"partial").expect("temp file");
            let old = app_managed_stand_in("0.18.0");
            fs::write(dir.join("known_hosts"), b"hosts").expect("known_hosts");

            let rig = quiet_rig(root, "S06BC");
            // (a) the leftovers are gone
            assert_eq!(leftovers(), Vec::<String>::new());
            // (b) the binary and other files stay
            assert_eq!(binary_identity(&app_managed_binary(&dir)), old);
            assert!(dir.join("known_hosts").is_file());
            rig.daemon.stop();
        },
    );
}

/// AC-M5-06-8 (b): a preview (dry run) counts as running from its version check to its end.
#[test]
fn ac_m5_06_8_install_waits_for_running_preview() {
    run_isolated_test(
        "ac_m5_06_8_install_waits_for_running_preview",
        "KESTIC_M5_06_8P_INNER",
        |root| {
            let rig = quiet_rig(root, "S06B8P");
            let client = &rig.daemon.client;
            let fx = rig.add_backup("Alpha", |_| {});
            let old = app_managed_stand_in("0.18.0");
            let key = TestSigningKey::generate("kestic-test-release");
            let _mock = serve(&ReleaseFiles::signed(RELEASE, &key), &key, Vec::new());

            rig.wrapper.hold("version");
            rig.wrapper.hold("backup");
            // The start waits for the preview's version check, so it runs on its own connection.
            let bus = latest_variant().bus_name("S06B8P");
            let backup_id = fx.backup_id_str.clone();
            let starter = std::thread::spawn(move || {
                KesticClient::connect(&bus, KESTIC_OBJECT_PATH)
                    .expect("connect")
                    .start_backup_dry_run(&backup_id, None)
                    .expect("StartBackupDryRun")
            });
            rig.wrapper
                .wait_for('S', "version", 1, Duration::from_secs(20));
            let id = client.start_restic_install().expect("StartResticInstall");
            wait_install(client, &id, INSTALL_TIMEOUT, "waiting_for_runs", |st| {
                st.phase == ResticInstallPhase::WaitingForRuns
            });

            // between the preview's version check and its dry run the binary stays
            rig.wrapper.release("version");
            let preview = starter.join().expect("dry run started");
            rig.wrapper
                .wait_for('S', "backup", 1, Duration::from_secs(20));
            // quiet window: the install keeps waiting for the dry run
            std::thread::sleep(Duration::from_millis(500));
            assert_eq!(
                status(client, &id).phase,
                ResticInstallPhase::WaitingForRuns
            );
            let binary = app_managed_binary(&data_dir());
            assert_eq!(binary_identity(&binary), old);

            rig.wrapper.release("backup");
            let end = wait_finished(client, &id);
            assert_eq!(end.state, "done", "{end:?}");
            let preview_end = client
                .get_backup_dry_run_status(&preview)
                .expect("dry run status");
            assert_ne!(preview_end.state, "running");
            assert_eq!(reported_version(&binary), RELEASE);
            rig.daemon.stop();
        },
    );
}

/// M5-S06: after an installation the daemon checks restic again: with the app-managed restic as the
/// source and its binary missing, (a) the state first reports restic unusable and (b) after the
/// installation reports the installed version, without a run or another call in between.
#[test]
fn restic_install_refreshes_the_restic_status() {
    run_isolated_test(
        "restic_install_refreshes_the_restic_status",
        "KESTIC_M5_06C_REPROBE_INNER",
        |root| {
            let key = TestSigningKey::generate("kestic-test-release");
            let _mock = serve(&ReleaseFiles::signed(RELEASE, &key), &key, Vec::new());
            let variant = latest_variant();
            let sub = variant.sub("S06CR");
            let (config_path, _) = common::m3::daemon_paths(root, &sub);
            let config = kestic_core::config::AppConfig {
                restic_provider: ResticProvider::AppManaged,
                ..Default::default()
            };
            fs::create_dir_all(config_path.parent().expect("cfg dir")).expect("cfg dir");
            kestic_core::config::save_config(&config_path, &config).expect("save config");
            let rig = M3Rig::start_on(
                root,
                &variant,
                &sub,
                &variant.bus_name("S06CR"),
                START,
                |o| {
                    o.enable_monitors = false;
                    o.enable_scheduler = false;
                    o.set_restic_provider = false;
                },
            );
            let client = &rig.daemon.client;
            // (a)
            assert!(!client.get_state().expect("state").restic_status.valid);

            let id = client.start_restic_install().expect("StartResticInstall");
            let end = wait_finished(client, &id);
            assert_eq!(
                end.outcome,
                Some(ResticInstallOutcome::Installed),
                "{end:?}"
            );
            // (b)
            let status = wait_for_state(client, Duration::from_secs(10), "restic valid", |state| {
                state.restic_status.valid
            })
            .restic_status;
            assert_eq!(status.version.as_deref(), Some(RELEASE));
            rig.daemon.stop();
        },
    );
}
