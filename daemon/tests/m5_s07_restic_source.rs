// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Integration tests for M5-S07a: the restic source in the daemon. The system restic is a
//! stand-in first in the daemon's `PATH`; the app-managed restic lies at its fixed place below
//! the isolated `XDG_DATA_HOME`.

mod common;

use common::m3::{M3Rig, assert_dbus_error_name, latest_variant, wait_until, wait_until_within};
use common::{SYSTEM_PROGRAMS_DIR, run_isolated_test_with_system_programs};
use kestic_core::model::{ResticProvider, RunStatus};
use kestic_core::restic_install::{app_managed_binary, kestic_data_dir};
use kestic_core::restic_release::{TrustAnchor, set_test_anchor};
use kestic_core::secrets::{SecretError, SecretStore, SecretValue};
use kestic_daemon::dbus_api::{KesticClient, ResticBinaryDto, ResticStatusDto};
use kestic_i18n::codes::{ResticSourceCode, ServiceErrorCode};
use kestic_test_support::release_mock::{
    ReleaseFiles, ReleaseMock, Route, TestSigningKey, asset_path,
};
use kestic_test_support::tools::write_executable;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use time::OffsetDateTime;
use time::macros::datetime;

const START: OffsetDateTime = datetime!(2026-12-01 10:00:00 UTC);
const IN_USE: &str = "net.daester.Kestic.Error.InUse";

fn app_binary() -> PathBuf {
    let dir = kestic_data_dir(
        std::env::var_os("HOME").as_deref(),
        std::env::var_os("XDG_DATA_HOME").as_deref(),
    )
    .expect("data dir");
    fs::create_dir_all(&dir).expect("data dir");
    app_managed_binary(&dir)
}

/// Writes `script` to `path` through a new file, so a process still running the old one is not
/// disturbed.
fn replace_executable(path: &Path, script: &str) {
    let temp = path.with_extension("new");
    write_executable(&temp, script);
    fs::rename(&temp, path).expect("rename");
}

fn json_script(version: &str) -> String {
    format!("#!/bin/sh\necho '{{\"message_type\":\"version\",\"version\":\"{version}\"}}'\n")
}

/// Like the real restic 0.16.4: no `version --json`, a text line instead.
const RESTIC_0_16_4: &str = "#!/bin/sh\n\
    if [ \"${2:-}\" = --json ]; then echo 'unknown flag: --json' >&2; exit 1; fi\n\
    echo 'restic 0.16.4 compiled with go1.22.2 on linux/amd64'\n";

fn system_restic(root: &Path) -> PathBuf {
    root.join(SYSTEM_PROGRAMS_DIR).join("restic")
}

/// A daemon without scheduler that keeps the restic source of its `config.toml` (none: the
/// default «Automatic»).
fn rig(root: &Path, base: &str) -> M3Rig {
    M3Rig::start(root, &latest_variant(), base, START, |o| {
        o.enable_monitors = false;
        o.enable_scheduler = false;
        o.set_restic_provider = false;
    })
}

fn restic_status(client: &KesticClient) -> ResticStatusDto {
    client.get_state().expect("GetState").restic_status
}

fn path_str(path: &Path) -> Option<String> {
    Some(path.to_string_lossy().into_owned())
}

/// AC-M5-07-5 (a)–(d), daemon side: the offer to switch once the system restic is recent
/// enough, the switch to «Automatic», and removing the app-managed restic.
#[test]
fn ac_m5_07_5_switch_to_automatic_and_remove_app_managed() {
    run_isolated_test_with_system_programs(
        "ac_m5_07_5_switch_to_automatic_and_remove_app_managed",
        "KESTIC_M5_07_5A_INNER",
        |root| {
            let system = system_restic(root);
            replace_executable(&system, RESTIC_0_16_4);
            let app = app_binary();
            write_executable(&app, &json_script("0.19.1"));
            let rig = rig(root, "S07A5");
            let client = &rig.daemon.client;

            // «Automatic» falls back to the app-managed restic: it may not be removed.
            let st = restic_status(client);
            assert_eq!(st.source, ResticSourceCode::Automatic);
            assert_eq!(st.binary_path, path_str(&app));
            let err = client.remove_app_managed_restic().expect_err("in use");
            assert_dbus_error_name(&err, IN_USE);
            // Neither while it is the chosen source.
            let st = client
                .set_restic_provider(&ResticProvider::AppManaged)
                .expect("app-managed");
            assert!(!st.offer_switch_to_system, "0.16.4 is not enough");
            let err = client.remove_app_managed_restic().expect_err("in use");
            assert_dbus_error_name(&err, IN_USE);
            assert!(app.exists());

            // (a) the system restic is updated; the next check within 60 s does not see it yet,
            // the one after 60 s offers the switch
            client.check_availability(true).expect("CheckAvailability");
            replace_executable(&system, &json_script("0.18.1"));
            client.check_availability(false).expect("CheckAvailability");
            assert!(!restic_status(client).offer_switch_to_system);
            rig.clock.advance(Duration::from_secs(61));
            client.check_availability(false).expect("CheckAvailability");
            let st = restic_status(client);
            assert!(st.offer_switch_to_system, "{st:?}");
            assert_eq!(st.binary_path, path_str(&app), "still the chosen one");

            // (b) switching to «Automatic» uses the system restic
            let st = client
                .set_restic_provider(&ResticProvider::Automatic)
                .expect("automatic");
            assert_eq!(st.binary_path, path_str(&system));
            assert!(!st.offer_switch_to_system);

            // (d) «Automatic» on the system restic: no question, the app-managed one is still
            // reported and may be removed
            let st = restic_status(client);
            assert_eq!(st.source, ResticSourceCode::Automatic);
            assert!(st.valid);
            assert!(!st.offer_switch_to_system);
            assert_eq!(
                st.system,
                Some(ResticBinaryDto {
                    path: system.to_string_lossy().into_owned(),
                    version: Some("0.18.1".to_string()),
                    too_old: false,
                })
            );
            assert_eq!(
                st.app_managed.as_ref().map(|a| a.version.clone()),
                Some(Some("0.19.1".to_string()))
            );
            assert_eq!(st.install_id, None, "no installation runs");

            // (c) removing deletes the binary; the status no longer reports it
            client.remove_app_managed_restic().expect("removed");
            assert!(!app.exists());
            let st = restic_status(client);
            assert_eq!(st.app_managed, None);
            assert_eq!(st.binary_path, path_str(&system));
            // removing it again changes nothing
            client
                .remove_app_managed_restic()
                .expect("nothing to remove");
            // without the binary it cannot be chosen
            let err = client
                .set_restic_provider(&ResticProvider::AppManaged)
                .expect_err("not installed");
            // Only the code is compared and printed: an error may carry secret details.
            assert_eq!(err.code(), ServiceErrorCode::InvalidInput);
            rig.daemon.stop();
        },
    );
}

/// AC-M5-07-5 (c): a backup that runs on the app-managed restic keeps it from being removed
/// after the switch to «Automatic»; once the run has ended, it can be removed.
#[test]
fn ac_m5_07_5_remove_refused_while_a_run_uses_app_managed() {
    run_isolated_test_with_system_programs(
        "ac_m5_07_5_remove_refused_while_a_run_uses_app_managed",
        "KESTIC_M5_07_5B_INNER",
        |root| {
            replace_executable(&system_restic(root), &json_script("0.18.1"));
            let rig = rig(root, "S07A5R");
            let client = &rig.daemon.client;
            // The app-managed restic is the real one behind the logging wrapper of the rig.
            let app = app_binary();
            write_executable(
                &app,
                &format!("#!/bin/sh\nexec '{}' \"$@\"\n", rig.wrapper.path.display()),
            );
            let st = client
                .set_restic_provider(&ResticProvider::AppManaged)
                .expect("app-managed");
            assert_eq!(st.binary_path, path_str(&app));
            let fx = rig.add_backup("app", |_| {});

            rig.wrapper.hold("backup");
            let run_id = client.start_backup(&fx.backup_id_str).expect("start");
            wait_until("restic backup started", || {
                rig.wrapper
                    .entries()
                    .iter()
                    .any(|e| e.kind == 'S' && e.sub == "backup")
            });
            client
                .set_restic_provider(&ResticProvider::Automatic)
                .expect("automatic");
            let err = client
                .remove_app_managed_restic()
                .expect_err("the run uses it");
            assert_dbus_error_name(&err, IN_USE);
            assert!(app.exists());

            rig.wrapper.release("backup");
            wait_until_within(Duration::from_secs(60), "the run ended", || {
                let run = client.get_run_detail(&run_id).expect("GetRunDetail").run;
                !matches!(run.status, RunStatus::Pending | RunStatus::Running)
            });
            // The run releases the binary when its worker ends, right after the result.
            wait_until("removed after the run", || {
                client.remove_app_managed_restic().is_ok()
            });
            assert!(!app.exists());
            rig.daemon.stop();
        },
    );
}

/// M5-S07: `ResticStatusDto.install_id` names the installation of the app-managed restic while it runs.
#[test]
fn restic_status_names_the_running_installation() {
    run_isolated_test_with_system_programs(
        "restic_status_names_the_running_installation",
        "KESTIC_M5_07_INSTALL_INNER",
        |root| {
            replace_executable(&system_restic(root), &json_script("0.18.1"));
            let rig = rig(root, "S07AI");
            let client = &rig.daemon.client;
            let key = TestSigningKey::generate("kestic-test-release");
            let files = ReleaseFiles::signed("0.99.0", &key);
            let archive = asset_path("0.99.0", &files.archive_name("amd64"));
            let mock =
                ReleaseMock::start_with(&files, vec![(archive.clone(), Route::StallAfterHeaders)]);
            let key_path = key.public_key_file().to_string_lossy().into_owned();
            let fpr = key.fingerprint().to_string();
            let base_url = mock.base_url();
            set_test_anchor(Some(
                TrustAnchor::from_lookup(|name| match name {
                    "KESTIC_TEST_RELEASE_URL" => Some(base_url.clone()),
                    "KESTIC_TEST_SIGNING_KEY" => Some(key_path.clone()),
                    "KESTIC_TEST_SIGNING_FPR" => Some(fpr.clone()),
                    _ => None,
                })
                .expect("test anchor"),
            ));

            assert_eq!(restic_status(client).install_id, None);
            let id = client.start_restic_install().expect("StartResticInstall");
            wait_until("archive requested", || mock.requests().contains(&archive));
            assert_eq!(restic_status(client).install_id, Some(id.clone()));
            client.cancel_restic_install(&id).expect("cancel");
            wait_until("installation ended", || {
                client.get_restic_install_status(&id).expect("status").state != "running"
            });
            assert_eq!(restic_status(client).install_id, None);
            rig.daemon.stop();
        },
    );
}

/// A password store whose availability check removes `binary` once, after `armed` was set: it
/// runs after `SetResticProvider` checked the restic and before it takes the daemon lock, where
/// a concurrent `RemoveAppManagedRestic` would fall.
struct RemovingStore {
    binary: PathBuf,
    armed: Arc<AtomicBool>,
}

impl SecretStore for RemovingStore {
    fn set_secret(&self, _: &str, _: &str, _: &SecretValue) -> Result<(), SecretError> {
        Ok(())
    }

    fn get_secret(&self, _: &str, _: &str) -> Result<Option<SecretValue>, SecretError> {
        Ok(None)
    }

    fn delete_secret(&self, _: &str, _: &str) -> Result<(), SecretError> {
        Ok(())
    }

    fn check_availability(&self) -> Result<(), SecretError> {
        if self.armed.swap(false, Ordering::SeqCst) {
            fs::remove_file(&self.binary).map_err(|err| SecretError::Io(err.to_string()))?;
        }
        Ok(())
    }
}

/// AC-M5-07-5 (c): the app-managed restic removed after `SetResticProvider` checked the chosen
/// path never becomes the source, whether the path is the binary or a link to it; the source
/// stays as it was.
#[test]
fn ac_m5_07_5_source_never_points_at_a_removed_binary() {
    run_isolated_test_with_system_programs(
        "ac_m5_07_5_source_never_points_at_a_removed_binary",
        "KESTIC_M5_07_5R_INNER",
        |root| {
            for through_link in [false, true] {
                let app = app_binary();
                write_executable(&app, &json_script("0.19.1"));
                let chosen = if through_link {
                    let link = root.join("custom-link/restic");
                    fs::create_dir_all(link.parent().expect("parent")).expect("mkdir");
                    std::os::unix::fs::symlink(&app, &link).expect("symlink");
                    link
                } else {
                    app.clone()
                };
                let armed = Arc::new(AtomicBool::new(false));
                let store = RemovingStore {
                    binary: app.clone(),
                    armed: Arc::clone(&armed),
                };
                let base = if through_link { "S07R5L" } else { "S07R5D" };
                let rig = M3Rig::start(root, &latest_variant(), base, START, |o| {
                    o.enable_monitors = false;
                    o.enable_scheduler = false;
                    o.set_restic_provider = false;
                    o.secret_store = Some(Arc::new(store));
                });
                let client = &rig.daemon.client;
                let config_before = fs::read(&rig.daemon.config_path).ok();

                armed.store(true, Ordering::SeqCst);
                let err = client
                    .set_restic_provider(&ResticProvider::Custom { path: chosen })
                    .expect_err("SetResticProvider on a removed restic");

                // (c) the binary is gone
                assert!(!app.exists(), "link: {through_link}");
                // (c) the change of source is refused
                assert_eq!(
                    err.code(),
                    ServiceErrorCode::ResticUnavailable,
                    "link: {through_link}"
                );
                // (c) the source is unchanged, in the daemon and on disk
                assert_eq!(
                    restic_status(client).source,
                    ResticSourceCode::Automatic,
                    "link: {through_link}"
                );
                assert_eq!(
                    fs::read(&rig.daemon.config_path).ok(),
                    config_before,
                    "link: {through_link}"
                );
                rig.daemon.stop();
            }
        },
    );
}
