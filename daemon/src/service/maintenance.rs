// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Destination maintenance (`pending_forget`, `prune`, `check`) and `StartAllBackups` orchestration.

use super::availability::{
    DestinationClaimGuard, check_source_status_bounded, drain_queued_backups_async,
    unlockable_stale_lock_count,
};
use super::backup_runner::{load_dest_secrets, start_backup_async, upsert_pending_run_entry};
use super::error::{
    KesticServiceErr, localized_source_check_reason, manual_run_blocked_reason, service_error_kind,
};
use super::run_errors::run_blocks_schedule;
use super::state::{ActiveRunEntry, DaemonCore, probe_before_run};
use crate::dbus_api::{
    ActiveRunProgressDto, DestinationAvailabilityDto, SkippedBackupDto, StartAllResultDto,
};
use crate::dbus_server::{emit_run_updated, emit_status_changed};
use crate::i18n::{i18n, tr, tr_text};
use crate::inhibit::PortalInhibitGuard;
use crate::job_view::{CAPABILITY_KILLABLE, JobViewHandle};
use crate::monitors::{LogindSleepInhibitor, MonitorBusConfig};
use crate::notifications::{KESTIC_DESKTOP_ENTRY, KesticEvent, send_configured_notification};
use crate::tray::TrayHandle;
use kestic_core::backup::{execute_forget, is_forget_lock_failure};
use kestic_core::clock::Clock;
use kestic_core::destination::{DEFAULT_CONNECT_TIMEOUT, preflight_check_destination};
use kestic_core::maintenance::{detect_stale_locks, execute_check, execute_prune};
use kestic_core::model::{
    Backup, BackupId, Destination, DestinationId, PendingReason, ResticProvider, Run, RunStatus,
};
use kestic_core::mount::{MountInspector, SourceCheckResult, current_home_dir};
use kestic_core::provider::ResolvedRestic;
use kestic_core::restic::{CancelHandle, ResticRuntime, redact_secrets};
use kestic_core::scheduler::{
    evaluate_run_conditions, mark_maintenance_slot_handled, record_maintenance_backoff_failure,
};
use kestic_core::secrets::SecretStore;
use kestic_i18n::codes::{ErrorKind, PendingReasonCode, SkipReason};
use kestic_i18n::messages::maintaining_message;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::thread;
use time::OffsetDateTime;
use zbus::Connection;

/// Runs catch-up retention (`forget`) for all backups on `destination_id` marked `pending_forget` (`AC-M2-07-4`).
pub fn run_destination_maintenance_async(
    shared: &Arc<Mutex<DaemonCore>>,
    destination_id: &str,
) -> Result<String, KesticServiceErr> {
    run_destination_maintenance_internal(shared, destination_id, false, false, false)
}

/// Runs destination maintenance (`pending_forget` -> optional `prune` -> optional `check`, `M3-S05`).
pub fn run_destination_maintenance_with_options(
    shared: &Arc<Mutex<DaemonCore>>,
    destination_id: &str,
    run_prune: bool,
    run_check: bool,
) -> Result<String, KesticServiceErr> {
    run_destination_maintenance_internal(shared, destination_id, run_prune, run_check, false)
}

pub(crate) fn run_scheduled_maintenance_async(
    shared: &Arc<Mutex<DaemonCore>>,
    destination_id: &str,
    run_prune: bool,
    run_check: bool,
) -> Result<String, KesticServiceErr> {
    run_destination_maintenance_internal(shared, destination_id, run_prune, run_check, true)
}

struct MaintenanceClaimContext {
    dest: Destination,
    provider: ResticProvider,
    kestic_binary: PathBuf,
    secret_store: Arc<dyn SecretStore + Send + Sync>,
    clock: Arc<dyn Clock>,
    enable_job_view: bool,
    job_view_dest: String,
    enable_portal_inhibit: bool,
    monitor_bus: MonitorBusConfig,
    enable_system_monitors: bool,
    claim_guard: DestinationClaimGuard,
}

fn claim_maintenance_destination(
    shared: &Arc<Mutex<DaemonCore>>,
    d_id: &DestinationId,
    destination_id: &str,
    is_scheduled: bool,
) -> Result<MaintenanceClaimContext, KesticServiceErr> {
    let mut guard = shared.lock().unwrap_or_else(|p| p.into_inner());
    let dest = guard
        .config
        .destination(d_id)
        .cloned()
        .ok_or_else(|| KesticServiceErr::NotFound(destination_id.to_string()))?;

    if is_scheduled && guard.sleep_in_progress {
        return Err(KesticServiceErr::SleepInProgress);
    }
    if guard.is_destination_busy(&dest.id) {
        return Err(KesticServiceErr::AlreadyRunning(format!(
            "another run is already active on destination '{}'",
            dest.name
        )));
    }

    guard.claiming_destinations.insert(d_id.clone());
    let claim_guard = DestinationClaimGuard::new(Arc::clone(shared), d_id.clone());
    Ok(MaintenanceClaimContext {
        dest,
        provider: guard.config.restic_provider.clone(),
        kestic_binary: guard.kestic_binary.clone(),
        secret_store: Arc::clone(&guard.secret_store),
        clock: Arc::clone(&guard.clock),
        enable_job_view: guard.enable_job_view,
        job_view_dest: guard.job_view_dest.clone(),
        enable_portal_inhibit: guard.enable_portal_inhibit,
        monitor_bus: guard.monitor_bus.clone(),
        enable_system_monitors: guard.enable_system_monitors,
        claim_guard,
    })
}

fn handle_maintenance_preflight_failure(
    guard: &mut DaemonCore,
    claim_guard: &mut DestinationClaimGuard,
    d_id: &DestinationId,
    now: OffsetDateTime,
    is_scheduled: bool,
    err_msg: String,
) -> KesticServiceErr {
    if is_scheduled {
        let mut ds = guard.load_destination_maintenance_state(d_id);
        record_maintenance_backoff_failure(&mut ds, now);
        let _ = guard.store_destination_maintenance_state(d_id, &ds);
    }
    guard.claiming_destinations.remove(d_id);
    claim_guard.disarm();
    let conn_opt = guard.conn.clone();
    let (state_dto, tray_state) = guard.build_state_and_tray();
    if let Some(tray) = &guard.tray {
        tray.set_full_state(tray_state);
    }
    if let Some(conn) = &conn_opt {
        emit_status_changed(conn, &state_dto);
    }
    KesticServiceErr::DestinationUnreachable(err_msg)
}

struct LaunchedMaintenanceRun {
    pending_backups: Vec<Backup>,
    run: Run,
    cancel: CancelHandle,
    conn_opt: Option<Connection>,
    tray_opt: Option<Arc<TrayHandle>>,
}

fn launch_maintenance_run_record(
    guard: &mut DaemonCore,
    claim_guard: &mut DestinationClaimGuard,
    d_id: &DestinationId,
    dest: &Destination,
) -> Result<LaunchedMaintenanceRun, KesticServiceErr> {
    let pending_backups: Vec<Backup> = guard
        .config
        .backups
        .iter()
        .filter(|b| &b.destination_id == d_id && guard.is_pending_forget_cached(&b.id))
        .cloned()
        .collect();

    let run = Run::new_maintenance(guard.clock.as_ref(), d_id.clone())
        .map_err(|e| KesticServiceErr::Internal(e.to_string()))?;
    guard
        .upsert_run_cached(&run, &[])
        .map_err(|e| KesticServiceErr::Internal(e.to_string()))?;

    let cancel = CancelHandle::new();
    let entry = ActiveRunEntry {
        run: run.clone(),
        backup_id: None,
        backup_name: dest.name.clone(),
        progress: ActiveRunProgressDto {
            run_id: run.id.as_str().to_string(),
            backup_id: None,
            backup_name: dest.name.clone(),
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
    guard.claiming_destinations.remove(d_id);
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
    Ok(LaunchedMaintenanceRun {
        pending_backups,
        run,
        cancel,
        conn_opt,
        tray_opt,
    })
}

fn run_destination_maintenance_internal(
    shared: &Arc<Mutex<DaemonCore>>,
    destination_id: &str,
    run_prune: bool,
    run_check: bool,
    is_scheduled: bool,
) -> Result<String, KesticServiceErr> {
    let d_id = DestinationId::parse(destination_id)
        .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;

    let mut ctx = claim_maintenance_destination(shared, &d_id, destination_id, is_scheduled)?;

    let (probe, resolved) = probe_before_run(
        shared,
        &ctx.provider,
        ctx.secret_store.as_ref(),
        &d_id,
        &mut ctx.claim_guard,
    )?;
    let preflight_res = preflight_check_destination(&ctx.dest, DEFAULT_CONNECT_TIMEOUT);

    let launched = {
        let mut guard = shared.lock().unwrap_or_else(|p| p.into_inner());
        guard.record_probe(probe);
        if is_scheduled && guard.sleep_in_progress {
            guard.claiming_destinations.remove(&d_id);
            ctx.claim_guard.disarm();
            return Err(KesticServiceErr::SleepInProgress);
        }
        let now = guard.clock.now_utc();
        guard.availability_probe_count += 1;
        let dto = match &preflight_res {
            Ok(()) => DestinationAvailabilityDto {
                destination_id: d_id.as_str().to_string(),
                reachable: true,
                detail: None,
                checked_at_unix: now.unix_timestamp(),
            },
            Err(e) => DestinationAvailabilityDto {
                destination_id: d_id.as_str().to_string(),
                reachable: false,
                detail: Some(e.to_string()),
                checked_at_unix: now.unix_timestamp(),
            },
        };
        guard.dest_availability.insert(d_id.clone(), dto);

        if let Err(err) = preflight_res {
            return Err(handle_maintenance_preflight_failure(
                &mut guard,
                &mut ctx.claim_guard,
                &d_id,
                now,
                is_scheduled,
                err.to_string(),
            ));
        }

        launch_maintenance_run_record(&mut guard, &mut ctx.claim_guard, &d_id, &ctx.dest)?
    };

    let run_id_str = launched.run.id.as_str().to_string();
    let shared_worker = Arc::clone(shared);
    thread::spawn(move || {
        execute_maintenance_worker(MaintenanceWorkerArgs {
            shared_worker,
            ctx,
            resolved,
            pending_backups: launched.pending_backups,
            run: launched.run,
            cancel: launched.cancel,
            run_prune,
            run_check,
            is_scheduled,
            conn_opt: launched.conn_opt,
            tray_opt: launched.tray_opt,
        });
    });

    Ok(run_id_str)
}

struct MaintenanceWorkerArgs {
    shared_worker: Arc<Mutex<DaemonCore>>,
    ctx: MaintenanceClaimContext,
    resolved: ResolvedRestic,
    pending_backups: Vec<Backup>,
    run: Run,
    cancel: CancelHandle,
    run_prune: bool,
    run_check: bool,
    is_scheduled: bool,
    conn_opt: Option<Connection>,
    tray_opt: Option<Arc<TrayHandle>>,
}

struct MaintenanceOpOutcome {
    cleared_ids: Vec<BackupId>,
    forget_failed: bool,
    prune_or_check_failed: bool,
    prune_succeeded_at: Option<OffsetDateTime>,
    check_succeeded_at: Option<OffsetDateTime>,
    detected_stale_locks: Option<usize>,
    last_exit: i32,
    combined_logs: Vec<String>,
    failure_detail: Option<String>,
}

fn execute_maintenance_worker(args: MaintenanceWorkerArgs) {
    let MaintenanceWorkerArgs {
        shared_worker,
        ctx,
        resolved,
        pending_backups,
        mut run,
        cancel,
        run_prune,
        run_check,
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
        PortalInhibitGuard::acquire_default(&format!("Kestic maintenance: {}", ctx.dest.name)).ok()
    } else {
        None
    };
    let job_view = if ctx.enable_job_view {
        JobViewHandle::request(
            &ctx.job_view_dest,
            KESTIC_DESKTOP_ENTRY,
            &tr(&maintaining_message(&ctx.dest.name)),
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
    let runtime = ResticRuntime {
        resolved: &resolved,
        kestic_binary: &ctx.kestic_binary,
        secret_store: ctx.secret_store.as_ref(),
        clock: ctx.clock.as_ref(),
        cache_dir: None,
    };

    let secrets_owned = load_dest_secrets(&ctx.dest, ctx.secret_store.as_ref());
    let secret_strs: Vec<&str> = secrets_owned
        .iter()
        .filter_map(|s| s.as_str().ok())
        .collect();

    let mut ops = run_maintenance_operations(
        MaintenanceOpsInput {
            dest: &ctx.dest,
            runtime: &runtime,
            pending_backups: &pending_backups,
            cancel: &cancel,
            clock: ctx.clock.as_ref(),
            secret_strs: &secret_strs,
            run_prune,
            run_check,
        },
        &mut run,
    );
    drop(job_view_cancel_guard);

    let sleep_interrupted = {
        let guard = shared_worker.lock().unwrap_or_else(|p| p.into_inner());
        guard
            .active_runs
            .get(&run_id)
            .is_some_and(|e| e.sleep_interrupted)
    };

    run.log_path = {
        let guard = shared_worker.lock().unwrap_or_else(|p| p.into_inner());
        match guard
            .history
            .write_redacted_log(&run_id, &ops.combined_logs, &secret_strs)
        {
            Ok(path) => Some(path),
            Err(err) => {
                eprintln!("kestic-daemon: failed to write maintenance log for run {run_id}: {err}");
                None
            }
        }
    };

    let status = if sleep_interrupted {
        RunStatus::Interrupted
    } else if cancel.is_cancelled() {
        RunStatus::Cancelled
    } else if ops.prune_or_check_failed {
        RunStatus::Error
    } else if ops.forget_failed {
        RunStatus::Warning
    } else {
        RunStatus::Ok
    };
    if run.summary_message.is_none() {
        run.summary_message = ops.failure_detail.take();
    }
    run.finish(ctx.clock.as_ref(), status, Some(ops.last_exit));

    if let Some(jv) = job_view {
        let jv_code = if status == RunStatus::Ok { 0 } else { 1 };
        let _ = jv.terminate(jv_code, run.summary_message.as_deref().unwrap_or(""));
    }

    finalize_maintenance_run_state(MaintenanceFinalizeInput {
        shared_worker: &shared_worker,
        ctx: &ctx,
        run,
        ops: &ops,
        status,
        is_scheduled,
        secret_strs: &secret_strs,
        logind_sleep_guard: logind_sleep_guard.as_mut(),
        conn_opt: conn_opt.as_ref(),
        tray_opt: tray_opt.as_ref(),
    });
}

struct MaintenanceOpsInput<'a> {
    dest: &'a Destination,
    runtime: &'a ResticRuntime<'a>,
    pending_backups: &'a [Backup],
    cancel: &'a CancelHandle,
    clock: &'a dyn Clock,
    secret_strs: &'a [&'a str],
    run_prune: bool,
    run_check: bool,
}

fn run_maintenance_operations(
    input: MaintenanceOpsInput<'_>,
    run: &mut Run,
) -> MaintenanceOpOutcome {
    let MaintenanceOpsInput {
        dest,
        runtime,
        pending_backups,
        cancel,
        clock: _,
        secret_strs,
        run_prune,
        run_check,
    } = input;
    let mut out = MaintenanceOpOutcome {
        cleared_ids: Vec::new(),
        forget_failed: false,
        prune_or_check_failed: false,
        prune_succeeded_at: None,
        check_succeeded_at: None,
        detected_stale_locks: None,
        last_exit: 0,
        combined_logs: Vec::new(),
        failure_detail: None,
    };

    for b in pending_backups {
        if cancel.is_cancelled() {
            break;
        }
        match execute_forget(b, dest, runtime, Some(cancel), &[]) {
            Ok(res) => {
                run.commands.push(res.command_record.clone());
                out.combined_logs.extend(res.log_lines.iter().cloned());
                out.last_exit = res.exit_code;
                if res.exit_code == 0 {
                    out.cleared_ids.push(b.id.clone());
                } else {
                    out.forget_failed = true;
                    if is_forget_lock_failure(&res)
                        && let Ok(lock_class) = detect_stale_locks(dest, runtime, &[])
                    {
                        out.detected_stale_locks = Some(unlockable_stale_lock_count(&lock_class));
                    }
                }
            }
            Err(err) => {
                out.forget_failed = true;
                out.last_exit = 1;
                let msg = redact_secrets(&err.to_string(), secret_strs);
                out.failure_detail = Some(msg.clone());
                out.combined_logs.push(msg);
            }
        }
    }

    if run_prune && !cancel.is_cancelled() {
        run_single_maintenance_step(
            execute_prune(dest, runtime, Some(cancel), &[]),
            &input,
            run,
            &mut out,
            true,
        );
    }

    if run_check && !cancel.is_cancelled() {
        run_single_maintenance_step(
            execute_check(dest, runtime, Some(cancel), &[]),
            &input,
            run,
            &mut out,
            false,
        );
    }

    out
}

fn run_single_maintenance_step(
    res: Result<kestic_core::restic::ResticExecutionResult, kestic_core::backup::BackupRunError>,
    input: &MaintenanceOpsInput<'_>,
    run: &mut Run,
    out: &mut MaintenanceOpOutcome,
    is_prune: bool,
) {
    match res {
        Ok(step_res) => {
            run.commands.push(step_res.command_record.clone());
            out.combined_logs.extend(step_res.log_lines.iter().cloned());
            out.last_exit = step_res.exit_code;
            if step_res.exit_code == 0 {
                let ts = Some(input.clock.now_utc());
                if is_prune {
                    out.prune_succeeded_at = ts;
                } else {
                    out.check_succeeded_at = ts;
                }
            } else if !input.cancel.is_cancelled() {
                out.prune_or_check_failed = true;
                let detail = redact_secrets(
                    &step_res.presentation.detail_or_summary(),
                    input.secret_strs,
                );
                out.failure_detail = Some(detail);
                if step_res.exit_code == 11
                    && let Ok(lock_class) = detect_stale_locks(input.dest, input.runtime, &[])
                {
                    out.detected_stale_locks = Some(unlockable_stale_lock_count(&lock_class));
                }
            }
        }
        Err(err) => {
            if !input.cancel.is_cancelled() {
                out.prune_or_check_failed = true;
                out.last_exit = 1;
                let msg = redact_secrets(&err.to_string(), input.secret_strs);
                out.failure_detail = Some(msg.clone());
                out.combined_logs.push(msg);
            }
        }
    }
}

struct MaintenanceFinalizeInput<'a> {
    shared_worker: &'a Arc<Mutex<DaemonCore>>,
    ctx: &'a MaintenanceClaimContext,
    run: Run,
    ops: &'a MaintenanceOpOutcome,
    status: RunStatus,
    is_scheduled: bool,
    secret_strs: &'a [&'a str],
    logind_sleep_guard: Option<&'a mut LogindSleepInhibitor>,
    conn_opt: Option<&'a Connection>,
    tray_opt: Option<&'a Arc<TrayHandle>>,
}

fn finalize_maintenance_run_state(input: MaintenanceFinalizeInput<'_>) {
    let MaintenanceFinalizeInput {
        shared_worker,
        ctx,
        run,
        ops,
        status,
        is_scheduled,
        secret_strs,
        logind_sleep_guard,
        conn_opt,
        tray_opt,
    } = input;
    let run_id = run.id.clone();
    let (state_dto, notif_settings, maint_backoff_attempt) = {
        let mut guard = shared_worker.lock().unwrap_or_else(|p| p.into_inner());
        guard.active_runs.remove(&run_id);
        if let Some(stale_count) = ops.detected_stale_locks {
            guard
                .dest_stale_locks
                .insert(ctx.dest.id.clone(), stale_count);
        }
        for id in &ops.cleared_ids {
            if let Err(err) = guard.set_pending_forget_cached(id, false) {
                eprintln!(
                    "kestic-daemon: failed to clear pending_forget for {id} in maintenance: {err}"
                );
            }
        }
        let recorded_attempt = update_maintenance_state_after_run(&mut guard, ctx, ops, status);
        if let Err(err) = guard.upsert_run_cached(&run, secret_strs) {
            eprintln!("kestic-daemon: failed to record maintenance run {run_id} in history: {err}");
        }
        let (state_dto, tray_state) = guard.build_state_and_tray();
        if let Some(tray) = tray_opt {
            tray.set_full_state(tray_state);
        }
        (
            state_dto,
            guard.config.notifications.clone(),
            recorded_attempt,
        )
    };

    if let Some(inh) = logind_sleep_guard {
        inh.release();
    }

    if status == RunStatus::Error && (!is_scheduled || maint_backoff_attempt == 1) {
        let summary = i18n("%1: Maintenance failed").subs(&[&ctx.dest.name]);
        let body = run
            .summary_message
            .clone()
            .unwrap_or_else(|| i18n("Maintenance failed").to_string());
        let _ = send_configured_notification(
            &notif_settings,
            KesticEvent::MaintenanceFailed,
            &summary,
            &body,
        );
    }

    if let Some(conn) = conn_opt {
        emit_run_updated(conn, &run);
        emit_status_changed(conn, &state_dto);
    }

    drain_queued_backups_async(shared_worker);
}

/// Updates prune/check timestamps and the maintenance backoff after a finished run; returns the
/// recorded backoff attempt (0 if none).
///
/// A failed `forget` counts as a failed attempt even though the run ends `Warning` (F2/F4):
/// `pending_forget` stays set, so without backoff the scheduler would re-run it every tick.
/// A user cancel marks the due work as handled until the next regular prune/check slot.
fn update_maintenance_state_after_run(
    guard: &mut DaemonCore,
    ctx: &MaintenanceClaimContext,
    ops: &MaintenanceOpOutcome,
    status: RunStatus,
) -> u32 {
    let mut ds = guard.load_destination_maintenance_state(&ctx.dest.id);
    if let Some(ts) = ops.prune_succeeded_at {
        ds.last_prune_at = Some(ts);
    }
    if let Some(ts) = ops.check_succeeded_at {
        ds.last_check_at = Some(ts);
    }
    let finish_now = ctx.clock.now_utc();
    let mut recorded_attempt = 0u32;
    let failed_attempt =
        status == RunStatus::Error || (status == RunStatus::Warning && ops.forget_failed);
    if failed_attempt {
        record_maintenance_backoff_failure(&mut ds, finish_now);
        recorded_attempt = ds.maintenance_backoff_attempt;
    } else if matches!(status, RunStatus::Ok | RunStatus::Warning) {
        ds.maintenance_backoff_attempt = 0;
        ds.maintenance_backoff_until = None;
        ds.last_maintenance_failure_at = None;
    } else if status == RunStatus::Cancelled && !guard.shutdown_flag.load(Ordering::SeqCst) {
        mark_maintenance_slot_handled(&mut ds, finish_now);
    }
    let _ = guard.store_destination_maintenance_state(&ctx.dest.id, &ds);
    recorded_attempt
}

struct StartAllCandidate {
    backup: Backup,
    has_error: bool,
    error_kind: Option<ErrorKind>,
    /// Daemon-language cause of the permanent error (run history only).
    error_text: Option<String>,
    mount_inspector: Arc<dyn MountInspector>,
    /// Daemon-language «Condition not met: …» while run conditions are unmet.
    unmet_text: Option<String>,
}

struct StartAllEntry {
    backup: Backup,
    has_error: bool,
    error_kind: Option<ErrorKind>,
    error_text: Option<String>,
    source_status: SourceCheckResult,
    unmet_text: Option<String>,
}

struct StartAllDispatchOutcome {
    started_run_ids: Vec<String>,
    queued_backup_ids: Vec<String>,
    queued_entries: Vec<Backup>,
    skipped: Vec<SkippedBackupDto>,
    skipped_history_entries: Vec<(Backup, SkipReason, String)>,
}

pub fn start_all_backups_async(shared: &Arc<Mutex<DaemonCore>>) -> StartAllResultDto {
    let entries = collect_start_all_entries(shared);
    let outcome = dispatch_start_all_entries(shared, entries);
    record_start_all_pending_and_skipped(
        shared,
        outcome.queued_entries,
        outcome.skipped_history_entries,
    );

    if !outcome.queued_backup_ids.is_empty() {
        let shared_drain = Arc::clone(shared);
        thread::spawn(move || {
            drain_queued_backups_async(&shared_drain);
        });
    }

    StartAllResultDto {
        started_run_ids: outcome.started_run_ids,
        skipped: outcome.skipped,
        queued_backup_ids: outcome.queued_backup_ids,
    }
}

fn collect_start_all_entries(shared: &Arc<Mutex<DaemonCore>>) -> Vec<StartAllEntry> {
    let candidates: Vec<StartAllCandidate> = {
        let guard = shared.lock().unwrap_or_else(|p| p.into_inner());
        let (state_dto, _) = guard.build_state_and_tray();
        state_dto
            .backups
            .into_iter()
            .map(|s| {
                let unmet = evaluate_run_conditions(&s.backup.run_conditions, &guard.system_env);
                let unmet_text =
                    (!unmet.is_empty()).then(|| guard.localized_unmet_conditions(&unmet));
                let has_error = guard
                    .runs_cache
                    .iter()
                    .find(|r| {
                        r.backup_id() == Some(&s.backup.id)
                            && !matches!(r.status, RunStatus::Pending | RunStatus::Running)
                    })
                    .is_some_and(run_blocks_schedule);
                StartAllCandidate {
                    backup: s.backup,
                    has_error,
                    error_text: s
                        .error_kind
                        .map(|k| tr(&k.message(s.error_exit_code, false))),
                    error_kind: s.error_kind,
                    mount_inspector: Arc::clone(&guard.mount_inspector),
                    unmet_text,
                }
            })
            .collect()
    };

    let home = current_home_dir();
    candidates
        .into_iter()
        .map(|c| {
            let source_status =
                check_source_status_bounded(&c.backup, home.as_deref(), &c.mount_inspector);
            StartAllEntry {
                backup: c.backup,
                has_error: c.has_error,
                error_kind: c.error_kind,
                error_text: c.error_text,
                source_status,
                unmet_text: c.unmet_text,
            }
        })
        .collect()
}

fn dispatch_start_all_entries(
    shared: &Arc<Mutex<DaemonCore>>,
    entries: Vec<StartAllEntry>,
) -> StartAllDispatchOutcome {
    let mut started_run_ids = Vec::new();
    let mut queued_backup_ids = Vec::new();
    let mut queued_entries: Vec<Backup> = Vec::new();
    let mut skipped = Vec::new();
    let mut skipped_history_entries: Vec<(Backup, SkipReason, String)> = Vec::new();
    let mut skip = |backup: Backup, skip: SkippedBackupDto, text: String| {
        skipped_history_entries.push((backup, skip.reason, text));
        skipped.push(skip);
    };

    for StartAllEntry {
        backup,
        has_error,
        error_kind,
        error_text,
        source_status,
        unmet_text,
    } in entries
    {
        let id = backup.id.as_str().to_string();
        let dto = |reason, error_kind, error_detail| SkippedBackupDto {
            backup_id: id.clone(),
            backup_name: backup.name.clone(),
            reason,
            error_kind,
            error_detail,
        };
        if has_error {
            let text =
                error_text.unwrap_or_else(|| tr_text(&SkipReason::PermanentError.text(None, None)));
            let skipped_dto = dto(SkipReason::PermanentError, error_kind, None);
            skip(backup, skipped_dto, text);
            continue;
        }
        if !source_status.is_ok() {
            let skipped_dto = dto(SkipReason::SourceUnavailable, None, None);
            skip(
                backup,
                skipped_dto,
                localized_source_check_reason(&source_status),
            );
            continue;
        }
        if let Some(text) = unmet_text {
            let skipped_dto = dto(SkipReason::ConditionNotMet, None, None);
            skip(backup, skipped_dto, text);
            continue;
        }
        match start_backup_async(shared, &id) {
            Ok(run_id) => started_run_ids.push(run_id),
            Err(KesticServiceErr::AlreadyRunning(_)) => {
                queued_backup_ids.push(id);
                queued_entries.push(backup);
            }
            Err(err) => {
                let (skipped_dto, text) = match manual_run_blocked_reason(&err) {
                    Some(reason) => (
                        dto(SkipReason::from(reason), None, None),
                        tr(&reason.message(None)),
                    ),
                    None => (
                        dto(
                            SkipReason::Error,
                            Some(service_error_kind(&err)),
                            Some(err.to_string()),
                        ),
                        err.to_string(),
                    ),
                };
                skip(backup, skipped_dto, text);
            }
        }
    }

    StartAllDispatchOutcome {
        started_run_ids,
        queued_backup_ids,
        queued_entries,
        skipped,
        skipped_history_entries,
    }
}

fn record_start_all_pending_and_skipped(
    shared: &Arc<Mutex<DaemonCore>>,
    queued_entries: Vec<Backup>,
    skipped_history_entries: Vec<(Backup, SkipReason, String)>,
) {
    if skipped_history_entries.is_empty() && queued_entries.is_empty() {
        return;
    }
    let (conn_opt, state_dto) = {
        let mut guard = shared.lock().unwrap_or_else(|p| p.into_inner());
        for backup in queued_entries {
            if !guard.queued_backups.contains(&backup.id) {
                guard.queued_backups.push_back(backup.id.clone());
            }
            let msg = tr(&PendingReasonCode::Queue.waiting_message(""));
            upsert_pending_run_entry(&mut guard, &backup, PendingReason::Queued, msg);
        }
        for (backup, reason, reason_text) in skipped_history_entries {
            let note = i18n("Skipped by 'Back Up All Now': %1").subs(&[&reason_text]);
            match reason {
                SkipReason::SourceUnavailable => upsert_pending_run_entry(
                    &mut guard,
                    &backup,
                    PendingReason::SourceUnavailable,
                    note,
                ),
                SkipReason::DestinationUnreachable => upsert_pending_run_entry(
                    &mut guard,
                    &backup,
                    PendingReason::DestinationUnreachable,
                    note,
                ),
                SkipReason::ConditionNotMet => upsert_pending_run_entry(
                    &mut guard,
                    &backup,
                    PendingReason::ConditionNotMet,
                    note,
                ),
                SkipReason::PermanentError | SkipReason::Error => {
                    if let Ok(mut meta_run) =
                        Run::new_meta(guard.clock.as_ref(), backup.destination_id.clone())
                    {
                        meta_run.summary_message =
                            Some(i18n("%1: %2").subs(&[&backup.name, &note]));
                        meta_run.finish(guard.clock.as_ref(), RunStatus::Warning, None);
                        let _ = guard.upsert_run_cached(&meta_run, &[]);
                    }
                }
            }
        }
        let (state_dto, tray_state) = guard.build_state_and_tray();
        if let Some(tray) = &guard.tray {
            tray.set_full_state(tray_state);
        }
        (guard.conn.clone(), state_dto)
    };
    if let Some(conn) = &conn_opt {
        emit_status_changed(conn, &state_dto);
    }
}
