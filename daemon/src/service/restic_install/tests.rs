// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use kestic_core::secrets::{SecretError, SecretStore, SecretValue};
use std::sync::atomic::{AtomicBool, Ordering};
use time::macros::datetime;

const START: OffsetDateTime = datetime!(2026-12-01 12:00:00 UTC);

fn observer() -> Observer {
    Observer {
        progress: Arc::new(Mutex::new(InstallProgress::new())),
    }
}

/// A cancel before the `rename` wins: the installation must not replace the binary.
#[test]
fn cancel_before_replace_stops_the_replacement() {
    let observer = observer();
    observer.phase(ResticInstallPhase::WaitingForRuns);
    // (a) the cancel takes effect
    assert!(observer.progress.lock_unpoisoned().cancel(START));
    // (b) the worker may no longer replace
    assert!(observer.cancelled());
    assert!(!observer.begin_replace());
    // (c) its late result keeps the state cancelled
    observer.progress.lock_unpoisoned().finish(
        Ok(InstallOutcome::Installed(
            "0.99.0".parse().expect("version"),
        )),
        START,
    );
    assert_eq!(observer.progress.lock_unpoisoned().dto().state, "cancelled");
}

/// A cancel that arrives once the binary is being replaced comes too late; the installation
/// ends as done.
#[test]
fn cancel_during_replace_comes_too_late() {
    let observer = observer();
    assert!(observer.begin_replace());
    // (a) refused
    assert!(!observer.progress.lock_unpoisoned().cancel(START));
    // (b) the result counts
    observer.progress.lock_unpoisoned().finish(
        Ok(InstallOutcome::Installed(
            "0.99.0".parse().expect("version"),
        )),
        START,
    );
    let dto = observer.progress.lock_unpoisoned().dto();
    assert_eq!(dto.state, "done");
    assert_eq!(dto.outcome, Some(ResticInstallOutcome::Installed));
    assert_eq!(dto.phase, ResticInstallPhase::Replacing);
}

/// A finished installation stays readable for ten minutes after its end, however long it ran;
/// a running one never expires.
#[test]
fn finished_installs_expire_ten_minutes_after_their_end() {
    let mut progress = InstallProgress::new();
    // (a) running: no expiry
    assert!(!progress.expired(START + time::Duration::hours(5)));
    let end = START + time::Duration::hours(5);
    progress.finish(Err(InstallError::Network("offline".to_string())), end);
    // (b) readable until just before ten minutes after the end
    assert!(!progress.expired(end + time::Duration::seconds(599)));
    // (c) gone after
    assert!(progress.expired(end + KEEP_FINISHED));
    let dto = progress.dto();
    assert_eq!(dto.state, "error");
    assert_eq!(dto.error_kind, Some(ResticInstallCode::Network));
    assert_eq!(
        dto.error_detail.as_deref(),
        Some("download from the restic release server failed: offline")
    );
}

/// A worker that ends without a result leaves an error, so a new installation can start.
#[test]
fn worker_without_result_ends_as_error() {
    let progress = Arc::new(Mutex::new(InstallProgress::new()));
    let clock: Arc<dyn Clock> = Arc::new(kestic_core::clock::FakeClock::new(START));
    let worker = {
        let progress = Arc::clone(&progress);
        thread::spawn(move || {
            let _guard = FinishGuard { progress, clock };
            panic!("worker fails");
        })
    };
    assert!(worker.join().is_err());
    let dto = progress.lock_unpoisoned().dto();
    assert_eq!(dto.state, "error");
    assert_eq!(dto.error_kind, Some(ResticInstallCode::WriteFailed));
}

/// The sweep removes what an interrupted installation left, but only once. The start of the
/// daemon and the first `StartResticInstall` both call it under the `DaemonCore` lock, in either
/// order: the first call removes the leftover, and the temp file the installation writes after
/// its call survives the second one.
#[test]
fn leftover_sweep_runs_once() {
    let dir = tempfile::tempdir().expect("tempdir");
    let old = dir.path().join(".restic-download-0a1b");
    std::fs::write(&old, b"partial").expect("old temp file");
    let mut sweep = LeftoverSweep::default();
    sweep.run_once(Some(dir.path()));
    // (a) the leftover is gone
    assert!(!old.exists());

    let running = dir.path().join(".restic-download-2c3d");
    std::fs::write(&running, b"downloading").expect("temp file of a running installation");
    sweep.run_once(Some(dir.path()));
    // (b) the second call leaves the running installation alone
    assert!(running.exists());
}

/// AC-M5-07-5 (c): the app-managed restic is in use while it is the source, the fallback of
/// «Automatic», the system restic through a link (also with the source «System»), or a custom
/// path to it.
#[test]
fn ac_m5_07_5_app_managed_in_use_as_active_source() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = tempfile::TempDir::new().expect("tempdir");
    let app = dir.path().join("data/restic");
    std::fs::create_dir_all(app.parent().expect("parent")).expect("mkdir");
    std::fs::write(
        &app,
        "#!/bin/sh\necho '{\"message_type\":\"version\",\"version\":\"0.19.1\"}'\n",
    )
    .expect("write");
    std::fs::set_permissions(&app, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let own_system = dir.path().join("bin/restic");
    std::fs::create_dir_all(own_system.parent().expect("parent")).expect("mkdir");
    std::fs::copy(&app, &own_system).expect("copy");
    let linked_system = dir.path().join("link/restic");
    std::fs::create_dir_all(linked_system.parent().expect("parent")).expect("mkdir");
    symlink(&app, &linked_system).expect("symlink");
    let at = |system: Option<&Path>| ResticLocations {
        system: system.map(Path::to_path_buf),
        app_managed: Some(app.clone()),
    };

    assert!(is_active_source(
        &ResticProvider::AppManaged,
        &app,
        &at(Some(&own_system))
    ));
    assert!(is_active_source(
        &ResticProvider::Automatic,
        &app,
        &at(None)
    ));
    assert!(is_active_source(
        &ResticProvider::Automatic,
        &app,
        &at(Some(&linked_system))
    ));
    assert!(!is_active_source(
        &ResticProvider::Automatic,
        &app,
        &at(Some(&own_system))
    ));
    assert!(is_active_source(
        &ResticProvider::Custom {
            path: linked_system.clone()
        },
        &app,
        &at(None)
    ));
    assert!(!is_active_source(
        &ResticProvider::System,
        &app,
        &at(Some(&own_system))
    ));
    assert!(is_active_source(
        &ResticProvider::System,
        &app,
        &at(Some(&linked_system))
    ));
}

/// A password store whose availability check removes `binary` once `armed`: it runs after
/// `SetResticProvider` checked the restic and before it takes the lock, where a concurrent
/// removal would fall.
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
        if self.armed.load(Ordering::SeqCst) {
            std::fs::remove_file(&self.binary).map_err(|err| SecretError::Io(err.to_string()))?;
        }
        Ok(())
    }
}

/// AC-M5-07-5 (c): a restic removed after `SetResticProvider` checked it never becomes the
/// source, also when the chosen path is a link to it; the source stays as it was.
#[test]
fn ac_m5_07_5_source_never_points_at_a_removed_binary() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    for through_link in [false, true] {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let binary = dir.path().join("data/restic");
        std::fs::create_dir_all(binary.parent().expect("parent")).expect("mkdir");
        std::fs::write(
            &binary,
            "#!/bin/sh\necho '{\"message_type\":\"version\",\"version\":\"0.19.1\"}'\n",
        )
        .expect("write");
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        let chosen = if through_link {
            let link = dir.path().join("custom/restic");
            std::fs::create_dir_all(link.parent().expect("parent")).expect("mkdir");
            symlink(&binary, &link).expect("symlink");
            link
        } else {
            binary.clone()
        };
        let config_path = dir.path().join("config/config.toml");
        let armed = Arc::new(AtomicBool::new(false));
        let run_config = crate::service::DaemonRunConfig {
            config_path: Some(config_path.clone()),
            state_dir: Some(dir.path().join("state")),
            secret_store: Some(Arc::new(RemovingStore {
                binary: binary.clone(),
                armed: Arc::clone(&armed),
            })),
            ..Default::default()
        };
        let shared = Arc::new(Mutex::new(
            DaemonCore::new(&run_config).expect("daemon core"),
        ));
        let provider = ResticProvider::Custom { path: chosen };
        let provider_json = serde_json::to_string(&provider).expect("provider JSON");

        armed.store(true, Ordering::SeqCst);
        let result = crate::service::set_restic_provider_unlocked(&shared, &provider_json);

        // (c) the binary is gone
        assert!(!binary.exists(), "link: {through_link}");
        // (c) the change of source is refused
        assert!(
            matches!(result, Err(KesticServiceErr::ResticUnavailable(_))),
            "link: {through_link}"
        );
        // (c) the source is unchanged, in memory and on disk
        assert_eq!(
            shared.lock_unpoisoned().config.restic_provider,
            ResticProvider::default(),
            "link: {through_link}"
        );
        assert!(!config_path.exists(), "link: {through_link}");
    }
}
