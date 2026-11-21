// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Asynchronous backup execution, preflight checks, progress reporting, forget catch-up, and run finalization.

use super::availability::{
    DestinationClaimGuard, check_source_status_bounded, drain_queued_backups_async,
    unlockable_stale_lock_count,
};
use super::error::parse_backup_id;
use super::error::{
    KesticServiceErr, localized_backup_run_error, localized_failure_summary,
    localized_source_check_reason,
};
use super::run_errors::{run_blocks_schedule, unreadable_items_from_lines};
use super::state::{DaemonCore, ResticDeps, probe_before_run};
use super::state_summary::{ActivatedRun, NewActiveRun};
use super::supervision::{RunSupervision, SupervisionConfig};
use crate::dbus_api::DestinationAvailabilityDto;
use crate::dbus_server::{
    emit_manual_run_blocked, emit_run_progress, emit_run_updated, emit_status_changed,
};
use crate::i18n::{i18n, tr};
use crate::job_view::{JobProgress, JobViewHandle};
use crate::monitors::LogindSleepInhibitor;
use crate::notifications::{KesticEvent, send_configured_notification, send_event_notification};
use crate::tray::TrayHandle;
use kestic_core::backup::{
    BackupExecutionContext, BackupRunError, execute_backup_with_inspector, execute_forget,
    is_forget_lock_failure,
};
use kestic_core::backup_progress::BackupProgressTracker;
use kestic_core::clock::Clock;
use kestic_core::destination::{
    DEFAULT_CONNECT_TIMEOUT, DestinationError, preflight_check_destination,
};
use kestic_core::history::BackoffKind;
use kestic_core::maintenance::detect_stale_locks;
use kestic_core::model::{
    Backup, BackupId, Destination, DestinationId, PendingReason, Run, RunId, RunKind, RunStatus,
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
use kestic_core::sync::LockUnpoisoned;
use kestic_i18n::codes::{BlockedReason, ErrorKind, RunProgressPhase};
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

/// Starts a backup manually (enforcing `RunConditions`).
pub fn start_backup_async(
    shared: &Arc<Mutex<DaemonCore>>,
    backup_id: &str,
) -> Result<String, KesticServiceErr> {
    start_backup_internal(shared, backup_id, false, false)
}

/// Starts a backup manually overriding unmet `RunConditions` for this single run.
pub fn start_backup_override_async(
    shared: &Arc<Mutex<DaemonCore>>,
    backup_id: &str,
) -> Result<String, KesticServiceErr> {
    start_backup_internal(shared, backup_id, true, false)
}

/// Starts a scheduled backup triggered by the scheduler.
pub(crate) fn start_scheduled_backup_async(
    shared: &Arc<Mutex<DaemonCore>>,
    backup_id: &str,
) -> Result<String, KesticServiceErr> {
    start_backup_internal(shared, backup_id, false, true)
}

struct BackupClaimContext {
    backup: Backup,
    dest: Destination,
    deps: ResticDeps,
    state_dir: PathBuf,
    mount_inspector: Arc<dyn MountInspector>,
    system_env: SystemEnvironment,
    supervision: SupervisionConfig,
    claim_guard: DestinationClaimGuard,
}

fn claim_backup_destination(
    shared: &Arc<Mutex<DaemonCore>>,
    b_id: &BackupId,
    is_scheduled: bool,
) -> Result<BackupClaimContext, KesticServiceErr> {
    let mut guard = shared.lock_unpoisoned();
    let (backup, dest) = guard.backup_with_destination(b_id)?;
    let claim_guard = guard.claim_destination(shared, &dest, is_scheduled)?;
    Ok(BackupClaimContext {
        backup,
        dest,
        deps: guard.restic_deps(),
        state_dir: guard.state_dir.clone(),
        mount_inspector: Arc::clone(&guard.mount_inspector),
        system_env: guard.system_env.clone(),
        supervision: guard.supervision_config(),
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
        guard.release_claim(&dest.id, claim_guard);
        guard.push_state();
        return if !source_ok {
            KesticServiceErr::ManualRunBlocked(reason)
        } else {
            KesticServiceErr::DestinationUnreachable(dest_res.unwrap_err().to_string())
        };
    }

    notify_manual_run_blocked(guard, claim_guard, backup, &dest.id, reason, &source_body)
}

/// Ends the claim of a refused manual start, tells the GUI with `ManualRunBlocked` and, when no
/// window is open, shows a notification with `body`. Returns the error for the caller.
fn notify_manual_run_blocked(
    guard: &mut DaemonCore,
    claim_guard: &mut DestinationClaimGuard,
    backup: &Backup,
    dest_id: &DestinationId,
    reason: BlockedReason,
    body: &str,
) -> KesticServiceErr {
    guard.release_claim(dest_id, claim_guard);
    let window_active = guard.window_active;
    let conn_opt = guard.conn.clone();
    let state_dto = guard.push_tray_state();
    if let Some(conn) = &conn_opt {
        emit_manual_run_blocked(conn, &backup.id, reason);
        emit_status_changed(conn, &state_dto);
    }
    if !window_active {
        let title = i18n("%1: Backup cannot start").subs(&[&backup.name]);
        let _ = send_event_notification(KesticEvent::ManualRunBlocked, &title, body);
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
    notify_manual_run_blocked(
        guard,
        claim_guard,
        backup,
        &dest.id,
        BlockedReason::ConditionNotMet,
        &body,
    )
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

    let ActivatedRun {
        cancel,
        conn_opt,
        tray_opt,
    } = guard.activate_run(
        NewActiveRun {
            run: &run,
            backup_id: Some(&backup.id),
            display_name: &backup.name,
            dest_id: &dest.id,
        },
        claim_guard,
    );
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
    let b_id = parse_backup_id(backup_id)?;

    let mut ctx = claim_backup_destination(shared, &b_id, is_scheduled)?;

    let (probe, resolved) = probe_before_run(
        shared,
        &ctx.deps.provider,
        ctx.deps.secret_store.as_ref(),
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
        let mut guard = shared.lock_unpoisoned();
        guard.record_probe(probe);
        if is_scheduled && guard.sleep_in_progress {
            guard.release_claim(&ctx.dest.id, &mut ctx.claim_guard);
            return Err(KesticServiceErr::SleepInProgress);
        }

        let now = guard.clock.now_utc();
        guard
            .source_availability
            .insert(ctx.backup.id.clone(), source_status.clone());
        if source_ok {
            guard.availability_probe_count += 1;
            let dto = DestinationAvailabilityDto::from_probe(
                &ctx.dest.id,
                &dest_res,
                now.unix_timestamp(),
            );
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
    /// Phase and highest percentage shown so far (no jumping percentage).
    tracker: BackupProgressTracker,
}

impl BackupProgressReporter<'_> {
    fn report(&mut self, st: &ResticStatusMessage) {
        let elapsed_secs = (self.clock.now_unix_secs() - self.start_unix).max(1) as u64;
        let p = self.tracker.update(st, elapsed_secs);

        if let Some(jv) = self.job_view {
            // Without a total, Plasma would show «4.9 GB of 0 B»; the text names the phase.
            let processed_bytes = match p.phase {
                RunProgressPhase::Scanning => 0,
                RunProgressPhase::ScanFinished => p.bytes_done,
            };
            let _ = jv.update(&JobProgress {
                percent: p.percent_done.round() as u32,
                processed_bytes,
                total_bytes: p.total_bytes,
                speed_bytes_per_sec: p.speed_bps,
                info_message: tr(&active_run_message(&ActiveRunInfo {
                    name: self.backup_name,
                    is_maintenance: false,
                    phase: p.phase,
                    percent_done: p.percent_done,
                })),
                elapsed_ms: elapsed_secs * 1000,
            });
        }

        let mut guard = self.shared_worker.lock_unpoisoned();
        let mut progress = None;
        if let Some(entry) = guard.active_runs.get_mut(self.run_id) {
            entry.progress.phase = p.phase;
            entry.progress.percent_done = p.percent_done;
            entry.progress.bytes_done = p.bytes_done;
            entry.progress.total_bytes = p.total_bytes;
            entry.progress.files_done = p.files_done;
            entry.progress.speed_bps = p.speed_bps;
            entry.progress.seconds_remaining = p.seconds_remaining.unwrap_or(0);
            progress = Some(entry.progress.clone());
        }
        let (_, tray_state) = guard.build_state_and_tray();
        if let Some(tray) = self.tray_opt {
            tray.set_full_state(tray_state);
        }
        drop(guard);

        if let (Some(conn), Some(progress)) = (self.conn_opt, progress) {
            emit_run_progress(conn, &progress);
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

    let mut supervision = RunSupervision::start(
        &ctx.supervision,
        &i18n("Backing up %1").subs(&[&ctx.backup.name]),
        &cancel,
    );

    let run_id = run.id.clone();
    let start_unix = ctx.deps.clock.now_unix_secs();
    let home = current_home_dir();

    let runtime = ctx.deps.runtime(&resolved);
    let exec_ctx = BackupExecutionContext {
        state_dir: &ctx.state_dir,
        home_dir: home.as_deref(),
        inspector: ctx.mount_inspector.as_ref(),
        cancel: Some(&cancel),
        extra_env: &[],
    };
    let mut reporter = BackupProgressReporter {
        backup_name: &ctx.backup.name,
        start_unix,
        clock: ctx.deps.clock.as_ref(),
        job_view: supervision.job_view.as_ref(),
        shared_worker: &shared_worker,
        run_id: &run_id,
        tray_opt: tray_opt.as_ref(),
        conn_opt: conn_opt.as_ref(),
        tracker: BackupProgressTracker::new(),
    };
    let exec_res =
        execute_backup_with_inspector(&ctx.backup, &ctx.dest, &runtime, &exec_ctx, |ev| {
            if let ResticStreamEvent::Status(st) = ev {
                reporter.report(st);
            }
        });
    supervision.stop_cancel_watcher();

    let sleep_interrupted = shared_worker
        .lock_unpoisoned()
        .run_sleep_interrupted(&run_id);

    let secrets_owned = load_dest_secrets(&ctx.dest, ctx.deps.secret_store.as_ref());
    let secret_strs = secret_strs(&secrets_owned);

    let outcome = process_backup_exec_result(
        BackupExecContext {
            shared_worker: &shared_worker,
            backup: &ctx.backup,
            dest: &ctx.dest,
            runtime: &runtime,
            pending_other_backups: &pending_other_backups,
            cancel: &cancel,
            sleep_interrupted,
            is_scheduled,
            secret_strs: &secret_strs,
        },
        &mut run,
        exec_res,
    );

    run.finish(
        ctx.deps.clock.as_ref(),
        outcome.final_status,
        outcome.exit_code,
    );
    if let Some(jv) = supervision.job_view.take() {
        let _ = jv.terminate(outcome.jv_code, &outcome.jv_msg);
    }

    finalize_backup_run_state(
        &shared_worker,
        &ctx,
        run,
        outcome,
        &secret_strs,
        supervision.logind.as_mut(),
        (conn_opt.as_ref(), tray_opt.as_ref()),
    );
}

/// The secrets as text, for redacting them from logs and messages.
pub(super) fn secret_strs(secrets: &[SecretValue]) -> Vec<&str> {
    secrets.iter().filter_map(|s| s.as_str().ok()).collect()
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

fn process_backup_exec_result(
    ctx: BackupExecContext<'_>,
    run: &mut Run,
    exec_res: Result<ResticExecutionResult, BackupRunError>,
) -> BackupPostExecOutcome {
    match exec_res {
        Ok(res) => process_backup_success_result(ctx, run, res),
        Err(err) => process_backup_error_result(
            run,
            err,
            ctx.sleep_interrupted,
            ctx.is_scheduled,
            ctx.secret_strs,
        ),
    }
}

struct BackupExecContext<'a> {
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
    ctx: BackupExecContext<'_>,
    run: &mut Run,
    res: ResticExecutionResult,
) -> BackupPostExecOutcome {
    let BackupExecContext {
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
        let guard = shared_worker.lock_unpoisoned();
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
        // Pre-start availability failures keep `exit_code = None` (transient);
        // other destination errors carry their restic exit code so permanent errors (Exit 12,
        // Exit 10) block the schedule.
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
                        i18n("Backed up; destination was locked, retention runs later").into()
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
        let mut guard = shared_worker.lock_unpoisoned();
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

        let finish_now = ctx.deps.clock.now_utc();
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
            kestic_core::scheduler::clear_backup_backoff(&mut rs);
            rs.last_overdue_notified_at = None;
            rs.overdue_anchor_at = Some(finish_now);
            let _ = guard.store_backup_runtime_state(&ctx.backup.id, &rs);
        } else if (outcome.final_status == RunStatus::Error
            && outcome.exit_code.is_some()
            && !run_blocks_schedule(&run))
            || (outcome.final_status == RunStatus::Cancelled
                && !guard.shutdown_flag.load(Ordering::SeqCst))
        {
            // Alert-only restic errors (neither transient nor blocking) and user cancels count
            // as handling the due slot: no restart before the next regular slot. Permanent
            // errors block anyway; pre-start failures (no exit code: source or destination
            // unavailable) stay retryable.
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
            .prune_logs_for_backup(&ctx.backup.id, ctx.deps.clock.as_ref())
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
