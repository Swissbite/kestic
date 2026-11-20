// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Asynchronous backup execution, preflight checks, progress reporting, forget catch-up, and run finalization.

use super::availability::{
    DestinationClaimGuard, check_source_status_bounded, drain_queued_backups_async,
    unlockable_stale_lock_count,
};
use super::error::{
    KesticServiceErr, localized_backup_run_error, localized_failure_summary,
    localized_source_check_reason,
};
use super::run_errors::{run_blocks_schedule, unreadable_items_from_lines};
use super::state::{ActiveRunEntry, DaemonCore, probe_before_run};
use crate::dbus_api::{ActiveRunProgressDto, DestinationAvailabilityDto};
use crate::dbus_server::{
    emit_manual_run_blocked, emit_run_progress, emit_run_updated, emit_status_changed,
};
use crate::i18n::{i18n, tr};
use crate::inhibit::PortalInhibitGuard;
use crate::job_view::{CAPABILITY_KILLABLE, JobProgress, JobViewHandle};
use crate::monitors::{LogindSleepInhibitor, MonitorBusConfig};
use crate::notifications::{
    KESTIC_DESKTOP_ENTRY, KesticEvent, send_configured_notification, send_event_notification,
};
use crate::tray::TrayHandle;
use kestic_core::backup::{
    BackupExecutionContext, BackupRunError, execute_backup_with_inspector, execute_forget,
    is_forget_lock_failure,
};
use kestic_core::clock::Clock;
use kestic_core::destination::{
    DEFAULT_CONNECT_TIMEOUT, DestinationError, preflight_check_destination,
};
use kestic_core::history::BackoffKind;
use kestic_core::maintenance::detect_stale_locks;
use kestic_core::model::{
    Backup, BackupId, Destination, PendingReason, ResticProvider, Run, RunId, RunKind, RunStatus,
};
use kestic_core::mount::{MountInspector, SourceCheckResult, current_home_dir};
use kestic_core::provider::ResolvedRestic;
use kestic_core::restic::{
    CancelHandle, ResticExecutionResult, ResticFailureKind, ResticRuntime, ResticStatusMessage,
    ResticStreamEvent, SECRET_KIND_REPO, SECRET_KIND_REST, redact_secrets,
};
use kestic_core::scheduler::{
    SystemEnvironment, UnmetPrecondition, evaluate_run_conditions, format_local_slot_hint,
    mark_backup_slot_handled, record_backup_backoff_failure,
};
use kestic_core::secrets::{SecretStore, SecretValue};
use kestic_i18n::codes::{BlockedReason, ErrorKind};
use kestic_i18n::messages::{ActiveRunInfo, active_run_message, interrupted_by_sleep_message};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::thread;
use zbus::Connection;

pub(super) fn upsert_pending_run_entry(
    guard: &mut DaemonCore,
    backup: &Backup,
    reason: PendingReason,
    summary_message: String,
) {
    let now = guard.clock.now_utc();
    let existing_pending = guard
        .runs_cache
        .iter()
        .find(|r| {
            r.backup_id() == Some(&backup.id)
                && matches!(r.kind, RunKind::Backup { .. })
                && r.status == RunStatus::Pending
        })
        .cloned();
    if let Some(mut existing) = existing_pending {
        if existing.pending_reason == Some(reason)
            && existing.summary_message.as_deref() == Some(summary_message.as_str())
        {
            return;
        }
        existing.pending_reason = Some(reason);
        existing.summary_message = Some(summary_message);
        let _ = guard.upsert_run_cached(&existing, &[]);
        return;
    }
    if let Ok(mut run) = Run::new_pending_backup(guard.clock.as_ref(), backup, reason) {
        run.started_at = now;
        run.summary_message = Some(summary_message);
        let _ = guard.upsert_run_cached(&run, &[]);
    }
}

/// Starts a backup manually (enforcing `RunConditions`, `AC-M3-03-7`).
pub fn start_backup_async(
    shared: &Arc<Mutex<DaemonCore>>,
    backup_id: &str,
) -> Result<String, KesticServiceErr> {
    start_backup_internal(shared, backup_id, false, false)
}

/// Starts a backup manually overriding unmet `RunConditions` for this single run (`FR-13b`, `AC-M3-03-7`).
pub fn start_backup_override_async(
    shared: &Arc<Mutex<DaemonCore>>,
    backup_id: &str,
) -> Result<String, KesticServiceErr> {
    start_backup_internal(shared, backup_id, true, false)
}

/// Starts a scheduled backup triggered by the M3 scheduler (`M3-S01..S13`).
pub(crate) fn start_scheduled_backup_async(
    shared: &Arc<Mutex<DaemonCore>>,
    backup_id: &str,
) -> Result<String, KesticServiceErr> {
    start_backup_internal(shared, backup_id, false, true)
}

struct BackupClaimContext {
    backup: Backup,
    dest: Destination,
    provider: ResticProvider,
    state_dir: PathBuf,
    kestic_binary: PathBuf,
    secret_store: Arc<dyn SecretStore + Send + Sync>,
    clock: Arc<dyn Clock>,
    mount_inspector: Arc<dyn MountInspector>,
    system_env: SystemEnvironment,
    enable_job_view: bool,
    job_view_dest: String,
    enable_portal_inhibit: bool,
    monitor_bus: MonitorBusConfig,
    enable_system_monitors: bool,
    claim_guard: DestinationClaimGuard,
}

fn claim_backup_destination(
    shared: &Arc<Mutex<DaemonCore>>,
    b_id: &BackupId,
    backup_id: &str,
    is_scheduled: bool,
) -> Result<BackupClaimContext, KesticServiceErr> {
    let mut guard = shared.lock().unwrap_or_else(|p| p.into_inner());
    let backup = guard
        .config
        .backup(b_id)
        .cloned()
        .ok_or_else(|| KesticServiceErr::NotFound(backup_id.to_string()))?;
    let dest = guard
        .config
        .destination(&backup.destination_id)
        .cloned()
        .ok_or_else(|| KesticServiceErr::NotFound(backup.destination_id.as_str().to_string()))?;

    if is_scheduled && guard.sleep_in_progress {
        return Err(KesticServiceErr::SleepInProgress);
    }
    if guard.is_destination_busy(&dest.id) {
        return Err(KesticServiceErr::AlreadyRunning(format!(
            "another run is already active on destination '{}'",
            dest.name
        )));
    }

    guard.claiming_destinations.insert(dest.id.clone());
    let claim_guard = DestinationClaimGuard::new(Arc::clone(shared), dest.id.clone());
    Ok(BackupClaimContext {
        backup,
        dest,
        provider: guard.config.restic_provider.clone(),
        state_dir: guard.state_dir.clone(),
        kestic_binary: guard.kestic_binary.clone(),
        secret_store: Arc::clone(&guard.secret_store),
        clock: Arc::clone(&guard.clock),
        mount_inspector: Arc::clone(&guard.mount_inspector),
        system_env: guard.system_env.clone(),
        enable_job_view: guard.enable_job_view,
        job_view_dest: guard.job_view_dest.clone(),
        enable_portal_inhibit: guard.enable_portal_inhibit,
        monitor_bus: guard.monitor_bus.clone(),
        enable_system_monitors: guard.enable_system_monitors,
        claim_guard,
    })
}

fn handle_backup_precheck_failure(
    guard: &mut DaemonCore,
    claim_guard: &mut DestinationClaimGuard,
    backup: &Backup,
    dest: &Destination,
    source_status: &SourceCheckResult,
    dest_res: Result<(), DestinationError>,
    is_scheduled: bool,
) -> KesticServiceErr {
    let now = guard.clock.now_utc();
    let source_ok = source_status.is_ok();
    let reason = if !source_ok {
        BlockedReason::SourceUnavailable
    } else {
        BlockedReason::DestinationUnreachable
    };
    let source_body = if !source_ok {
        localized_source_check_reason(source_status)
    } else {
        tr(&ErrorKind::DestinationUnreachable.message(None, false))
    };

    if is_scheduled {
        if !source_ok {
            upsert_pending_run_entry(guard, backup, PendingReason::SourceUnavailable, source_body);
        } else {
            let mut rs = guard.load_backup_runtime_state(&backup.id);
            record_backup_backoff_failure(&mut rs, now, Some(BackoffKind::Network));
            let _ = guard.store_backup_runtime_state(&backup.id, &rs);
            let pending_reason = if rs.backoff_until.is_some() {
                PendingReason::DestinationUnreachable
            } else {
                PendingReason::Backoff
            };
            let msg = match rs.backoff_until {
                Some(until) => {
                    let hint = format_local_slot_hint(until, now, &guard.timezone);
                    i18n("Destination unreachable · next attempt at %1").subs(&[&hint])
                }
                None => i18n("Destination unreachable · waiting for next scheduled run").into(),
            };
            upsert_pending_run_entry(guard, backup, pending_reason, msg);
        }
        guard.claiming_destinations.remove(&dest.id);
        claim_guard.disarm();
        let conn_opt = guard.conn.clone();
        let (state_dto, tray_state) = guard.build_state_and_tray();
        if let Some(tray) = &guard.tray {
            tray.set_full_state(tray_state);
        }
        if let Some(conn) = &conn_opt {
            emit_status_changed(conn, &state_dto);
        }
        return if !source_ok {
            KesticServiceErr::ManualRunBlocked(reason)
        } else {
            KesticServiceErr::DestinationUnreachable(dest_res.unwrap_err().to_string())
        };
    }

    guard.claiming_destinations.remove(&dest.id);
    claim_guard.disarm();
    let window_active = guard.window_active;
    let conn_opt = guard.conn.clone();
    let (state_dto, tray_state) = guard.build_state_and_tray();
    if let Some(tray) = &guard.tray {
        tray.set_full_state(tray_state);
    }
    if let Some(conn) = &conn_opt {
        emit_manual_run_blocked(conn, &backup.id, reason);
        emit_status_changed(conn, &state_dto);
    }
    if !window_active {
        let title = i18n("%1: Backup cannot start").subs(&[&backup.name]);
        let _ = send_event_notification(KesticEvent::ManualRunBlocked, &title, &source_body);
    }
    KesticServiceErr::ManualRunBlocked(reason)
}

fn handle_backup_unmet_conditions(
    guard: &mut DaemonCore,
    claim_guard: &mut DestinationClaimGuard,
    backup: &Backup,
    dest: &Destination,
    unmet_conditions: &[UnmetPrecondition],
) -> KesticServiceErr {
    let body = guard.localized_unmet_conditions(unmet_conditions);
    guard.claiming_destinations.remove(&dest.id);
    claim_guard.disarm();
    let window_active = guard.window_active;
    let conn_opt = guard.conn.clone();
    let (state_dto, tray_state) = guard.build_state_and_tray();
    if let Some(tray) = &guard.tray {
        tray.set_full_state(tray_state);
    }
    if let Some(conn) = &conn_opt {
        emit_manual_run_blocked(conn, &backup.id, BlockedReason::ConditionNotMet);
        emit_status_changed(conn, &state_dto);
    }
    if !window_active {
        let title = i18n("%1: Backup cannot start").subs(&[&backup.name]);
        let _ = send_event_notification(KesticEvent::ManualRunBlocked, &title, &body);
    }
    KesticServiceErr::ManualRunBlocked(BlockedReason::ConditionNotMet)
}

struct LaunchedBackupRun {
    pending_other_backups: Vec<Backup>,
    run: Run,
    cancel: CancelHandle,
    conn_opt: Option<Connection>,
    tray_opt: Option<Arc<TrayHandle>>,
}

fn launch_backup_run_record(
    guard: &mut DaemonCore,
    claim_guard: &mut DestinationClaimGuard,
    backup: &Backup,
    dest: &Destination,
) -> Result<LaunchedBackupRun, KesticServiceErr> {
    let _ = guard.delete_pending_runs_cached(&backup.id);
    guard.sleep_interrupted_backups.remove(&backup.id);
    guard.queued_backups.retain(|id| id != &backup.id);

    let pending_other_backups: Vec<Backup> = guard
        .config
        .backups
        .iter()
        .filter(|b| {
            b.id != backup.id
                && b.destination_id == dest.id
                && guard.is_pending_forget_cached(&b.id)
        })
        .cloned()
        .collect();

    let run = Run::new_backup(guard.clock.as_ref(), backup)
        .map_err(|e| KesticServiceErr::Internal(e.to_string()))?;
    guard
        .upsert_run_cached(&run, &[])
        .map_err(|e| KesticServiceErr::Internal(e.to_string()))?;

    let cancel = CancelHandle::new();
    let entry = ActiveRunEntry {
        run: run.clone(),
        backup_id: Some(backup.id.clone()),
        backup_name: backup.name.clone(),
        progress: ActiveRunProgressDto {
            run_id: run.id.as_str().to_string(),
            backup_id: Some(backup.id.as_str().to_string()),
            backup_name: backup.name.clone(),
            percent_done: 0.0,
            bytes_done: 0,
            total_bytes: 0,
            speed_bps: 0,
            seconds_remaining: 0,
        },
        cancel: cancel.clone(),
        sleep_interrupted: false,
    };
    guard.active_runs.insert(run.id.clone(), entry);
    guard.claiming_destinations.remove(&dest.id);
    claim_guard.disarm();

    let conn_opt = guard.conn.clone();
    let tray_opt = guard.tray.clone();
    let (state_dto, tray_state) = guard.build_state_and_tray();
    if let Some(tray) = &tray_opt {
        tray.set_full_state(tray_state);
    }
    if let Some(conn) = &conn_opt {
        emit_run_updated(conn, &run);
        emit_status_changed(conn, &state_dto);
    }
    Ok(LaunchedBackupRun {
        pending_other_backups,
        run,
        cancel,
        conn_opt,
        tray_opt,
    })
}

fn start_backup_internal(
    shared: &Arc<Mutex<DaemonCore>>,
    backup_id: &str,
    allow_condition_override: bool,
    is_scheduled: bool,
) -> Result<String, KesticServiceErr> {
    let b_id =
        BackupId::parse(backup_id).map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;

    let mut ctx = claim_backup_destination(shared, &b_id, backup_id, is_scheduled)?;

    let (probe, resolved) = probe_before_run(
        shared,
        &ctx.provider,
        ctx.secret_store.as_ref(),
        &ctx.dest.id,
        &mut ctx.claim_guard,
    )?;
    let source_status = check_source_status_bounded(
        &ctx.backup,
        current_home_dir().as_deref(),
        &ctx.mount_inspector,
    );
    let source_ok = source_status.is_ok();
    let dest_res = if source_ok {
        preflight_check_destination(&ctx.dest, DEFAULT_CONNECT_TIMEOUT)
    } else {
        Ok(())
    };
    let unmet_conditions =
        if source_ok && dest_res.is_ok() && !is_scheduled && !allow_condition_override {
            evaluate_run_conditions(&ctx.backup.run_conditions, &ctx.system_env)
        } else {
            Vec::new()
        };

    let launched = {
        let mut guard = shared.lock().unwrap_or_else(|p| p.into_inner());
        guard.record_probe(probe);
        if is_scheduled && guard.sleep_in_progress {
            guard.claiming_destinations.remove(&ctx.dest.id);
            ctx.claim_guard.disarm();
            return Err(KesticServiceErr::SleepInProgress);
        }

        let now = guard.clock.now_utc();
        guard
            .source_availability
            .insert(ctx.backup.id.clone(), source_status.clone());
        if source_ok {
            guard.availability_probe_count += 1;
            let dto = match &dest_res {
                Ok(()) => DestinationAvailabilityDto {
                    destination_id: ctx.dest.id.as_str().to_string(),
                    reachable: true,
                    detail: None,
                    checked_at_unix: now.unix_timestamp(),
                },
                Err(e) => DestinationAvailabilityDto {
                    destination_id: ctx.dest.id.as_str().to_string(),
                    reachable: false,
                    detail: Some(e.to_string()),
                    checked_at_unix: now.unix_timestamp(),
                },
            };
            guard.dest_availability.insert(ctx.dest.id.clone(), dto);
        }

        if !source_ok || dest_res.is_err() {
            return Err(handle_backup_precheck_failure(
                &mut guard,
                &mut ctx.claim_guard,
                &ctx.backup,
                &ctx.dest,
                &source_status,
                dest_res,
                is_scheduled,
            ));
        }
        if !unmet_conditions.is_empty() {
            return Err(handle_backup_unmet_conditions(
                &mut guard,
                &mut ctx.claim_guard,
                &ctx.backup,
                &ctx.dest,
                &unmet_conditions,
            ));
        }

        launch_backup_run_record(&mut guard, &mut ctx.claim_guard, &ctx.backup, &ctx.dest)?
    };

    let run_id_str = launched.run.id.as_str().to_string();
    let shared_worker = Arc::clone(shared);
    thread::spawn(move || {
        execute_backup_worker(BackupWorkerArgs {
            shared_worker,
            ctx,
            resolved,
            pending_other_backups: launched.pending_other_backups,
            run: launched.run,
            cancel: launched.cancel,
            is_scheduled,
            conn_opt: launched.conn_opt,
            tray_opt: launched.tray_opt,
        });
    });

    Ok(run_id_str)
}

struct BackupWorkerArgs {
    shared_worker: Arc<Mutex<DaemonCore>>,
    ctx: BackupClaimContext,
    resolved: ResolvedRestic,
    pending_other_backups: Vec<Backup>,
    run: Run,
    cancel: CancelHandle,
    is_scheduled: bool,
    conn_opt: Option<Connection>,
    tray_opt: Option<Arc<TrayHandle>>,
}

struct BackupProgressReporter<'a> {
    backup_name: &'a str,
    start_unix: i64,
    clock: &'a dyn Clock,
    job_view: Option<&'a JobViewHandle>,
    shared_worker: &'a Arc<Mutex<DaemonCore>>,
    run_id: &'a RunId,
    tray_opt: Option<&'a Arc<TrayHandle>>,
    conn_opt: Option<&'a Connection>,
}

impl BackupProgressReporter<'_> {
    fn report(&self, st: &ResticStatusMessage) {
        let pct = (st.percent_done * 100.0).clamp(0.0, 100.0);
        let bytes_done = st.bytes_done.unwrap_or(0);
        let total_bytes = st.total_bytes.unwrap_or(0);
        let elapsed_secs = (self.clock.now_unix_secs() - self.start_unix).max(1) as u64;
        let speed_bps = st
            .seconds_elapsed
            .filter(|s| *s > 0)
            .map(|s| bytes_done / s)
            .unwrap_or(bytes_done / elapsed_secs);
        let seconds_remaining = st.seconds_remaining.unwrap_or(0);

        if let Some(jv) = self.job_view {
            let _ = jv.update(&JobProgress {
                percent: pct.round() as u32,
                processed_bytes: bytes_done,
                total_bytes,
                speed_bytes_per_sec: speed_bps,
                info_message: tr(&active_run_message(&ActiveRunInfo {
                    name: self.backup_name,
                    is_maintenance: false,
                    percent_done: pct,
                })),
                elapsed_ms: elapsed_secs * 1000,
            });
        }

        let mut guard = self.shared_worker.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(entry) = guard.active_runs.get_mut(self.run_id) {
            entry.progress.percent_done = pct;
            entry.progress.bytes_done = bytes_done;
            entry.progress.total_bytes = total_bytes;
            entry.progress.speed_bps = speed_bps;
            entry.progress.seconds_remaining = seconds_remaining;
        }
        let (_, tray_state) = guard.build_state_and_tray();
        if let Some(tray) = self.tray_opt {
            tray.set_full_state(tray_state);
        }
        drop(guard);

        if let Some(conn) = self.conn_opt {
            emit_run_progress(
                conn,
                self.run_id.as_str(),
                pct,
                bytes_done,
                total_bytes,
                speed_bps,
                seconds_remaining,
            );
        }
    }
}

struct BackupPostExecOutcome {
    final_status: RunStatus,
    exit_code: Option<i32>,
    raw_detail: Option<String>,
    jv_code: u32,
    jv_msg: String,
    primary_forget_state: Option<bool>,
    cleared_other_pending: Vec<BackupId>,
    detected_stale_locks: Option<usize>,
    transient_backoff_kind: Option<BackoffKind>,
    /// Unreadable items parsed from this run's log lines (cached for state building).
    unreadable_items: Vec<String>,
}

fn execute_backup_worker(args: BackupWorkerArgs) {
    let BackupWorkerArgs {
        shared_worker,
        ctx,
        resolved,
        pending_other_backups,
        mut run,
        cancel,
        is_scheduled,
        conn_opt,
        tray_opt,
    } = args;

    let mut logind_sleep_guard = if ctx.enable_system_monitors {
        LogindSleepInhibitor::acquire(&ctx.monitor_bus)
    } else {
        None
    };
    let _inhibit_guard = if ctx.enable_portal_inhibit {
        PortalInhibitGuard::acquire_default(&format!("Kestic backup: {}", ctx.backup.name)).ok()
    } else {
        None
    };

    let job_view = if ctx.enable_job_view {
        JobViewHandle::request(
            &ctx.job_view_dest,
            KESTIC_DESKTOP_ENTRY,
            &i18n("Backing up %1").subs(&[&ctx.backup.name]),
            CAPABILITY_KILLABLE,
        )
        .ok()
    } else {
        None
    };
    let job_view_cancel_guard = job_view.as_ref().map(|jv| {
        let cancel_for_jv = cancel.clone();
        jv.start_cancel_watcher(move || cancel_for_jv.cancel())
    });

    let run_id = run.id.clone();
    let start_unix = ctx.clock.now_unix_secs();
    let home = current_home_dir();

    let runtime = ResticRuntime {
        resolved: &resolved,
        kestic_binary: &ctx.kestic_binary,
        secret_store: ctx.secret_store.as_ref(),
        clock: ctx.clock.as_ref(),
        cache_dir: None,
    };
    let exec_ctx = BackupExecutionContext {
        state_dir: &ctx.state_dir,
        home_dir: home.as_deref(),
        inspector: ctx.mount_inspector.as_ref(),
        cancel: Some(&cancel),
        extra_env: &[],
    };
    let reporter = BackupProgressReporter {
        backup_name: &ctx.backup.name,
        start_unix,
        clock: ctx.clock.as_ref(),
        job_view: job_view.as_ref(),
        shared_worker: &shared_worker,
        run_id: &run_id,
        tray_opt: tray_opt.as_ref(),
        conn_opt: conn_opt.as_ref(),
    };
    let exec_res =
        execute_backup_with_inspector(&ctx.backup, &ctx.dest, &runtime, &exec_ctx, |ev| {
            if let ResticStreamEvent::Status(st) = ev {
                reporter.report(st);
            }
        });
    drop(job_view_cancel_guard);

    let sleep_interrupted = {
        let guard = shared_worker.lock().unwrap_or_else(|p| p.into_inner());
        guard
            .active_runs
            .get(&run_id)
            .is_some_and(|e| e.sleep_interrupted)
    };

    let secrets_owned = load_dest_secrets(&ctx.dest, ctx.secret_store.as_ref());
    let secret_strs: Vec<&str> = secrets_owned
        .iter()
        .filter_map(|s| s.as_str().ok())
        .collect();

    let outcome = process_backup_exec_result(BackupExecInputs {
        shared_worker: &shared_worker,
        backup: &ctx.backup,
        dest: &ctx.dest,
        runtime: &runtime,
        pending_other_backups: &pending_other_backups,
        run: &mut run,
        exec_res,
        cancel: &cancel,
        sleep_interrupted,
        is_scheduled,
        secret_strs: &secret_strs,
    });

    run.finish(ctx.clock.as_ref(), outcome.final_status, outcome.exit_code);
    if let Some(jv) = job_view {
        let _ = jv.terminate(outcome.jv_code, &outcome.jv_msg);
    }

    finalize_backup_run_state(
        &shared_worker,
        &ctx,
        run,
        outcome,
        &secret_strs,
        logind_sleep_guard.as_mut(),
        (conn_opt.as_ref(), tray_opt.as_ref()),
    );
}

pub(super) fn load_dest_secrets(
    dest: &Destination,
    secret_store: &dyn SecretStore,
) -> Vec<SecretValue> {
    let mut secrets_owned = Vec::new();
    if let Ok(Some(s)) = secret_store.get_secret(dest.id.as_str(), SECRET_KIND_REPO) {
        secrets_owned.push(s);
    }
    if dest.kind.uses_rest_credentials()
        && let Ok(Some(s)) = secret_store.get_secret(dest.id.as_str(), SECRET_KIND_REST)
    {
        secrets_owned.push(s);
    }
    secrets_owned
}

struct BackupExecInputs<'a> {
    shared_worker: &'a Arc<Mutex<DaemonCore>>,
    backup: &'a Backup,
    dest: &'a Destination,
    runtime: &'a ResticRuntime<'a>,
    pending_other_backups: &'a [Backup],
    run: &'a mut Run,
    exec_res: Result<ResticExecutionResult, BackupRunError>,
    cancel: &'a CancelHandle,
    sleep_interrupted: bool,
    is_scheduled: bool,
    secret_strs: &'a [&'a str],
}

fn process_backup_exec_result(inputs: BackupExecInputs<'_>) -> BackupPostExecOutcome {
    let BackupExecInputs {
        shared_worker,
        backup,
        dest,
        runtime,
        pending_other_backups,
        run,
        exec_res,
        cancel,
        sleep_interrupted,
        is_scheduled,
        secret_strs,
    } = inputs;
    match exec_res {
        Ok(res) => process_backup_success_result(
            BackupSuccessContext {
                shared_worker,
                backup,
                dest,
                runtime,
                pending_other_backups,
                cancel,
                sleep_interrupted,
                is_scheduled,
                secret_strs,
            },
            run,
            res,
        ),
        Err(err) => {
            process_backup_error_result(run, err, sleep_interrupted, is_scheduled, secret_strs)
        }
    }
}

struct BackupSuccessContext<'a> {
    shared_worker: &'a Arc<Mutex<DaemonCore>>,
    backup: &'a Backup,
    dest: &'a Destination,
    runtime: &'a ResticRuntime<'a>,
    pending_other_backups: &'a [Backup],
    cancel: &'a CancelHandle,
    sleep_interrupted: bool,
    is_scheduled: bool,
    secret_strs: &'a [&'a str],
}

fn process_backup_success_result(
    ctx: BackupSuccessContext<'_>,
    run: &mut Run,
    res: ResticExecutionResult,
) -> BackupPostExecOutcome {
    let BackupSuccessContext {
        shared_worker,
        backup,
        dest,
        runtime,
        pending_other_backups,
        cancel,
        sleep_interrupted,
        is_scheduled,
        secret_strs,
    } = ctx;
    let mut primary_forget_state: Option<bool> = None;
    let mut cleared_other_pending: Vec<BackupId> = Vec::new();
    let mut detected_stale_locks: Option<usize> = None;
    let mut transient_backoff_kind: Option<BackoffKind> = None;

    let mut status = if sleep_interrupted {
        RunStatus::Interrupted
    } else {
        res.presentation.status
    };
    let backup_exit = res.exit_code;
    let mut combined_logs = res.log_lines;
    run.commands = vec![res.command_record];
    run.stats = res.stats;

    let mut summary = if sleep_interrupted {
        Some(tr(&interrupted_by_sleep_message()))
    } else if matches!(status, RunStatus::Error | RunStatus::Warning) {
        let base = localized_failure_summary(&res.presentation.kind);
        let raw = res
            .presentation
            .raw_detail
            .as_deref()
            .map(|d| redact_secrets(d, secret_strs))
            .unwrap_or_default();
        Some(if raw.is_empty() {
            base
        } else {
            format!("{base}: {raw}")
        })
    } else {
        Some(localized_failure_summary(&res.presentation.kind))
    };

    if !sleep_interrupted
        && (backup_exit == 11
            || matches!(res.presentation.kind, ResticFailureKind::RepositoryLocked))
    {
        if let Ok(lock_class) = detect_stale_locks(dest, runtime, &[]) {
            detected_stale_locks = Some(unlockable_stale_lock_count(&lock_class));
        }
        if is_scheduled {
            transient_backoff_kind = Some(BackoffKind::Lock);
            status = RunStatus::Pending;
            run.pending_reason = Some(PendingReason::Backoff);
        }
    } else if !sleep_interrupted
        && is_scheduled
        && matches!(
            res.presentation.kind,
            ResticFailureKind::DestinationUnreachable | ResticFailureKind::DnsResolutionFailed
        )
    {
        transient_backoff_kind = Some(BackoffKind::Network);
        status = RunStatus::Pending;
        run.pending_reason = Some(PendingReason::DestinationUnreachable);
    } else if !sleep_interrupted && (backup_exit == 0 || backup_exit == 3) {
        detected_stale_locks = Some(0);
    }

    if !sleep_interrupted && !cancel.is_cancelled() && (backup_exit == 0 || backup_exit == 3) {
        let mut forget_out = PostForgetOutcome {
            status: &mut status,
            summary: &mut summary,
            primary_forget_state: &mut primary_forget_state,
            cleared_other_pending: &mut cleared_other_pending,
            detected_stale_locks: &mut detected_stale_locks,
        };
        run_post_backup_forget(
            backup,
            dest,
            runtime,
            pending_other_backups,
            run,
            &mut combined_logs,
            (cancel, backup_exit, secret_strs, &mut forget_out),
        );
    }

    let unreadable_items = unreadable_items_from_lines(combined_logs.iter().map(String::as_str));
    run.log_path = {
        let guard = shared_worker.lock().unwrap_or_else(|p| p.into_inner());
        guard
            .history
            .write_redacted_log(&run.id, &combined_logs, secret_strs)
            .map_err(|err| {
                eprintln!(
                    "kestic-daemon: failed to write redacted log for run {}: {err}",
                    run.id
                );
            })
            .ok()
    };
    run.summary_message = summary.clone();
    let (jv_code, jv_msg) = match status {
        RunStatus::Ok => (0, String::new()),
        _ => (1, summary.clone().unwrap_or_default()),
    };
    BackupPostExecOutcome {
        final_status: status,
        exit_code: Some(backup_exit),
        raw_detail: summary,
        jv_code,
        jv_msg,
        primary_forget_state,
        cleared_other_pending,
        detected_stale_locks,
        transient_backoff_kind,
        unreadable_items,
    }
}

fn process_backup_error_result(
    run: &mut Run,
    err: BackupRunError,
    sleep_interrupted: bool,
    is_scheduled: bool,
    secret_strs: &[&str],
) -> BackupPostExecOutcome {
    let mut transient_backoff_kind: Option<BackoffKind> = None;
    let (status, exit_code, msg) = if sleep_interrupted {
        (
            RunStatus::Interrupted,
            None,
            tr(&interrupted_by_sleep_message()),
        )
    } else if is_scheduled
        && matches!(
            err,
            BackupRunError::Destination(DestinationError::Unreachable(_))
        )
    {
        let localized = localized_backup_run_error(&err);
        let msg = redact_secrets(&localized, secret_strs);
        transient_backoff_kind = Some(BackoffKind::Network);
        run.pending_reason = Some(PendingReason::DestinationUnreachable);
        (RunStatus::Pending, None, msg)
    } else {
        let localized = localized_backup_run_error(&err);
        let msg = redact_secrets(&localized, secret_strs);
        // Pre-start availability failures keep `exit_code = None` (transient, `FR-29`/`FR-30`);
        // other destination errors carry their restic exit code so permanent errors (Exit 12,
        // Exit 10) block the schedule (`F2`).
        // The cause of a pre-start failure is kept in `pending_reason` (error kind for the GUI).
        run.pending_reason = match err {
            BackupRunError::SourceUnavailable(_) => Some(PendingReason::SourceUnavailable),
            BackupRunError::Destination(DestinationError::Unreachable(_)) => {
                Some(PendingReason::DestinationUnreachable)
            }
            _ => None,
        };
        let code = match err {
            BackupRunError::SourceUnavailable(_)
            | BackupRunError::Destination(DestinationError::Unreachable(_)) => None,
            BackupRunError::Destination(DestinationError::WrongPassword) => Some(12),
            BackupRunError::Destination(DestinationError::RepositoryMissing(_)) => Some(10),
            BackupRunError::Destination(DestinationError::CommandFailed { exit_code, .. }) => {
                Some(exit_code)
            }
            _ => Some(1),
        };
        (RunStatus::Error, code, msg)
    };
    run.summary_message = Some(msg.clone());
    BackupPostExecOutcome {
        final_status: status,
        exit_code,
        raw_detail: Some(msg.clone()),
        jv_code: 1,
        jv_msg: msg,
        primary_forget_state: None,
        cleared_other_pending: Vec::new(),
        detected_stale_locks: None,
        transient_backoff_kind,
        unreadable_items: Vec::new(),
    }
}

struct PostForgetOutcome<'a> {
    status: &'a mut RunStatus,
    summary: &'a mut Option<String>,
    primary_forget_state: &'a mut Option<bool>,
    cleared_other_pending: &'a mut Vec<BackupId>,
    detected_stale_locks: &'a mut Option<usize>,
}

fn run_post_backup_forget(
    backup: &Backup,
    dest: &Destination,
    runtime: &ResticRuntime<'_>,
    pending_other_backups: &[Backup],
    run: &mut Run,
    combined_logs: &mut Vec<String>,
    extra: (&CancelHandle, i32, &[&str], &mut PostForgetOutcome<'_>),
) {
    let (cancel, backup_exit, secret_strs, out) = extra;
    match execute_forget(backup, dest, runtime, Some(cancel), &[]) {
        Ok(f_res) => {
            run.commands.push(f_res.command_record.clone());
            combined_logs.extend(f_res.log_lines.iter().cloned());
            if f_res.exit_code == 0 {
                *out.primary_forget_state = Some(false);
                for other in pending_other_backups {
                    if cancel.is_cancelled() {
                        break;
                    }
                    if let Ok(other_res) = execute_forget(other, dest, runtime, Some(cancel), &[]) {
                        run.commands.push(other_res.command_record.clone());
                        combined_logs.extend(other_res.log_lines.iter().cloned());
                        if other_res.exit_code == 0 {
                            out.cleared_other_pending.push(other.id.clone());
                        }
                    }
                }
            } else {
                *out.primary_forget_state = Some(true);
                *out.status = RunStatus::Warning;
                if is_forget_lock_failure(&f_res)
                    && let Ok(lock_class) = detect_stale_locks(dest, runtime, &[])
                {
                    *out.detected_stale_locks = Some(unlockable_stale_lock_count(&lock_class));
                }
                if backup_exit == 0 {
                    *out.summary = Some(if is_forget_lock_failure(&f_res) {
                        i18n("Backed up, destination locked — retention will catch up later").into()
                    } else {
                        tr(&ErrorKind::RetentionPending.message(None, false))
                    });
                }
            }
        }
        Err(f_err) => {
            *out.primary_forget_state = Some(true);
            *out.status = RunStatus::Warning;
            if backup_exit == 0 {
                *out.summary = Some(tr(&ErrorKind::RetentionPending.message(None, false)));
            }
            combined_logs.push(redact_secrets(&f_err.to_string(), secret_strs));
        }
    }
}

fn finalize_backup_run_state(
    shared_worker: &Arc<Mutex<DaemonCore>>,
    ctx: &BackupClaimContext,
    run: Run,
    outcome: BackupPostExecOutcome,
    secret_strs: &[&str],
    logind_sleep_guard: Option<&mut LogindSleepInhibitor>,
    handles: (Option<&Connection>, Option<&Arc<TrayHandle>>),
) {
    let (conn_opt, tray_opt) = handles;
    let run_id = run.id.clone();
    let (state_dto, notif_settings) = {
        let mut guard = shared_worker.lock().unwrap_or_else(|p| p.into_inner());
        guard.active_runs.remove(&run_id);
        if let Some(stale_count) = outcome.detected_stale_locks {
            guard
                .dest_stale_locks
                .insert(ctx.dest.id.clone(), stale_count);
        }
        if let Some(pending) = outcome.primary_forget_state
            && let Err(err) = guard.set_pending_forget_cached(&ctx.backup.id, pending)
        {
            eprintln!(
                "kestic-daemon: failed to update pending_forget for {}: {err}",
                ctx.backup.id
            );
        }
        for other_id in &outcome.cleared_other_pending {
            if let Err(err) = guard.set_pending_forget_cached(other_id, false) {
                eprintln!("kestic-daemon: failed to clear pending_forget for {other_id}: {err}");
            }
        }

        let finish_now = ctx.clock.now_utc();
        if let Some(kind) = outcome.transient_backoff_kind {
            if kind == BackoffKind::Network {
                guard.dest_availability.insert(
                    ctx.dest.id.clone(),
                    DestinationAvailabilityDto {
                        destination_id: ctx.dest.id.as_str().to_string(),
                        reachable: false,
                        detail: run.summary_message.clone(),
                        checked_at_unix: finish_now.unix_timestamp(),
                    },
                );
            }
            let mut rs = guard.load_backup_runtime_state(&ctx.backup.id);
            record_backup_backoff_failure(&mut rs, finish_now, Some(kind));
            let _ = guard.store_backup_runtime_state(&ctx.backup.id, &rs);
        } else if matches!(outcome.final_status, RunStatus::Ok | RunStatus::Warning) {
            let mut rs = guard.load_backup_runtime_state(&ctx.backup.id);
            rs.backoff_attempt = 0;
            rs.backoff_until = None;
            rs.backoff_kind = None;
            rs.last_failure_at = None;
            rs.last_overdue_notified_at = None;
            rs.overdue_anchor_at = Some(finish_now);
            let _ = guard.store_backup_runtime_state(&ctx.backup.id, &rs);
        } else if (outcome.final_status == RunStatus::Error
            && outcome.exit_code.is_some()
            && !run_blocks_schedule(&run))
            || (outcome.final_status == RunStatus::Cancelled
                && !guard.shutdown_flag.load(Ordering::SeqCst))
        {
            // A restic error that neither is transient nor blocks the schedule (`F2`: alert, no
            // retry) or a user cancel handles the due slot: no restart before the next regular
            // slot. Permanent errors block anyway; pre-start failures (no exit code: source or
            // destination unavailable) stay retryable (`FR-29`, `FR-30`).
            let mut rs = guard.load_backup_runtime_state(&ctx.backup.id);
            mark_backup_slot_handled(&mut rs, finish_now);
            let _ = guard.store_backup_runtime_state(&ctx.backup.id, &rs);
        }

        guard
            .run_unreadable_cache
            .insert(run_id.clone(), outcome.unreadable_items.clone());
        if let Err(err) = guard.upsert_run_cached(&run, secret_strs) {
            eprintln!("kestic-daemon: failed to record finished run {run_id} in history: {err}");
        }
        if let Err(err) = guard
            .history
            .prune_logs_for_backup(&ctx.backup.id, ctx.clock.as_ref())
        {
            eprintln!(
                "kestic-daemon: failed to prune old logs for backup {}: {err}",
                ctx.backup.id
            );
        }
        let (state_dto, tray_state) = guard.build_state_and_tray();
        if let Some(tray) = tray_opt {
            tray.set_full_state(tray_state);
        }
        (state_dto, guard.config.notifications.clone())
    };

    if let Some(inh) = logind_sleep_guard {
        inh.release();
    }

    match outcome.final_status {
        RunStatus::Error => {
            let summary = i18n("%1: Backup failed").subs(&[&ctx.backup.name]);
            let body = outcome
                .raw_detail
                .unwrap_or_else(|| localized_failure_summary(&ResticFailureKind::GenericFailure));
            let _ = send_configured_notification(
                &notif_settings,
                KesticEvent::BackupFailed,
                &summary,
                &body,
            );
        }
        RunStatus::Warning => {
            let summary = i18n("%1: Backup finished with warning").subs(&[&ctx.backup.name]);
            let body = outcome.raw_detail.unwrap_or_else(|| {
                localized_failure_summary(&ResticFailureKind::IncompleteSnapshot {
                    unreadable_items: Vec::new(),
                })
            });
            let _ = send_configured_notification(
                &notif_settings,
                KesticEvent::BackupWarning,
                &summary,
                &body,
            );
        }
        _ => {}
    }

    if let Some(conn) = conn_opt {
        emit_run_updated(conn, &run);
        emit_status_changed(conn, &state_dto);
    }

    drain_queued_backups_async(shared_worker);
}
