// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! `StartRestore`: checks the request, the target and (at the original location or in a folder
//! with content) the preview, then runs `restic restore` as a run of the kind `Restore`.
//! Restores keep their own accounting: they run beside backups and other restores, wait for
//! maintenance of their destination, and maintenance waits for them.

use super::backup_runner::{load_dest_secrets, secret_strs};
use super::error::{KesticServiceErr, backup_run_error_kind, localized_backup_run_error};
use super::restore_preview::{
    RestoreContext, RestoreParams, check_params, fingerprint, resolve_plan,
};
use super::snapshot_queries::QueryResult;
use super::state::{DaemonCore, ResticDeps, probe_for_run};
use super::state_summary::{ActivatedRun, NewActiveRun};
use super::supervision::{RunSupervision, SupervisionConfig};
use crate::dbus_api::RestoreRequest;
use crate::dbus_server::{emit_run_progress, emit_run_updated, emit_status_changed};
use crate::i18n::{i18n, i18np, tr, tr_text};
use crate::notifications::{KesticEvent, send_event_notification};
use crate::tray::TrayHandle;
use kestic_core::backup::{BackupRunError, SnapshotListAccess};
use kestic_core::model::{
    BackupId, DestinationId, PendingReason, RestoreStats, Run, RunId, RunKind, RunStatus,
};
use kestic_core::provider::ResolvedRestic;
use kestic_core::query_list::QueryState;
use kestic_core::restic::{
    CancelHandle, ResticExecutionResult, ResticStatusMessage, redact_secrets,
};
use kestic_core::restore::{RestorePlan, RestoreRunResult, RestoreTarget, execute_restore};
use kestic_core::restore_preview::blocked_above_selection;
use kestic_core::restore_target::{PREVIEW_MAX_AGE, TargetCheck, check_target, preview_required};
use kestic_core::sync::LockUnpoisoned;
use kestic_i18n::codes::{ErrorKind, RestoreTargetCode, RunProgressPhase, ServiceErrorCode};
use kestic_i18n::messages::{interrupted_by_sleep_message, nothing_restored_message};
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use zbus::Connection;

/// A restore waiting for maintenance of its destination to end.
pub(crate) struct PendingRestore {
    run: Run,
    setup: RestoreSetup,
}

/// What an accepted restore holds until its run ends.
pub(crate) struct RestoreClaim {
    destination_id: DestinationId,
    /// The resolved target folder; a second restore into it is refused meanwhile.
    target: PathBuf,
}

/// Everything a restore run needs once it starts.
#[derive(Clone)]
struct RestoreSetup {
    ctx: RestoreContext,
    params: RestoreParams,
    target: RestoreTarget,
    check: TargetCheck,
    /// Plan, backup and kept files of the matching preview, when one was given.
    from_preview: Option<PreviewFacts>,
    /// The folders this restore created for its new folder, outermost first; they are removed
    /// again when the restore ends and they are still empty.
    created: Vec<PathBuf>,
}

#[derive(Clone)]
struct PreviewFacts {
    /// The preview ran the dry runs (it was made for a target that needed them).
    dry_run: bool,
    /// The target check of the preview; it may refuse a target the check without the snapshot
    /// accepts (a repository below it that the selection reaches).
    target_code: RestoreTargetCode,
    plan: RestorePlan,
    files_kept: u64,
    backup: Option<(BackupId, String)>,
}

fn invalid(message: impl std::fmt::Display) -> KesticServiceErr {
    KesticServiceErr::InvalidInput(message.to_string())
}

/// A target the restore cannot write to is refused before any restic process starts.
fn refuse_unusable(check: &TargetCheck) -> Result<(), KesticServiceErr> {
    // restic takes the target as text: a folder whose real path is not UTF-8 would become
    // another folder.
    if check.resolved.to_str().is_none() {
        return Err(invalid(format!(
            "{}: the real path of the target folder is not valid UTF-8",
            check.resolved.display()
        )));
    }
    if check.usable() {
        return Ok(());
    }
    Err(invalid(format!(
        "{}: {}",
        check.code.as_str(),
        check.resolved.display()
    )))
}

/// Creates the new folder of a restore (and the folder holding it); only this leaf is ever
/// created, never a missing original location.
fn create_new_folder(setup: &mut RestoreSetup) -> Result<(), KesticServiceErr> {
    if !(setup.check.missing && matches!(setup.target, RestoreTarget::NewFolder { .. })) {
        return Ok(());
    }
    // Only the folder holding the new folder (`~/Restored`) and the new folder itself are
    // created; a deeper missing path is refused.
    let dir = PathBuf::from(setup.target.dir());
    if let Some(parent) = dir.parent().filter(|p| !p.exists()) {
        std::fs::create_dir(parent)
            .map_err(|e| invalid(format!("cannot create {}: {e}", parent.display())))?;
        setup.created.push(parent.to_path_buf());
    }
    // Private to the user: restic keeps the mode of an existing target folder, and the
    // restored folder may be private (e.g. ~/.ssh).
    match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
        Ok(()) => {
            setup.created.push(dir);
            Ok(())
        }
        // Someone created it meanwhile: the check below decides like for any existing folder.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(invalid(format!("cannot create {}: {e}", dir.display()))),
    }
}

/// Removes the folders the restore created while they are empty, so a failed, cancelled or
/// empty restore leaves no empty new folder behind; a folder with anything in it stays.
fn remove_created_if_empty(setup: &RestoreSetup) {
    for dir in setup.created.iter().rev() {
        if std::fs::remove_dir(dir).is_err() {
            break;
        }
    }
}

/// Creates a missing new folder and checks it again. It runs after the restic probe, so a
/// start that fails before leaves no folder behind.
fn make_target(setup: &mut RestoreSetup) -> Result<(), KesticServiceErr> {
    if !setup.check.missing {
        return Ok(());
    }
    create_new_folder(setup)?;
    setup.check = check_target(&setup.target, setup.ctx.repositories());
    refuse_unusable(&setup.check)?;
    if setup.check.code == RestoreTargetCode::ExistsNotEmpty
        && !setup.from_preview.as_ref().is_some_and(|f| f.dry_run)
    {
        return Err(KesticServiceErr::PreviewRequired(
            "the new folder has content now".to_string(),
        ));
    }
    Ok(())
}

/// The facts of preview `preview_id` when it is done, at most ten minutes old (by the clock)
/// and made for exactly these parameters; `None` otherwise.
fn matching_preview(
    core: &DaemonCore,
    preview_id: Option<&str>,
    setup: &RestoreSetup,
) -> Option<PreviewFacts> {
    let query = core.snapshot_queries.peek(preview_id?)?;
    if query.state != QueryState::Done {
        return None;
    }
    let fresh = query
        .finished_at
        .is_some_and(|t| core.clock.now_utc() - t <= PREVIEW_MAX_AGE);
    let Some(QueryResult::RestorePreview(preview)) = &query.result else {
        return None;
    };
    let wanted = fingerprint(&setup.ctx, &setup.params, &setup.target, &setup.check);
    (fresh && preview.fingerprint == wanted).then(|| PreviewFacts {
        dry_run: preview.dry_run,
        target_code: preview.check.code,
        plan: preview.plan.clone(),
        files_kept: preview.classification.counts.kept_files,
        backup: preview.backup.clone(),
    })
}

/// Checks a `StartRestore` request and returns the setup, before anything is written or started.
fn prepare_setup(
    shared: &Arc<Mutex<DaemonCore>>,
    request: &RestoreRequest,
) -> Result<(RestoreSetup, ResticDeps, SupervisionConfig), KesticServiceErr> {
    let destination_id = DestinationId::parse(&request.destination_id).map_err(invalid)?;
    let params = check_params(
        &request.snapshot_id,
        &request.selection,
        Some(&request.target),
        request.overwrite,
    )?;
    let target = params
        .target
        .clone()
        .ok_or_else(|| invalid("restore without target"))?;
    let (ctx, deps, supervision) = {
        let guard = shared.lock_unpoisoned();
        let dest = guard
            .config
            .destination(&destination_id)
            .cloned()
            .ok_or_else(|| KesticServiceErr::NotFound(destination_id.to_string()))?;
        (
            RestoreContext::from_config(&guard.config, dest, guard.timezone.clone()),
            guard.restic_deps(),
            guard.supervision_config(),
        )
    };
    // Checked again right before the start, so a folder that went missing meanwhile is caught.
    let first_check = check_target(&target, ctx.repositories());
    refuse_unusable(&first_check)?;
    let mut setup = RestoreSetup {
        ctx,
        params,
        target,
        check: first_check,
        from_preview: None,
        created: Vec::new(),
    };
    let required = preview_required(&setup.target, &setup.check);
    let facts = matching_preview(
        &shared.lock_unpoisoned(),
        request.preview_id.as_deref(),
        &setup,
    );
    if let Some(f) = facts
        .as_ref()
        .filter(|f| f.target_code == RestoreTargetCode::InsideRepository)
    {
        return Err(invalid(format!(
            "{}: {}",
            f.target_code.as_str(),
            setup.check.resolved.display()
        )));
    }
    // Where a preview is required, only one that ran the dry runs counts: a preview of a folder
    // that was empty then lists nothing that would be overwritten now.
    if required && !facts.as_ref().is_some_and(|f| f.dry_run) {
        return Err(KesticServiceErr::PreviewRequired(
            "no finished, fresh preview with these parameters".to_string(),
        ));
    }
    setup.from_preview = facts;
    Ok((setup, deps, supervision))
}

/// Accepts a checked restore: a second restore into the same target folder is refused while
/// the first one starts, waits or runs. Returns the new run, not yet recorded.
fn claim(shared: &Arc<Mutex<DaemonCore>>, setup: &RestoreSetup) -> Result<Run, KesticServiceErr> {
    let mut guard = shared.lock_unpoisoned();
    let dest_id = &setup.ctx.destination.id;
    // Removed while the request was checked.
    if guard.config.destination(dest_id).is_none() {
        return Err(KesticServiceErr::NotFound(dest_id.to_string()));
    }
    let target = &setup.check.resolved;
    if guard.restore_claims.values().any(|c| &c.target == target) {
        return Err(KesticServiceErr::AlreadyRunning(format!(
            "a restore into {} already runs or waits",
            target.display()
        )));
    }
    // The preview named the snapshot's backup already; without one the worker sets it once
    // restic has read the snapshot.
    let backup_id = setup
        .from_preview
        .as_ref()
        .and_then(|f| f.backup.as_ref())
        .map(|(id, _)| id.clone());
    let run = Run::new_restore(
        guard.clock.as_ref(),
        setup.ctx.destination.id.clone(),
        backup_id,
    )
    .map_err(|e| KesticServiceErr::Internal(e.to_string()))?;
    guard.restore_claims.insert(
        run.id.clone(),
        RestoreClaim {
            destination_id: setup.ctx.destination.id.clone(),
            target: target.clone(),
        },
    );
    Ok(run)
}

/// Probes restic and creates the new folder of a claimed restore.
fn accept(
    shared: &Arc<Mutex<DaemonCore>>,
    setup: &mut RestoreSetup,
    deps: &ResticDeps,
) -> Result<ResolvedRestic, KesticServiceErr> {
    let resolved = match probe_for_run(&deps.provider, deps.secret_store.as_ref()) {
        (_, Ok(resolved)) => resolved,
        (probe, Err(err)) => {
            shared.lock_unpoisoned().record_probe(probe);
            return Err(err);
        }
    };
    make_target(setup)?;
    Ok(resolved)
}

/// Starts a restore (`StartRestore`); returns its run ID. While maintenance runs on the
/// destination, the run waits as `ausstehend` (queue) and starts after it.
pub fn start_restore_async(
    shared: &Arc<Mutex<DaemonCore>>,
    request_json: &str,
) -> Result<String, KesticServiceErr> {
    let request: RestoreRequest = serde_json::from_str(request_json)
        .map_err(|e| invalid(format!("invalid restore request JSON: {e}")))?;
    let (mut setup, deps, supervision) = prepare_setup(shared, &request)?;
    let mut run = claim(shared, &setup)?;
    let resolved = match accept(shared, &mut setup, &deps) {
        Ok(resolved) => resolved,
        Err(err) => {
            remove_created_if_empty(&setup);
            shared.lock_unpoisoned().release_restore(&run.id);
            return Err(err);
        }
    };

    let mut guard = shared.lock_unpoisoned();
    let dest_id = setup.ctx.destination.id.clone();
    let run_id = run.id.as_str().to_string();
    if guard.has_exclusive_run(&dest_id) {
        run.status = RunStatus::Pending;
        run.pending_reason = Some(PendingReason::Queued);
        if let Err(err) = guard.upsert_run_cached(&run, &[]) {
            remove_created_if_empty(&setup);
            guard.release_restore(&run.id);
            return Err(KesticServiceErr::Internal(err.to_string()));
        }
        let conn = guard.conn.clone();
        guard.pending_restores.push(PendingRestore {
            run: run.clone(),
            setup,
        });
        guard.push_state();
        if let Some(conn) = conn {
            emit_run_updated(&conn, &run);
        }
        return Ok(run_id);
    }
    launch(shared, &mut guard, run, setup, deps, supervision, resolved)?;
    Ok(run_id)
}

/// Registers `run` as active and starts its worker thread. A run that cannot be recorded ends
/// its claim and leaves no new folder behind.
fn launch(
    shared: &Arc<Mutex<DaemonCore>>,
    guard: &mut DaemonCore,
    mut run: Run,
    setup: RestoreSetup,
    deps: ResticDeps,
    supervision: SupervisionConfig,
    resolved: ResolvedRestic,
) -> Result<(), KesticServiceErr> {
    run.status = RunStatus::Running;
    run.pending_reason = None;
    run.started_at = guard.clock.now_utc();
    if let Err(err) = guard.upsert_run_cached(&run, &[]) {
        remove_created_if_empty(&setup);
        guard.release_restore(&run.id);
        return Err(KesticServiceErr::Internal(err.to_string()));
    }
    let ActivatedRun {
        cancel,
        conn_opt,
        tray_opt,
    } = guard.activate_run(
        NewActiveRun {
            run: &run,
            backup_id: None,
            display_name: &setup.ctx.destination.name,
            dest_id: &setup.ctx.destination.id,
        },
        None,
    );
    let shared_worker = Arc::clone(shared);
    thread::spawn(move || {
        restore_worker(RestoreWorker {
            shared: shared_worker,
            run,
            setup,
            deps,
            supervision,
            resolved,
            cancel,
            conn: conn_opt,
            tray: tray_opt,
        });
    });
    Ok(())
}

impl DaemonCore {
    /// Ends the claim of restore `run_id` (its run has ended or never started).
    fn release_restore(&mut self, run_id: &RunId) {
        self.restore_claims.remove(run_id);
    }

    /// Whether a restore of `dest_id` starts, waits or runs.
    pub(super) fn has_restore_on(&self, dest_id: &DestinationId) -> bool {
        self.restore_claims
            .values()
            .any(|c| &c.destination_id == dest_id)
    }

    /// Ends the waiting restore `run_id` as cancelled; `false` when no restore waits under it.
    pub(super) fn cancel_pending_restore(&mut self, run_id: &RunId) -> bool {
        let Some(pos) = self
            .pending_restores
            .iter()
            .position(|p| &p.run.id == run_id)
        else {
            return false;
        };
        let PendingRestore { mut run, setup } = self.pending_restores.remove(pos);
        let clock = Arc::clone(&self.clock);
        run.finish(clock.as_ref(), RunStatus::Cancelled, None);
        run.summary_message = Some(tr(&ErrorKind::Cancelled.message(None, false)));
        self.end_pending(&run, &setup);
        true
    }

    /// Records the end of a restore that never started and ends its claim.
    fn end_pending(&mut self, run: &Run, setup: &RestoreSetup) {
        remove_created_if_empty(setup);
        self.release_restore(&run.id);
        if let Err(err) = self.upsert_run_cached(run, &[]) {
            eprintln!("kestic-daemon: failed to record restore {}: {err}", run.id);
        }
        let conn = self.conn.clone();
        self.push_state();
        if let Some(conn) = conn {
            emit_run_updated(&conn, run);
        }
    }
}

/// Checks the target of a restore again right before restic starts: the folder must still be
/// the one that was checked (and previewed), it must exist, and where it now needs a preview,
/// the one the restore was accepted with must have run the dry runs. The age of that preview
/// counted when the restore was accepted. The reason goes into the run history, so it is said
/// in the daemon's language.
fn recheck_target(setup: &mut RestoreSetup) -> Result<(), String> {
    let accepted = setup.check.resolved.clone();
    setup.check = check_target(&setup.target, setup.ctx.repositories());
    let path = setup.check.resolved.to_string_lossy().into_owned();
    if setup.check.resolved.to_str().is_none() || setup.check.resolved != accepted {
        return Err(tr_text(&ErrorKind::InvalidTarget.text(Some(&path))));
    }
    // The folder was there (or created) when the restore was accepted; restic would create a
    // missing one with all its parents.
    let code = if setup.check.missing {
        RestoreTargetCode::MissingMappedDir
    } else {
        setup.check.code
    };
    if !matches!(
        code,
        RestoreTargetCode::Ok | RestoreTargetCode::ExistsNotEmpty
    ) {
        return Err(code.message(&path).map_or_else(|| path.clone(), |m| tr(&m)));
    }
    if preview_required(&setup.target, &setup.check)
        && !setup.from_preview.as_ref().is_some_and(|f| f.dry_run)
    {
        return Err(tr(&ServiceErrorCode::PreviewRequired.message()));
    }
    Ok(())
}

/// Starts the waiting restores of every destination (after waking up).
pub(crate) fn start_all_pending_restores(shared: &Arc<Mutex<DaemonCore>>) {
    let mut dests: Vec<DestinationId> = Vec::new();
    for pending in &shared.lock_unpoisoned().pending_restores {
        if !dests.contains(&pending.run.destination_id) {
            dests.push(pending.run.destination_id.clone());
        }
    }
    for dest in dests {
        start_pending_restores(shared, &dest);
    }
}

/// Starts the restores that waited for maintenance on `dest_id` to end.
pub(crate) fn start_pending_restores(shared: &Arc<Mutex<DaemonCore>>, dest_id: &DestinationId) {
    let waiting: Vec<RunId> = shared
        .lock_unpoisoned()
        .pending_restores
        .iter()
        .filter(|p| &p.run.destination_id == dest_id)
        .map(|p| p.run.id.clone())
        .collect();
    for run_id in waiting {
        start_pending_restore(shared, dest_id, &run_id);
    }
}

/// Starts the waiting restore `run_id`. It stays in the queue (and so can be cancelled) while
/// its target is checked and restic is probed; only under the lock it leaves the queue and
/// becomes active in one step.
fn start_pending_restore(shared: &Arc<Mutex<DaemonCore>>, dest_id: &DestinationId, run_id: &RunId) {
    let (mut setup, deps) = {
        let guard = shared.lock_unpoisoned();
        // Nothing starts while the system goes to sleep; waking up starts the waiting ones.
        if guard.sleep_in_progress || guard.has_exclusive_run(dest_id) {
            return;
        }
        let Some(pending) = guard.pending_restores.iter().find(|p| &p.run.id == run_id) else {
            return;
        };
        (pending.setup.clone(), guard.restic_deps())
    };
    let ready = recheck_target(&mut setup).and_then(|()| {
        probe_for_run(&deps.provider, deps.secret_store.as_ref())
            .1
            .map_err(|e| e.to_string())
    });
    let mut guard = shared.lock_unpoisoned();
    // Cancelled meanwhile, or started by another caller.
    let Some(pos) = guard
        .pending_restores
        .iter()
        .position(|p| &p.run.id == run_id)
    else {
        return;
    };
    // Maintenance or a backup's `forget` may have claimed the destination meanwhile.
    if guard.sleep_in_progress || guard.has_exclusive_run(dest_id) {
        return;
    }
    let PendingRestore { mut run, .. } = guard.pending_restores.remove(pos);
    let supervision = guard.supervision_config();
    let started = ready.and_then(|resolved| {
        launch(
            shared,
            &mut guard,
            run.clone(),
            setup.clone(),
            deps,
            supervision,
            resolved,
        )
        .map_err(|e| e.to_string())
    });
    if let Err(detail) = started {
        let clock = Arc::clone(&guard.clock);
        run.finish(clock.as_ref(), RunStatus::Error, None);
        run.summary_message = Some(detail);
        guard.end_pending(&run, &setup);
    }
}

struct RestoreWorker {
    shared: Arc<Mutex<DaemonCore>>,
    run: Run,
    setup: RestoreSetup,
    deps: ResticDeps,
    supervision: SupervisionConfig,
    resolved: ResolvedRestic,
    cancel: CancelHandle,
    conn: Option<Connection>,
    tray: Option<Arc<TrayHandle>>,
}

/// What the restic part of a restore ended with.
struct RestoreOutcome {
    plan: Option<RestorePlan>,
    result: Result<(RestoreRunResult, ResticExecutionResult), BackupRunError>,
}

impl RestoreWorker {
    /// Shows the backup of the snapshot on the run once restic named it.
    fn set_backup(&mut self, backup: Option<(BackupId, String)>) {
        let Some((backup_id, name)) = backup else {
            return;
        };
        self.run.kind = RunKind::Restore {
            backup_id: Some(backup_id),
        };
        let mut guard = self.shared.lock_unpoisoned();
        let _ = guard.upsert_run_cached(&self.run, &[]);
        if let Some(entry) = guard.active_runs.get_mut(&self.run.id) {
            entry.run = self.run.clone();
            entry.backup_name.clone_from(&name);
            entry.progress.backup_name = name;
            // Only the progress names the backup: the run must not count as that backup running.
            entry.progress.backup_id = self.run.backup_id().map(|id| id.as_str().to_string());
        }
        drop(guard);
        if let Some(conn) = &self.conn {
            emit_run_updated(conn, &self.run);
        }
    }

    fn report(&self, status: &ResticStatusMessage) {
        let mut guard = self.shared.lock_unpoisoned();
        let mut progress = None;
        if let Some(entry) = guard.active_runs.get_mut(&self.run.id) {
            let total = status.total_bytes.unwrap_or(0);
            entry.progress.phase = if total > 0 {
                RunProgressPhase::ScanFinished
            } else {
                RunProgressPhase::Scanning
            };
            let percent = (status.percent_done * 100.0).clamp(0.0, 100.0);
            entry.progress.percent_done = entry.progress.percent_done.max(percent);
            entry.progress.bytes_done = status.bytes_done.unwrap_or(0);
            entry.progress.total_bytes = total;
            entry.progress.files_done = status.files_done.unwrap_or(0);
            entry.progress.seconds_remaining = status.seconds_remaining.unwrap_or(0);
            progress = Some(entry.progress.clone());
        }
        let (_, tray_state) = guard.build_state_and_tray();
        drop(guard);
        if let Some(tray) = &self.tray {
            tray.set_full_state(tray_state);
        }
        if let (Some(conn), Some(progress)) = (&self.conn, progress) {
            emit_run_progress(conn, &progress);
        }
    }

    fn execute(&mut self) -> RestoreOutcome {
        let kestic_binary = self.deps.kestic_binary.clone();
        let secret_store = Arc::clone(&self.deps.secret_store);
        let clock = Arc::clone(&self.deps.clock);
        let resolved = self.resolved.clone();
        let destination = self.setup.ctx.destination.clone();
        let access = SnapshotListAccess {
            destination: &destination,
            resolved: &resolved,
            kestic_binary: &kestic_binary,
            secret_store: secret_store.as_ref(),
            clock: clock.as_ref(),
            extra_env: &[],
        };
        let plan = match self.setup.from_preview.clone() {
            Some(facts) => {
                self.set_backup(facts.backup);
                facts.plan
            }
            None => match resolve_plan(&access, &self.setup.params, Some(&self.cancel)) {
                Ok((snapshot, plan)) => {
                    self.set_backup(self.setup.ctx.backup_of(&snapshot));
                    plan
                }
                Err(err) => {
                    return RestoreOutcome {
                        plan: None,
                        result: Err(err),
                    };
                }
            },
        };
        // The probe and the plan may have taken a while (the password store, restic reading
        // the snapshot): the folder is checked once more right before restic writes into it.
        if let Err(reason) = recheck_target(&mut self.setup) {
            return RestoreOutcome {
                plan: None,
                result: Err(BackupRunError::InvalidTarget {
                    path: self.setup.check.resolved.to_string_lossy().into_owned(),
                    reason,
                }),
            };
        }
        // A folder above the selection that became a link or a file after the preview would
        // make restic write into another folder or fail midway.
        if let Some(path) = blocked_above_selection(
            &plan,
            &self.setup.params.selection,
            &self.setup.check.resolved,
        )
        .into_iter()
        .next()
        {
            return RestoreOutcome {
                plan: None,
                result: Err(BackupRunError::InvalidTarget {
                    reason: tr_text(&ErrorKind::InvalidTarget.text(Some(&path))),
                    path,
                }),
            };
        }
        // Cancelled while the target was checked: restic does not start at all.
        if self.cancel.is_cancelled() {
            return RestoreOutcome {
                plan: None,
                result: Err(BackupRunError::Cancelled),
            };
        }
        let target = self.setup.check.resolved.clone();
        let overwrite = self.setup.params.overwrite;
        let cancel = self.cancel.clone();
        let result = execute_restore(
            &access,
            &plan,
            &target,
            overwrite,
            Some(&cancel),
            &mut |st| {
                self.report(st);
            },
        );
        RestoreOutcome {
            plan: Some(plan),
            result,
        }
    }
}

fn restore_worker(mut worker: RestoreWorker) {
    let mut supervision = RunSupervision::start(
        &worker.supervision,
        &i18n("Restoring from %1").subs(&[&worker.setup.ctx.destination.name]),
        &worker.cancel,
    );
    let outcome = worker.execute();
    supervision.stop_cancel_watcher();

    let secrets_owned = load_dest_secrets(
        &worker.setup.ctx.destination,
        worker.deps.secret_store.as_ref(),
    );
    let secrets = secret_strs(&secrets_owned);
    let mut run = worker.run.clone();
    let status = record_outcome(&mut run, &worker.setup, outcome, &secrets, &worker.shared);
    run.finish(worker.deps.clock.as_ref(), status, run.exit_code);
    if let Some(jv) = supervision.job_view.take() {
        let code = u32::from(status != RunStatus::Ok);
        let _ = jv.terminate(code, run.summary_message.as_deref().unwrap_or_default());
    }
    if let Some(inh) = supervision.logind.as_mut() {
        inh.release();
    }
    finalize(&worker, &run, &secrets);
}

/// Fills the result of `run` from the restic outcome and returns its status.
fn record_outcome(
    run: &mut Run,
    setup: &RestoreSetup,
    outcome: RestoreOutcome,
    secrets: &[&str],
    shared: &Arc<Mutex<DaemonCore>>,
) -> RunStatus {
    let mut stats = RestoreStats {
        target_dir: setup.check.resolved.to_string_lossy().into_owned(),
        overwrite: setup.params.overwrite,
        selection_count: setup.params.selection.len() as u64,
        files_kept: setup.from_preview.as_ref().map_or(0, |f| f.files_kept),
        ..RestoreStats::default()
    };
    let status = match outcome.result {
        Ok((result, res)) => {
            stats.files_new = result.files_new;
            stats.files_overwritten = result.files_overwritten;
            stats.bytes_restored = result.bytes_restored;
            run.exit_code = Some(res.exit_code);
            run.commands = vec![res.command_record];
            run.log_path = shared
                .lock_unpoisoned()
                .history
                .write_redacted_log(&run.id, &res.log_lines, secrets)
                .ok();
            if result.nothing_matched {
                run.summary_message = Some(tr(&nothing_restored_message()));
                RunStatus::Warning
            } else {
                let files = result.files_new + result.files_overwritten;
                run.summary_message =
                    Some(i18np("Restored %1 file", "Restored %1 files", files, &[]));
                RunStatus::Ok
            }
        }
        // Stopped because the system goes to sleep: interrupted, not cancelled by the user.
        Err(BackupRunError::Cancelled)
            if shared.lock_unpoisoned().run_sleep_interrupted(&run.id) =>
        {
            run.summary_message = Some(tr(&interrupted_by_sleep_message()));
            RunStatus::Interrupted
        }
        Err(BackupRunError::Cancelled) => {
            run.summary_message = Some(tr(&ErrorKind::Cancelled.message(None, false)));
            RunStatus::Cancelled
        }
        Err(err) => {
            let text = match &err {
                BackupRunError::PathNotInSnapshot(_) => {
                    tr(&backup_run_error_kind(&err).message(None, false))
                }
                // The target check right before the run says why, in the daemon's language.
                BackupRunError::InvalidTarget { reason, .. } => reason.clone(),
                other => localized_backup_run_error(other),
            };
            let detail = redact_secrets(&err.to_string(), secrets);
            let kind = backup_run_error_kind(&err);
            run.summary_message = Some(redact_secrets(&text, secrets));
            run.exit_code = match err {
                BackupRunError::Destination(
                    kestic_core::destination::DestinationError::CommandFailed { exit_code, .. },
                ) => Some(exit_code),
                _ => run.exit_code,
            };
            run.log_path = shared
                .lock_unpoisoned()
                .history
                .write_redacted_log(&run.id, &[error_kind_line(kind), detail], secrets)
                .ok();
            RunStatus::Error
        }
    };
    if outcome.plan.is_some() || status == RunStatus::Ok {
        run.restore = Some(stats);
    }
    status
}

const ERROR_KIND_LINE: &str = "error_kind: ";

/// The first log line of a failed restore names its error code, so the run detail can show it
/// after a restart.
fn error_kind_line(kind: ErrorKind) -> String {
    format!("{ERROR_KIND_LINE}{}", kind.as_str())
}

/// The error code of a restore run from its log; `None` for a restore that did not fail.
pub(crate) fn restore_error_kind(run: &Run, log_lines: &[String]) -> Option<ErrorKind> {
    if run.status != RunStatus::Error {
        return None;
    }
    let kind = log_lines
        .iter()
        .find_map(|l| l.strip_prefix(ERROR_KIND_LINE))
        .and_then(ErrorKind::parse);
    Some(kind.unwrap_or(ErrorKind::Unknown))
}

fn finalize(worker: &RestoreWorker, run: &Run, secrets: &[&str]) {
    let run_id: RunId = run.id.clone();
    let (state_dto, window_active) = {
        let mut guard = worker.shared.lock_unpoisoned();
        guard.active_runs.remove(&run_id);
        remove_created_if_empty(&worker.setup);
        guard.release_restore(&run_id);
        if let Err(err) = guard.upsert_run_cached(run, secrets) {
            eprintln!("kestic-daemon: failed to record finished restore {run_id}: {err}");
        }
        let (state_dto, tray_state) = guard.build_state_and_tray();
        if let Some(tray) = &worker.tray {
            tray.set_full_state(tray_state);
        }
        (state_dto, guard.window_active)
    };
    if let Some(conn) = &worker.conn {
        emit_run_updated(conn, run);
        emit_status_changed(conn, &state_dto);
    }
    if !window_active {
        let title = i18n("Restore from %1 finished").subs(&[&worker.setup.ctx.destination.name]);
        let body = run.summary_message.clone().unwrap_or_default();
        let _ = send_event_notification(KesticEvent::RestoreFinished, &title, &body);
    }
}
