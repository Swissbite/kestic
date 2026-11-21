// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! What keeps a running backup or maintenance visible and alive: the logind sleep inhibitor, the
//! portal inhibit, the Plasma job view and its cancel watcher.

use crate::inhibit::PortalInhibitGuard;
use crate::job_view::{CAPABILITY_KILLABLE, JobViewCancelGuard, JobViewHandle};
use crate::monitors::{LogindSleepInhibitor, MonitorBusConfig};
use crate::notifications::KESTIC_DESKTOP_ENTRY;
use kestic_core::restic::CancelHandle;

/// The daemon switches a run's supervision depends on, cloned out of the `DaemonCore`.
#[derive(Clone)]
pub(super) struct SupervisionConfig {
    pub(super) enable_job_view: bool,
    pub(super) job_view_dest: String,
    pub(super) enable_portal_inhibit: bool,
    pub(super) monitor_bus: MonitorBusConfig,
    pub(super) enable_system_monitors: bool,
}

/// The inhibitors and job view of one run.
///
/// Fields drop in declaration order: cancel watcher, job view, portal inhibit, logind inhibitor.
pub(super) struct RunSupervision {
    cancel_guard: Option<JobViewCancelGuard>,
    pub(super) job_view: Option<JobViewHandle>,
    _portal_inhibit: Option<PortalInhibitGuard>,
    pub(super) logind: Option<LogindSleepInhibitor>,
}

impl RunSupervision {
    /// Takes the inhibitors, opens the job view titled `title` and lets its cancel button
    /// cancel `cancel`.
    pub(super) fn start(config: &SupervisionConfig, title: &str, cancel: &CancelHandle) -> Self {
        let logind = config
            .enable_system_monitors
            .then(|| LogindSleepInhibitor::acquire(&config.monitor_bus))
            .flatten();
        let portal_inhibit = config
            .enable_portal_inhibit
            .then(|| PortalInhibitGuard::acquire_default(title).ok())
            .flatten();
        let job_view = config
            .enable_job_view
            .then(|| {
                JobViewHandle::request(
                    &config.job_view_dest,
                    KESTIC_DESKTOP_ENTRY,
                    title,
                    CAPABILITY_KILLABLE,
                )
                .ok()
            })
            .flatten();
        let cancel_guard = job_view.as_ref().map(|jv| {
            let cancel_for_jv = cancel.clone();
            jv.start_cancel_watcher(move || cancel_for_jv.cancel())
        });
        Self {
            cancel_guard,
            job_view,
            _portal_inhibit: portal_inhibit,
            logind,
        }
    }

    /// Stops listening for the job view's cancel button.
    pub(super) fn stop_cancel_watcher(&mut self) {
        drop(self.cancel_guard.take());
    }
}
