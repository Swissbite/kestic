// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
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
