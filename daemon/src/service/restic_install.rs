// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Installation of the restic managed by Kestic (`StartResticInstall`,
//! `GetResticInstallStatus`, `CancelResticInstall`): one at a time, on a worker thread. Download
//! and checks run at once; the binary is replaced only when no restic runs
//! ([`ResticBinaryGate`]).

use super::error::KesticServiceErr;
use super::state::{DaemonCore, STOPPING, refresh_probe_unlocked};
use crate::dbus_api::ResticInstallStatusDto;
use kestic_core::clock::Clock;
use kestic_core::config::default_data_dir;
use kestic_core::model::ResticProvider;
use kestic_core::model::RunId;
use kestic_core::provider::{ResticLocations, resolve_with, same_binary};
// An installation has the states of a list query.
use kestic_core::query_list::QueryState as InstallState;
use kestic_core::restic_gate::ResticBinaryGate;
use kestic_core::restic_install::{
    InstallObserver, InstallOutcome, InstallRequest, RemoveError, app_managed_binary,
    install_latest_observed, remove_app_managed, remove_leftovers,
};
use kestic_core::restic_release::{Arch, InstallError};
use kestic_core::sync::LockUnpoisoned;
use kestic_i18n::codes::{ResticInstallCode, ResticInstallOutcome, ResticInstallPhase};
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use time::OffsetDateTime;

/// How long a finished installation stays readable.
const KEEP_FINISHED: time::Duration = time::Duration::minutes(10);

/// What an installation reported so far. While it installs, the worker locks only this, never
/// `DaemonCore`, so it can report while it holds the replace lease and restic processes wait
/// for it.
#[derive(Debug)]
pub(crate) struct InstallProgress {
    state: InstallState,
    phase: ResticInstallPhase,
    version: Option<String>,
    bytes_done: u64,
    total_bytes: Option<u64>,
    outcome: Option<ResticInstallOutcome>,
    error: Option<(ResticInstallCode, String)>,
    finished_at: Option<OffsetDateTime>,
}

impl InstallProgress {
    fn new() -> Self {
        Self {
            state: InstallState::Running,
            phase: ResticInstallPhase::FetchingRelease,
            version: None,
            bytes_done: 0,
            total_bytes: None,
            outcome: None,
            error: None,
            finished_at: None,
        }
    }

    fn dto(&self) -> ResticInstallStatusDto {
        ResticInstallStatusDto {
            state: self.state.wire_name().to_string(),
            phase: self.phase,
            version: self.version.clone(),
            bytes_done: self.bytes_done,
            total_bytes: self.total_bytes,
            outcome: self.outcome,
            error_kind: self.error.as_ref().map(|(code, _)| *code),
            error_detail: self.error.as_ref().map(|(_, detail)| detail.clone()),
        }
    }

    fn expired(&self, now: OffsetDateTime) -> bool {
        self.finished_at
            .is_some_and(|finished| now >= finished + KEEP_FINISHED)
    }

    /// Stops a running installation unless it already replaces the binary; true if it stopped.
    fn cancel(&mut self, now: OffsetDateTime) -> bool {
        if self.state != InstallState::Running || self.phase == ResticInstallPhase::Replacing {
            return false;
        }
        self.state = InstallState::Cancelled;
        self.finished_at = Some(now);
        true
    }

    /// Records the worker's result, unless the installation was cancelled meanwhile.
    fn finish(&mut self, result: Result<InstallOutcome, InstallError>, now: OffsetDateTime) {
        if self.state != InstallState::Running {
            return;
        }
        match result {
            Ok(InstallOutcome::Installed(version)) => {
                self.state = InstallState::Done;
                self.outcome = Some(ResticInstallOutcome::Installed);
                self.version = Some(version.to_string());
            }
            Ok(InstallOutcome::AlreadyCurrent(version)) => {
                self.state = InstallState::Done;
                self.outcome = Some(ResticInstallOutcome::AlreadyCurrent);
                self.version = Some(version.to_string());
            }
            Err(err) => match err.code() {
                Some(code) => {
                    self.state = InstallState::Error;
                    self.error = Some((code, err.to_string()));
                }
                None => self.state = InstallState::Cancelled,
            },
        }
        self.finished_at = Some(now);
    }
}

pub(crate) type InstallMap = HashMap<String, Arc<Mutex<InstallProgress>>>;

/// Reports the worker's progress into its [`InstallProgress`].
struct Observer {
    progress: Arc<Mutex<InstallProgress>>,
}

impl InstallObserver for Observer {
    fn phase(&self, phase: ResticInstallPhase) {
        let mut progress = self.progress.lock_unpoisoned();
        if progress.state == InstallState::Running {
            progress.phase = phase;
            if phase == ResticInstallPhase::Downloading {
                progress.bytes_done = 0;
                progress.total_bytes = None;
            }
        }
    }

    fn version(&self, version: &str) {
        self.progress.lock_unpoisoned().version = Some(version.to_string());
    }

    fn bytes(&self, done: u64, total: Option<u64>) {
        let mut progress = self.progress.lock_unpoisoned();
        progress.bytes_done = done;
        progress.total_bytes = total;
    }

    fn cancelled(&self) -> bool {
        self.progress.lock_unpoisoned().state != InstallState::Running
    }

    fn begin_replace(&self) -> bool {
        let mut progress = self.progress.lock_unpoisoned();
        if progress.state != InstallState::Running {
            return false;
        }
        progress.phase = ResticInstallPhase::Replacing;
        true
    }
}

/// Marks the installation as failed if its worker ends without a result (a panic).
struct FinishGuard {
    progress: Arc<Mutex<InstallProgress>>,
    clock: Arc<dyn Clock>,
}

impl Drop for FinishGuard {
    fn drop(&mut self) {
        let mut progress = self.progress.lock_unpoisoned();
        if progress.state == InstallState::Running {
            progress.finish(
                Err(InstallError::WriteFailed(
                    "the installation ended unexpectedly".to_string(),
                )),
                self.clock.now_utc(),
            );
        }
    }
}

/// Removes the leftovers of an installation that was interrupted when an earlier daemon ended,
/// once: at the start or before this daemon's first installation, whichever comes first. Both
/// run under the `DaemonCore` lock, so the sweep never meets the temp file of a running
/// installation.
#[derive(Debug, Default)]
pub(crate) struct LeftoverSweep {
    done: bool,
}

/// Removes the leftovers of an earlier daemon's interrupted installation unless this daemon did
/// already.
pub(crate) fn remove_install_leftovers(shared: &Arc<Mutex<DaemonCore>>) {
    shared
        .lock_unpoisoned()
        .install_leftovers
        .run_once(default_data_dir().as_deref());
}

impl LeftoverSweep {
    /// Removes the leftovers in `dir` unless that happened already.
    pub(crate) fn run_once(&mut self, dir: Option<&std::path::Path>) {
        if self.done {
            return;
        }
        self.done = true;
        let Some(dir) = dir else {
            return;
        };
        let cleanup = remove_leftovers(dir);
        for (path, err) in cleanup.failed {
            eprintln!(
                "kestic: could not remove {} from an interrupted restic installation: {err}",
                path.display()
            );
        }
    }
}

impl DaemonCore {
    fn forget_expired_installs(&mut self) {
        let now = self.clock.now_utc();
        self.restic_installs
            .retain(|_, progress| !progress.lock_unpoisoned().expired(now));
    }

    fn install(
        &mut self,
        install_id: &str,
    ) -> Result<Arc<Mutex<InstallProgress>>, KesticServiceErr> {
        self.forget_expired_installs();
        self.restic_installs
            .get(install_id)
            .cloned()
            .ok_or_else(|| KesticServiceErr::NotFound(install_id.to_string()))
    }

    pub fn get_restic_install_status(
        &mut self,
        install_id: &str,
    ) -> Result<ResticInstallStatusDto, KesticServiceErr> {
        Ok(self.install(install_id)?.lock_unpoisoned().dto())
    }

    /// The `install_id` of the installation that is running.
    pub(super) fn running_restic_install(&self) -> Option<String> {
        self.restic_installs
            .iter()
            .find(|(_, progress)| progress.lock_unpoisoned().state == InstallState::Running)
            .map(|(id, _)| id.clone())
    }

    /// Cancels every running installation, as `CancelResticInstall` would (the daemon stops).
    pub(super) fn cancel_running_restic_installs(&mut self) {
        let now = self.clock.now_utc();
        for progress in self.restic_installs.values() {
            let mut progress = progress.lock_unpoisoned();
            if progress.state == InstallState::Running {
                progress.cancel(now);
            }
        }
    }

    pub fn cancel_restic_install(&mut self, install_id: &str) -> Result<(), KesticServiceErr> {
        let progress = self.install(install_id)?;
        let now = self.clock.now_utc();
        progress.lock_unpoisoned().cancel(now);
        Ok(())
    }
}

/// What the worker needs.
struct InstallWorker {
    shared: Arc<Mutex<DaemonCore>>,
    progress: Arc<Mutex<InstallProgress>>,
    clock: Arc<dyn Clock>,
    search_path: Option<OsString>,
}

/// `StartResticInstall`: registers the installation and runs it on a worker thread; refused
/// while another one runs.
pub fn start_restic_install_async(
    shared: &Arc<Mutex<DaemonCore>>,
) -> Result<String, KesticServiceErr> {
    let (install_id, worker) = {
        let mut guard = shared.lock_unpoisoned();
        guard.forget_expired_installs();
        // A call that arrives right after the bus name was taken may come before the sweep at
        // the start; then it sweeps here, before its worker writes a temp file.
        guard
            .install_leftovers
            .run_once(default_data_dir().as_deref());
        if guard.stopping() {
            return Err(KesticServiceErr::Internal(STOPPING.to_string()));
        }
        let running = guard
            .restic_installs
            .values()
            .any(|progress| progress.lock_unpoisoned().state == InstallState::Running);
        if running {
            return Err(KesticServiceErr::AlreadyRunning(
                "a restic installation is already running".to_string(),
            ));
        }
        let install_id = RunId::generate()
            .map(|id| id.as_str().to_string())
            .map_err(|err| KesticServiceErr::Internal(err.to_string()))?;
        let progress = Arc::new(Mutex::new(InstallProgress::new()));
        guard
            .restic_installs
            .insert(install_id.clone(), Arc::clone(&progress));
        let worker = InstallWorker {
            shared: Arc::clone(shared),
            progress,
            clock: Arc::clone(&guard.clock),
            search_path: guard.program_search_path(),
        };
        (install_id, worker)
    };
    thread::spawn(move || run_install(&worker));
    Ok(install_id)
}

fn run_install(worker: &InstallWorker) {
    let _guard = FinishGuard {
        progress: Arc::clone(&worker.progress),
        clock: Arc::clone(&worker.clock),
    };
    let result = Arch::host().and_then(|arch| {
        let dir = default_data_dir().ok_or_else(|| {
            InstallError::WriteFailed("neither XDG_DATA_HOME nor HOME is set".to_string())
        })?;
        let request = InstallRequest {
            data_dir: &dir,
            arch,
            search_path: worker.search_path.as_deref(),
        };
        let observer = Observer {
            progress: Arc::clone(&worker.progress),
        };
        install_latest_observed(&request, ResticBinaryGate::global(), &observer)
    });
    if let Err(err) = &result {
        eprintln!("kestic: restic installation: {err}");
    }
    let installed = matches!(result, Ok(InstallOutcome::Installed(_)));
    if installed {
        // The restic in use may be the one just installed: check it again before the
        // installation reports its end, so a GUI that reads the state then sees the new one.
        refresh_probe_unlocked(&worker.shared);
    }
    worker
        .progress
        .lock_unpoisoned()
        .finish(result, worker.clock.now_utc());
    if installed {
        publish_state(&worker.shared);
    }
}

/// `RemoveAppManagedRestic`: deletes the app-managed restic at once. Refused with `InUse` while
/// it is the restic in use (its source, «Automatic» without a usable system restic, a system
/// restic that links to it, or a custom path to it) or a restic process may run it; removing a
/// missing binary changes nothing.
pub fn remove_app_managed_restic(shared: &Arc<Mutex<DaemonCore>>) -> Result<(), KesticServiceErr> {
    let Some(dir) = default_data_dir() else {
        return Ok(());
    };
    let binary = app_managed_binary(&dir);
    if binary.symlink_metadata().is_err() {
        return Ok(());
    }
    let mut provider = shared.lock_unpoisoned().config.restic_provider.clone();
    let removed = loop {
        // The check runs restic, so it runs without the lock; the removal then holds the lock,
        // and a source chosen meanwhile is checked again. `SetResticProvider` checks under the
        // same lock that the binary it chose still exists.
        if is_active_source(&provider, &binary, &ResticLocations::from_env()) {
            return Err(KesticServiceErr::InUse(
                "the restic managed by Kestic is the restic in use".to_string(),
            ));
        }
        let guard = shared.lock_unpoisoned();
        if guard.config.restic_provider == provider {
            break remove_app_managed(&dir, ResticBinaryGate::global());
        }
        provider = guard.config.restic_provider.clone();
    };
    match removed {
        Ok(_) => {}
        Err(RemoveError::InUse) => {
            return Err(KesticServiceErr::InUse(
                "a restic process uses the restic managed by Kestic".to_string(),
            ));
        }
        Err(RemoveError::Io(err)) => return Err(KesticServiceErr::Internal(err.to_string())),
    }
    refresh_probe_unlocked(shared);
    publish_state(shared);
    Ok(())
}

/// Whether `provider` runs the app-managed restic at `binary`, with the system restic at
/// `locations`.
fn is_active_source(provider: &ResticProvider, binary: &Path, locations: &ResticLocations) -> bool {
    // A system restic that is a link to the app-managed one is the app-managed one.
    let system_is_it = || {
        locations
            .system
            .as_deref()
            .is_some_and(|system| same_binary(system, binary))
    };
    match provider {
        ResticProvider::AppManaged => true,
        // Without a usable system restic «Automatic» falls back to it.
        ResticProvider::Automatic => {
            resolve_with(&ResticProvider::System, locations).is_err() || system_is_it()
        }
        ResticProvider::Custom { path } => same_binary(path, binary),
        ResticProvider::System => system_is_it(),
    }
}

/// Sends the current state to the tray and as `StatusChanged`.
fn publish_state(shared: &Arc<Mutex<DaemonCore>>) {
    let publication = shared.lock_unpoisoned().state_publication();
    publication.publish();
}

#[cfg(test)]
mod tests;
