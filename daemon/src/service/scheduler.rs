// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Scheduler evaluation tick, global/per-backup pause/resume, and settings updates.

use super::availability::{drain_queued_backups_async, trigger_background_source_probe};
use super::backup_runner::{start_scheduled_backup_async, upsert_pending_run_entry};
use super::error::KesticServiceErr;
use super::error::parse_backup_id;
use super::maintenance::run_scheduled_maintenance_async;
use super::run_errors::{destination_transport, error_run_kind, run_blocks_schedule};
use super::state::DaemonCore;
use crate::dbus_server::emit_status_changed;
use crate::i18n::{i18n, i18np};
use crate::notifications::{KesticEvent, send_configured_notification};
use kestic_core::history::PauseState;
use kestic_core::model::{
    Backup, BackupId, Destination, DestinationId, NotificationSettings, RunKind, RunStatus,
    Schedule,
};
use kestic_core::restic::Transport;
use kestic_core::scheduler::{
    BackupScheduleEvalInput, BackupScheduleEvaluation, DestinationMaintenanceEvalInput,
    DestinationMaintenanceNeed, DestinationTickAction, PauseDuration, backup_slots_between,
    collect_pending_preconditions, compute_pause_until, evaluate_backup_schedule,
    evaluate_destination_maintenance, evaluate_run_conditions, is_pause_active,
    maintenance_slots_between, plan_destination_actions, validate_pause_duration,
};
use kestic_core::sync::LockUnpoisoned;
use kestic_i18n::codes::ErrorKind;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::thread;
use time::OffsetDateTime;

impl DaemonCore {
    pub fn pause_global(&mut self, duration_json: &str) -> Result<(), KesticServiceErr> {
        let duration: PauseDuration = serde_json::from_str(duration_json).map_err(|e| {
            KesticServiceErr::InvalidInput(format!("invalid pause duration JSON: {e}"))
        })?;
        validate_pause_duration(duration)
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        let now = self.clock.now_utc();
        let schedules: Vec<&Schedule> = self.config.backups.iter().map(|b| &b.schedule).collect();
        let paused_until = compute_pause_until(now, duration, &schedules, &self.timezone);
        let pause = PauseState {
            paused_since: now,
            paused_until,
        };
        self.set_global_pause_cached(Some(&pause))
            .map_err(|e| KesticServiceErr::Internal(e.to_string()))?;
        let backup_ids: Vec<BackupId> = self.config.backups.iter().map(|b| b.id.clone()).collect();
        for bid in &backup_ids {
            let _ = self.delete_pending_runs_cached(bid);
        }
        self.queued_backups.clear();
        Ok(())
    }

    pub fn resume_global(&mut self) -> Result<(), KesticServiceErr> {
        let now = self.clock.now_utc();
        self.set_global_pause_cached(None)
            .map_err(|e| KesticServiceErr::Internal(e.to_string()))?;
        // reset overdue anchor on resume so paused slots do not count as overdue.
        let backup_ids: Vec<BackupId> = self.config.backups.iter().map(|b| b.id.clone()).collect();
        for id in &backup_ids {
            let mut rs = self.load_backup_runtime_state(id);
            if !is_pause_active(rs.pause.as_ref(), now) {
                rs.overdue_anchor_at = Some(now);
                rs.last_overdue_notified_at = None;
                let _ = self.store_backup_runtime_state(id, &rs);
            }
        }
        Ok(())
    }

    pub fn pause_backup(
        &mut self,
        backup_id: &str,
        duration_json: &str,
    ) -> Result<(), KesticServiceErr> {
        let b_id = parse_backup_id(backup_id)?;
        let backup = self
            .config
            .backup(&b_id)
            .ok_or_else(|| KesticServiceErr::NotFound(backup_id.to_string()))?;
        let duration: PauseDuration = serde_json::from_str(duration_json).map_err(|e| {
            KesticServiceErr::InvalidInput(format!("invalid pause duration JSON: {e}"))
        })?;
        validate_pause_duration(duration)
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        let now = self.clock.now_utc();
        let paused_until = compute_pause_until(now, duration, &[&backup.schedule], &self.timezone);
        let mut rs = self.load_backup_runtime_state(&b_id);
        rs.pause = Some(PauseState {
            paused_since: now,
            paused_until,
        });
        self.store_backup_runtime_state(&b_id, &rs)
            .map_err(|e| KesticServiceErr::Internal(e.to_string()))?;
        let _ = self.delete_pending_runs_cached(&b_id);
        self.queued_backups.retain(|id| id != &b_id);
        Ok(())
    }

    pub fn resume_backup(&mut self, backup_id: &str) -> Result<(), KesticServiceErr> {
        let b_id = parse_backup_id(backup_id)?;
        if self.config.backup(&b_id).is_none() {
            return Err(KesticServiceErr::NotFound(backup_id.to_string()));
        }
        let now = self.clock.now_utc();
        let mut rs = self.load_backup_runtime_state(&b_id);
        rs.pause = None;
        // reset overdue anchor to resume timestamp.
        rs.overdue_anchor_at = Some(now);
        rs.last_overdue_notified_at = None;
        self.store_backup_runtime_state(&b_id, &rs)
            .map_err(|e| KesticServiceErr::Internal(e.to_string()))?;
        Ok(())
    }

    pub fn update_settings(
        &mut self,
        default_overdue_threshold: u32,
        notifications_json: &str,
    ) -> Result<(), KesticServiceErr> {
        if default_overdue_threshold == 0 {
            return Err(KesticServiceErr::InvalidInput(
                "default_overdue_threshold must be >= 1".to_string(),
            ));
        }
        let notifications: NotificationSettings = serde_json::from_str(notifications_json)
            .map_err(|e| {
                KesticServiceErr::InvalidInput(format!("invalid notifications JSON: {e}"))
            })?;
        let mut next = self.config.clone();
        next.default_overdue_threshold = default_overdue_threshold;
        next.notifications = notifications;
        next.validate()
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        self.persist_config(next)?;
        Ok(())
    }
}

/// Planned outcome of one scheduler evaluation (computed under the `DaemonCore` lock).
struct TickPlan {
    actions: Vec<DestinationTickAction>,
    overdue_notices: Vec<(String, usize)>,
    conn_opt: Option<zbus::Connection>,
    state_dto_opt: Option<crate::dbus_api::DaemonStateDto>,
    notif_settings: NotificationSettings,
    /// A queued backup's destination is idle (e.g. its earlier start lost a claim race).
    drain_queue: bool,
}

/// Evaluates scheduled backups & destination maintenance for one tick.
///
/// Runs on the daemon main loop, so it only evaluates and claims under the `DaemonCore` lock;
/// everything that can block (restic/Secret Service probe, source stat, destination preflight,
/// desktop notifications) runs on worker threads, keeping `PrepareForSleep` handling prompt.
pub fn tick_scheduler_async(shared: &Arc<Mutex<DaemonCore>>) {
    trigger_background_source_probe(Arc::clone(shared));

    let plan = {
        let mut guard = shared.lock_unpoisoned();
        if guard.sleep_in_progress || guard.stopping() {
            return;
        }
        plan_scheduler_tick(&mut guard)
    };

    if !plan.overdue_notices.is_empty() {
        let notif_settings = plan.notif_settings.clone();
        let notices = plan.overdue_notices;
        thread::spawn(move || {
            for (name, missed) in notices {
                let summary = i18n("%1: Backup overdue").subs(&[&name]);
                let body = i18np(
                    "%1 scheduled backup missed",
                    "%1 scheduled backups missed",
                    missed,
                    &[],
                );
                let _ = send_configured_notification(
                    &notif_settings,
                    KesticEvent::BackupOverdue,
                    &summary,
                    &body,
                );
            }
        });
    }

    if let (Some(conn), Some(state_dto)) = (&plan.conn_opt, &plan.state_dto_opt) {
        emit_status_changed(conn, state_dto);
    }

    dispatch_tick_actions(shared, plan.actions);

    if plan.drain_queue {
        let shared_drain = Arc::clone(shared);
        thread::spawn(move || drain_queued_backups_async(&shared_drain));
    }
}

fn plan_scheduler_tick(guard: &mut DaemonCore) -> TickPlan {
    let now = guard.clock.now_utc();
    let global_pause = expire_global_pause_if_due(guard, now);

    let backups_snapshot = guard.config.backups.clone();
    let destinations_snapshot = guard.config.destinations.clone();
    let mut busy_dests: HashSet<DestinationId> = guard
        .active_runs
        .values()
        .filter(|e| !e.is_restore())
        .map(|e| e.run.destination_id.clone())
        .collect();
    busy_dests.extend(guard.claiming_destinations.iter().cloned());
    busy_dests.extend(guard.scheduled_dispatch.keys().cloned());
    // Backups run beside a restore; maintenance waits for it.
    let mut maintenance_busy = busy_dests.clone();
    maintenance_busy.extend(
        guard
            .active_runs
            .values()
            .filter(|e| e.is_restore())
            .map(|e| e.run.destination_id.clone()),
    );

    let mut backup_dest_map = HashMap::new();
    let mut evaluations = Vec::with_capacity(backups_snapshot.len());
    let mut overdue_notices: Vec<(String, usize)> = Vec::new();

    for backup in &backups_snapshot {
        // The start of this backup is already being dispatched on a worker thread.
        if guard
            .scheduled_dispatch
            .values()
            .any(|b| b.as_ref() == Some(&backup.id))
        {
            continue;
        }
        backup_dest_map.insert(backup.id.clone(), backup.destination_id.clone());
        let eval = evaluate_single_backup_tick(
            guard,
            backup,
            global_pause.as_ref(),
            &busy_dests,
            now,
            &mut overdue_notices,
        );
        evaluations.push(eval);
    }

    let maint_needs: Vec<DestinationMaintenanceNeed> = destinations_snapshot
        .iter()
        .map(|dest| {
            evaluate_scheduled_maintenance_tick(
                guard,
                dest,
                &backups_snapshot,
                global_pause.as_ref(),
                &maintenance_busy,
                now,
            )
        })
        .collect();

    // Locked password store / unusable restic: wait for the re-probe cooldown.
    let actions = if guard.scheduled_probe_blocked(now) {
        Vec::new()
    } else {
        plan_destination_actions(&evaluations, &maint_needs, &busy_dests, &backup_dest_map)
    };
    for action in &actions {
        let (dest_id, backup_id) = match action {
            DestinationTickAction::StartBackup {
                backup_id,
                destination_id,
            } => (destination_id.clone(), Some(backup_id.clone())),
            DestinationTickAction::StartMaintenance { destination_id, .. } => {
                (destination_id.clone(), None)
            }
        };
        busy_dests.insert(dest_id.clone());
        guard.scheduled_dispatch.insert(dest_id, backup_id);
    }
    let drain_queue = guard.queued_backups.iter().any(|bid| {
        guard
            .config
            .backup(bid)
            .is_some_and(|b| !busy_dests.contains(&b.destination_id))
    });

    let state_dto_opt = if !overdue_notices.is_empty() || !actions.is_empty() {
        Some(guard.push_tray_state())
    } else {
        None
    };

    TickPlan {
        actions,
        overdue_notices,
        conn_opt: guard.conn.clone(),
        state_dto_opt,
        notif_settings: guard.config.notifications.clone(),
        drain_queue,
    }
}

fn expire_global_pause_if_due(guard: &mut DaemonCore, now: OffsetDateTime) -> Option<PauseState> {
    let mut global_pause = guard.global_pause_cached();
    if let Some(ref gp) = global_pause
        && let Some(until) = gp.paused_until
        && now >= until
    {
        let _ = guard.set_global_pause_cached(None);
        global_pause = None;
        let backup_ids: Vec<BackupId> = guard.config.backups.iter().map(|b| b.id.clone()).collect();
        for id in &backup_ids {
            let mut rs = guard.load_backup_runtime_state(id);
            if !is_pause_active(rs.pause.as_ref(), now) {
                let anchor = rs.overdue_anchor_at.map_or(until, |a| a.max(until));
                rs.overdue_anchor_at = Some(anchor);
                rs.last_overdue_notified_at = None;
                let _ = guard.store_backup_runtime_state(id, &rs);
            }
        }
    }
    global_pause
}

fn evaluate_single_backup_tick(
    guard: &mut DaemonCore,
    backup: &Backup,
    global_pause: Option<&PauseState>,
    busy_dests: &HashSet<DestinationId>,
    now: OffsetDateTime,
    overdue_notices: &mut Vec<(String, usize)>,
) -> BackupScheduleEvaluation {
    let mut rs = guard.load_backup_runtime_state(&backup.id);

    // Expire finished per-backup pause.
    if let Some(ref p) = rs.pause
        && let Some(until) = p.paused_until
        && now >= until
    {
        rs.pause = None;
        let anchor = rs.overdue_anchor_at.map_or(until, |a| a.max(until));
        rs.overdue_anchor_at = Some(anchor);
        rs.last_overdue_notified_at = None;
        let _ = guard.store_backup_runtime_state(&backup.id, &rs);
    }

    let last_success_at = guard.last_successful_backup_at_cached(&backup.id);

    // Reset stale backoff state from an earlier slot once a newer regular slot arrives.
    let due_now_slots = {
        let catchup_anchor =
            kestic_core::scheduler::catchup_anchor(last_success_at, rs.baseline_at, now);
        backup_slots_between(&backup.schedule, catchup_anchor, now, &guard.timezone)
    };
    if let (Some(latest_slot), Some(failed_at)) =
        (due_now_slots.last().copied(), rs.last_failure_at)
        && latest_slot > failed_at
        && (rs.backoff_attempt > 0 || rs.backoff_until.is_some())
    {
        kestic_core::scheduler::clear_backup_backoff(&mut rs);
        let _ = guard.store_backup_runtime_state(&backup.id, &rs);
    }

    let backoff_elapsed = rs.backoff_until.is_some_and(|until| now >= until);

    // Only permanent errors (wrong password, missing repository, configuration) block the
    // schedule; transient failures (network, DNS, lock) are retried.
    let has_error = guard
        .runs_cache
        .iter()
        .find(|r| {
            r.is_backup_run_of(&backup.id)
                && !matches!(r.status, RunStatus::Pending | RunStatus::Running)
        })
        .is_some_and(|r| {
            run_blocks_schedule(
                r,
                destination_transport(&guard.config, &backup.destination_id),
            )
        });
    let source_ok = guard
        .source_availability
        .get(&backup.id)
        .is_none_or(|s| s.is_ok());
    let cached_dest_ok = guard
        .dest_availability
        .get(&backup.destination_id)
        .is_none_or(|d| d.reachable);
    let dest_busy = busy_dests.contains(&backup.destination_id);
    let can_probe_unreachable_dest = !cached_dest_ok
        && !dest_busy
        && !has_error
        && source_ok
        && !due_now_slots.is_empty()
        && !is_pause_active(rs.pause.as_ref(), now)
        && !is_pause_active(global_pause, now)
        && evaluate_run_conditions(&backup.run_conditions, &guard.system_env).is_empty()
        && (backoff_elapsed || (rs.backoff_attempt == 0 && rs.backoff_until.is_none()));
    let dest_ok = cached_dest_ok || can_probe_unreachable_dest;

    let mut eval = evaluate_backup_schedule(
        &BackupScheduleEvalInput {
            backup,
            runtime_state: &rs,
            last_success_at,
            overdue_anchor_at: None,
            global_pause,
            global_overdue_threshold: guard.config.default_overdue_threshold,
            source_available: source_ok,
            destination_reachable: dest_ok,
            destination_busy: dest_busy,
            has_permanent_error: has_error,
        },
        &guard.system_env,
        now,
        &guard.timezone,
    );

    // Allow sleep-interrupted backups (even ManualOnly) to resume after wake.
    if eval.due_slot.is_none()
        && !eval.is_paused
        && !has_error
        && guard.sleep_interrupted_backups.contains(&backup.id)
    {
        let unmet = collect_pending_preconditions(
            source_ok,
            dest_ok,
            &backup.run_conditions,
            &guard.system_env,
            dest_busy,
            eval.in_backoff,
        );
        eval.due_slot = Some(now);
        eval.should_start_now = unmet.is_empty();
        eval.unmet_preconditions = unmet;
    }

    if eval.is_overdue && rs.last_overdue_notified_at.is_none() {
        rs.last_overdue_notified_at = Some(now);
        let _ = guard.store_backup_runtime_state(&backup.id, &rs);
        overdue_notices.push((backup.name.clone(), eval.missed_slots_for_overdue));
    }

    if !eval.should_start_now
        && !eval.is_paused
        && !has_error
        && !dest_busy
        && !eval.unmet_preconditions.is_empty()
    {
        let primary = &eval.unmet_preconditions[0];
        let msg = guard.localized_pending_reason(primary, rs.backoff_until, now);
        upsert_pending_run_entry(guard, backup, primary.to_pending_reason(), msg);
    } else if (eval.is_paused || has_error) && !guard.queued_backups.contains(&backup.id) {
        // Without a due slot the scheduler never wrote a pending entry; pending entries of
        // not-due backups (e.g. a «Back up all» skip) stay in the history until the
        // next run of that backup replaces them.
        let _ = guard.delete_pending_runs_cached(&backup.id);
    }

    eval
}

fn evaluate_scheduled_maintenance_tick(
    guard: &mut DaemonCore,
    dest: &Destination,
    backups_snapshot: &[Backup],
    global_pause: Option<&PauseState>,
    busy_dests: &HashSet<DestinationId>,
    now: OffsetDateTime,
) -> DestinationMaintenanceNeed {
    let mut ds = guard.load_destination_maintenance_state(&dest.id);
    if let Some(failed_at) = ds.last_maintenance_failure_at
        && (ds.maintenance_backoff_attempt > 0 || ds.maintenance_backoff_until.is_some())
    {
        let (prune_anchor, check_anchor) = kestic_core::scheduler::maintenance_anchors(&ds, now);
        let latest_slot =
            maintenance_slots_between(&dest.maintenance.prune, prune_anchor, now, &guard.timezone)
                .into_iter()
                .chain(maintenance_slots_between(
                    &dest.maintenance.check,
                    check_anchor,
                    now,
                    &guard.timezone,
                ))
                .max();
        if latest_slot.is_some_and(|s| s > failed_at) {
            kestic_core::scheduler::clear_maintenance_backoff(&mut ds);
            let _ = guard.store_destination_maintenance_state(&dest.id, &ds);
        }
    }
    let has_pending_forget = backups_snapshot
        .iter()
        .any(|b| b.destination_id == dest.id && guard.is_pending_forget_cached(&b.id));
    let dest_busy = busy_dests.contains(&dest.id);
    let dest_ok = guard
        .dest_availability
        .get(&dest.id)
        .is_none_or(|d| d.reachable);
    // A maintenance run stopped at an unknown or changed SFTP host key is a permanent error:
    // scheduled maintenance waits until a run ends without it.
    let stopped_at_host_key = guard
        .runs_cache
        .iter()
        .find(|r| r.destination_id == dest.id && matches!(r.kind, RunKind::Maintenance))
        .is_some_and(|r| {
            r.status == RunStatus::Error
                && matches!(
                    error_run_kind(r, Transport::of(&dest.kind)),
                    ErrorKind::HostKeyUnknown | ErrorKind::HostKeyChanged
                )
        });
    let mut need = evaluate_destination_maintenance(
        &DestinationMaintenanceEvalInput {
            destination: dest,
            state: &ds,
            has_pending_forget,
            global_pause,
            destination_busy: dest_busy,
            destination_reachable: dest_ok,
        },
        &guard.system_env,
        now,
        &guard.timezone,
    );
    if stopped_at_host_key {
        need.due_now = false;
    }
    need
}

/// Removes a destination from [`DaemonCore::scheduled_dispatch`] when its worker finishes.
struct ScheduledDispatchGuard {
    shared: Arc<Mutex<DaemonCore>>,
    dest_id: DestinationId,
}

impl Drop for ScheduledDispatchGuard {
    fn drop(&mut self) {
        let mut guard = self.shared.lock_unpoisoned();
        guard.scheduled_dispatch.remove(&self.dest_id);
    }
}

/// Starts each planned action on its own worker thread (probe, source stat, and preflight may
/// block for up to the connect timeout; one hung destination must not delay the others).
fn dispatch_tick_actions(shared: &Arc<Mutex<DaemonCore>>, actions: Vec<DestinationTickAction>) {
    for action in actions {
        let dest_id = match &action {
            DestinationTickAction::StartBackup { destination_id, .. }
            | DestinationTickAction::StartMaintenance { destination_id, .. } => {
                destination_id.clone()
            }
        };
        let dispatch_guard = ScheduledDispatchGuard {
            shared: Arc::clone(shared),
            dest_id,
        };
        let shared_bg = Arc::clone(shared);
        thread::spawn(move || {
            let _dispatch_guard = dispatch_guard;
            match action {
                DestinationTickAction::StartBackup { backup_id, .. } => {
                    let _ = start_scheduled_backup_async(&shared_bg, backup_id.as_str());
                }
                DestinationTickAction::StartMaintenance {
                    destination_id,
                    run_prune,
                    run_check,
                } => {
                    let _ = run_scheduled_maintenance_async(
                        &shared_bg,
                        destination_id.as_str(),
                        run_prune,
                        run_check,
                    );
                }
            }
        });
    }
}
