// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! State DTO, backup/destination summary, overall status, and tray state builders for `DaemonCore`.

use super::availability::DestinationClaimGuard;
use super::run_errors::extract_last_run_error_info;
use super::state::{ActiveRunEntry, DaemonCore, Interruption, SimulatedOverallStatus};
use crate::dbus_api::{
    ActiveRunProgressDto, BackupAvailabilityDto, BackupSummaryDto, BlockingStateDto,
    DaemonStateDto, DestinationSummaryDto, OverallStatusDto, OverallStatusKind, ResticStatusDto,
    blocking_message,
};
use crate::dbus_server::{emit_run_updated, emit_status_changed, emit_status_changed_async};
use crate::i18n::{i18n, tr};
use crate::tray::{KesticTrayState, TrayBackupItem, TrayHandle, compute_tray_presentation};
use kestic_core::history::{BackupRuntimeState, PauseState};
use kestic_core::model::{
    Backup, BackupId, Destination, DestinationId, NestedSourceConflict, Run, RunId, RunKind,
    RunStatus, Schedule, find_nested_source_warnings, lint_exclude_rules,
};
use kestic_core::restic::CancelHandle;
use kestic_core::revision::revision_of;
use kestic_core::scheduler::{
    BackupScheduleEvalInput, BackupScheduleEvaluation, DestinationMaintenanceEvalInput,
    UnmetPrecondition, collect_pending_preconditions, evaluate_backup_schedule,
    evaluate_destination_maintenance, evaluate_run_conditions, format_local_slot_hint,
    is_pause_active,
};
use kestic_i18n::codes::{PendingReasonCode, RunKindCode, RunProgressPhase};
use kestic_i18n::messages::{
    maintenance_running_message, overall_headline, schedules_paused_message,
};
use std::collections::HashMap;
use std::sync::Arc;
use time::OffsetDateTime;
use zbus::Connection;

/// A run that has just been recorded and now takes over its destination.
#[derive(Clone, Copy)]
pub(super) struct NewActiveRun<'a> {
    pub(super) run: &'a Run,
    /// `None` for maintenance runs, which belong to a destination only.
    pub(super) backup_id: Option<&'a BackupId>,
    /// Name of the backup, or of the destination for a maintenance run.
    pub(super) display_name: &'a str,
    pub(super) dest_id: &'a DestinationId,
}

/// What the run's worker needs after [`DaemonCore::activate_run`].
pub(super) struct ActivatedRun {
    pub(super) cancel: CancelHandle,
    pub(super) conn_opt: Option<Connection>,
}

struct OverallStatusCounts {
    failed_count: usize,
    overdue_count: usize,
    warning_count: usize,
    running_count: usize,
    waiting_count: usize,
    global_paused: bool,
    any_paused: bool,
    all_paused: bool,
    has_interrupted: bool,
    has_cancelled: bool,
    has_never_run: bool,
    all_manual: bool,
}

struct PendingReasonsSummary {
    codes: Vec<PendingReasonCode>,
    retry_at_unix: Option<i64>,
    unmet_run_conditions: Vec<PendingReasonCode>,
}

#[derive(Clone, Copy)]
struct TrayInput<'a> {
    backups: &'a [BackupSummaryDto],
    overall: &'a OverallStatusDto,
    blocking: Option<&'a BlockingStateDto>,
    restic_status: &'a ResticStatusDto,
    global_pause: Option<&'a PauseState>,
    paused_backup_window: Option<(OffsetDateTime, Option<OffsetDateTime>)>,
    now: OffsetDateTime,
}

#[derive(Clone, Copy)]
struct BackupPendingSummaryInput<'a> {
    backup: &'a Backup,
    rs: &'a BackupRuntimeState,
    eval: &'a BackupScheduleEvaluation,
    availability: &'a BackupAvailabilityDto,
    active_run: Option<&'a Run>,
    last_run: Option<&'a Run>,
    has_error: bool,
    dest_busy: bool,
}

/// The state snapshot taken under the daemon lock, published (tray and `StatusChanged`) after
/// the lock is released.
pub(crate) struct StatePublication {
    conn: Option<Connection>,
    state_dto: DaemonStateDto,
    tray: Option<Arc<TrayHandle>>,
    tray_state: KesticTrayState,
}

impl StatePublication {
    /// Pushes the snapshot to the tray and emits `StatusChanged`.
    pub(super) fn publish(self) {
        if let Some(tray) = self.tray {
            tray.set_full_state(self.tray_state);
        }
        if let Some(conn) = &self.conn {
            emit_status_changed(conn, &self.state_dto);
        }
    }

    /// [`Self::publish`] for a D-Bus method: the method returns once `StatusChanged` is sent.
    pub(crate) async fn publish_async(self) {
        if let Some(tray) = self.tray {
            tray.set_full_state(self.tray_state);
        }
        if let Some(conn) = &self.conn {
            emit_status_changed_async(conn, &self.state_dto).await;
        }
    }
}

impl DaemonCore {
    /// The state snapshot to publish once the lock is released.
    pub(crate) fn state_publication(&self) -> StatePublication {
        let (state_dto, tray_state) = self.build_state_and_tray();
        StatePublication {
            conn: self.conn.clone(),
            state_dto,
            tray: self.tray.clone(),
            tray_state,
        }
    }

    /// Rebuilds the state snapshot, pushes it to the tray and returns it for the
    /// `StatusChanged` signal.
    pub(super) fn push_tray_state(&self) -> DaemonStateDto {
        let (state_dto, tray_state) = self.build_state_and_tray();
        if let Some(tray) = &self.tray {
            tray.set_full_state(tray_state);
        }
        state_dto
    }

    /// Rebuilds the state snapshot, pushes it to the tray and emits `StatusChanged`.
    pub(super) fn push_state(&self) {
        let state_dto = self.push_tray_state();
        if let Some(conn) = &self.conn {
            emit_status_changed(conn, &state_dto);
        }
    }

    /// Ends the claim on `dest_id` once the run took over (or gave up on) the destination.
    pub(super) fn release_claim(
        &mut self,
        dest_id: &DestinationId,
        claim_guard: &mut DestinationClaimGuard,
    ) {
        self.claiming_destinations.remove(dest_id);
        claim_guard.disarm();
    }

    /// Registers `new` as an active run, ends the claim on its destination (restores claim
    /// none), updates the tray and announces the run with `RunUpdated` and `StatusChanged`.
    pub(super) fn activate_run(
        &mut self,
        new: NewActiveRun<'_>,
        claim_guard: Option<&mut DestinationClaimGuard>,
    ) -> ActivatedRun {
        let cancel = CancelHandle::new();
        let entry = ActiveRunEntry {
            run: new.run.clone(),
            backup_id: new.backup_id.cloned(),
            backup_name: new.display_name.to_string(),
            progress: ActiveRunProgressDto {
                run_id: new.run.id.as_str().to_string(),
                kind: run_kind_code(&new.run.kind),
                backup_id: new.backup_id.map(|id| id.as_str().to_string()),
                backup_name: new.display_name.to_string(),
                // Maintenance has no percentage; its texts check `backup_id: None` first.
                phase: RunProgressPhase::Scanning,
                percent_done: 0.0,
                bytes_done: 0,
                total_bytes: 0,
                files_done: 0,
                speed_bps: 0,
                seconds_remaining: 0,
            },
            cancel: cancel.clone(),
            interruption: None,
            exclusive: false,
        };
        self.active_runs.insert(new.run.id.clone(), entry);
        if let Some(claim_guard) = claim_guard {
            self.release_claim(new.dest_id, claim_guard);
        }

        let conn_opt = self.conn.clone();
        let state_dto = self.push_tray_state();
        if let Some(conn) = &conn_opt {
            emit_run_updated(conn, new.run);
            emit_status_changed(conn, &state_dto);
        }
        ActivatedRun { cancel, conn_opt }
    }

    /// Why the daemon stopped the active run `run_id`, if it did (sleep or `SIGTERM`).
    /// A run that ends after `SIGTERM` counts as stopped by it, also one that started in the
    /// moment before the daemon marked the running ones.
    pub(super) fn run_interruption(&self, run_id: &RunId) -> Option<Interruption> {
        self.active_runs
            .get(run_id)
            .and_then(|e| e.interruption)
            .or_else(|| self.signal_received().then_some(Interruption::Stop))
    }

    /// Computes full state snapshot (`DaemonStateDto`) and synchronized `KesticTrayState`.
    pub fn build_state_and_tray(&self) -> (DaemonStateDto, KesticTrayState) {
        let now = self.clock.now_utc();
        let (mut restic_status, secret_store_status, blocking) = self.cached_probe.clone();
        restic_status.install_id = self.running_restic_install();
        let nested_map = find_nested_source_warnings(&self.config.backups);
        let global_pause_raw = self.global_pause_cached();
        let global_paused = is_pause_active(global_pause_raw.as_ref(), now);

        let mut backup_summaries = Vec::with_capacity(self.config.backups.len());
        let mut dest_password_rejected: HashMap<DestinationId, bool> = HashMap::new();
        let mut paused_backup_window: Option<(OffsetDateTime, Option<OffsetDateTime>)> = None;

        for backup in &self.config.backups {
            let summary = self.build_backup_summary(
                backup,
                &nested_map,
                global_pause_raw.as_ref(),
                now,
                &mut dest_password_rejected,
                &mut paused_backup_window,
            );
            backup_summaries.push(summary);
        }

        let dest_summaries: Vec<DestinationSummaryDto> = self
            .config
            .destinations
            .iter()
            .map(|dest| {
                self.build_destination_summary(
                    dest,
                    &dest_password_rejected,
                    global_pause_raw.as_ref(),
                    now,
                )
            })
            .collect();

        let global_pause_active = if global_paused {
            global_pause_raw.as_ref()
        } else {
            None
        };
        let overall = self.compute_overall_status(
            &backup_summaries,
            &dest_summaries,
            blocking.as_ref(),
            global_pause_active,
            paused_backup_window,
            now,
        );

        let tray_state = self.build_tray_state(TrayInput {
            backups: &backup_summaries,
            overall: &overall,
            blocking: blocking.as_ref(),
            restic_status: &restic_status,
            global_pause: global_pause_active,
            paused_backup_window,
            now,
        });

        let recent_runs = self.runs_cache.iter().take(200).cloned().collect();
        let state = DaemonStateDto {
            daemon_version: crate::daemon_version().to_string(),
            daemon_build_commit: crate::build_commit().to_string(),
            has_config_file: self.has_config_file,
            config_load_error: self.config_load_error.clone(),
            config: self.config.clone(),
            restic_status,
            secret_store_status,
            blocking,
            overall,
            backups: backup_summaries,
            destinations: dest_summaries,
            recent_runs,
        };

        (state, tray_state)
    }

    fn build_backup_summary(
        &self,
        backup: &Backup,
        nested_map: &HashMap<BackupId, Vec<NestedSourceConflict>>,
        global_pause_raw: Option<&PauseState>,
        now: OffsetDateTime,
        dest_password_rejected: &mut HashMap<DestinationId, bool>,
        paused_backup_window: &mut Option<(OffsetDateTime, Option<OffsetDateTime>)>,
    ) -> BackupSummaryDto {
        let dest_name = self.config.destination(&backup.destination_id).map_or_else(
            || backup.destination_id.as_str().to_string(),
            |d| d.name.clone(),
        );

        let active_run = self
            .active_runs
            .values()
            .find(|entry| entry.backup_id.as_ref() == Some(&backup.id))
            .map(|entry| entry.run.clone());

        let last_run = self
            .runs_cache
            .iter()
            .find(|r| {
                r.is_backup_run_of(&backup.id)
                    && !matches!(r.status, RunStatus::Pending | RunStatus::Running)
            })
            .cloned()
            .or_else(|| {
                self.runs_cache
                    .iter()
                    .find(|r| r.is_backup_run_of(&backup.id))
                    .cloned()
            });

        let pending_forget = self.is_pending_forget_cached(&backup.id);
        let err_info = extract_last_run_error_info(
            last_run.as_ref(),
            super::run_errors::destination_transport(&self.config, &backup.destination_id),
            pending_forget,
            last_run
                .as_ref()
                .and_then(|r| self.run_unreadable_cache.get(&r.id)),
        );
        if err_info.wrong_password {
            dest_password_rejected.insert(backup.destination_id.clone(), true);
        }

        let availability = self.availability_for_backup(backup);
        let dest_busy = self.is_destination_busy(&backup.destination_id);
        let rs = self.backup_runtime_state(&backup.id);
        let last_success_at = self.last_successful_backup_at_cached(&backup.id);

        let eval = evaluate_backup_schedule(
            &BackupScheduleEvalInput {
                backup,
                runtime_state: &rs,
                last_success_at,
                overdue_anchor_at: None,
                global_pause: global_pause_raw,
                global_overdue_threshold: self.config.default_overdue_threshold,
                source_available: availability.source_available,
                destination_reachable: availability.destination_available,
                destination_busy: dest_busy,
                has_permanent_error: err_info.blocks_schedule,
            },
            &self.system_env,
            now,
            &self.timezone,
        );

        if eval.is_paused
            && let Some(ps) = eval.paused_since
            && paused_backup_window.is_none()
        {
            *paused_backup_window = Some((ps, eval.paused_until));
        }

        let pending = self.resolve_backup_pending_summary(BackupPendingSummaryInput {
            backup,
            rs: &rs,
            eval: &eval,
            availability: &availability,
            active_run: active_run.as_ref(),
            last_run: last_run.as_ref(),
            has_error: err_info.blocks_schedule,
            dest_busy,
        });

        BackupSummaryDto {
            backup: backup.clone(),
            revision: revision_of(backup),
            destination_name: dest_name,
            last_run,
            active_run,
            availability,
            has_error: err_info.has_error,
            is_overdue: eval.is_overdue,
            error_action: err_info.error_action,
            error_kind: err_info.error_kind,
            error_detail: err_info.error_detail,
            error_exit_code: err_info.error_exit_code,
            unreadable_files: err_info.unreadable_files,
            pending_forget,
            nested_warnings: nested_map.get(&backup.id).cloned().unwrap_or_default(),
            exclude_lint_warnings: lint_exclude_rules(&backup.excludes),
            is_paused: eval.is_paused,
            paused_since_unix: eval.paused_since.map(|t| t.unix_timestamp()),
            paused_until_unix: eval.paused_until.map(|t| t.unix_timestamp()),
            next_scheduled_at_unix: eval.next_scheduled_at.map(|t| t.unix_timestamp()),
            pending_primary_code: pending.codes.first().copied(),
            pending_reason_codes: pending.codes,
            pending_retry_at_unix: pending.retry_at_unix,
            missed_slots_count: eval.missed_slots_for_overdue,
            has_unmet_run_condition: !pending.unmet_run_conditions.is_empty(),
            battery_percent: pending
                .unmet_run_conditions
                .contains(&PendingReasonCode::OnBattery)
                .then_some(self.system_env.battery_percent)
                .flatten(),
            unmet_run_conditions: pending.unmet_run_conditions,
        }
    }

    fn resolve_backup_pending_summary(
        &self,
        input: BackupPendingSummaryInput<'_>,
    ) -> PendingReasonsSummary {
        let BackupPendingSummaryInput {
            backup,
            rs,
            eval,
            availability,
            active_run,
            last_run,
            has_error,
            dest_busy,
        } = input;
        let interrupted_recovery = active_run.is_none()
            && !eval.is_paused
            && !has_error
            && eval.due_slot.is_none()
            && last_run.is_some_and(|r| r.status == RunStatus::Interrupted);

        let effective_unmet = if active_run.is_some() || eval.is_paused || has_error {
            Vec::new()
        } else if !eval.unmet_preconditions.is_empty() {
            eval.unmet_preconditions.clone()
        } else if self.queued_backups.contains(&backup.id) {
            vec![UnmetPrecondition::Queued]
        } else if interrupted_recovery {
            collect_pending_preconditions(
                availability.source_available,
                availability.destination_available,
                &backup.run_conditions,
                &self.system_env,
                dest_busy,
                eval.in_backoff,
            )
        } else {
            Vec::new()
        };

        let codes = effective_unmet
            .iter()
            .map(PendingReasonCode::from)
            .collect();
        let retry_at_unix = effective_unmet
            .contains(&UnmetPrecondition::Backoff)
            .then(|| rs.backoff_until.map(|t| t.unix_timestamp()))
            .flatten();

        let unmet_run_conditions =
            evaluate_run_conditions(&backup.run_conditions, &self.system_env)
                .iter()
                .map(PendingReasonCode::from)
                .collect();

        PendingReasonsSummary {
            codes,
            retry_at_unix,
            unmet_run_conditions,
        }
    }

    fn build_destination_summary(
        &self,
        dest: &Destination,
        dest_password_rejected: &HashMap<DestinationId, bool>,
        global_pause_raw: Option<&PauseState>,
        now: OffsetDateTime,
    ) -> DestinationSummaryDto {
        let backup_count = self
            .config
            .backups
            .iter()
            .filter(|b| b.destination_id == dest.id)
            .count();
        let avail = self.availability_for_destination(dest);
        let password_rejected = dest_password_rejected
            .get(&dest.id)
            .copied()
            .unwrap_or(false);
        let stale_lock_count = self.dest_stale_locks.get(&dest.id).copied().unwrap_or(0);
        let has_stale_lock = stale_lock_count > 0;
        let ds = self.destination_maintenance_state(&dest.id);
        let pending_forget_count = self
            .config
            .backups
            .iter()
            .filter(|b| b.destination_id == dest.id && self.is_pending_forget_cached(&b.id))
            .count();
        // Maintenance waits for every other run on the destination, restores included.
        let dest_busy = self
            .active_runs
            .values()
            .any(|e| e.run.destination_id == dest.id);
        let m_eval = evaluate_destination_maintenance(
            &DestinationMaintenanceEvalInput {
                destination: dest,
                state: &ds,
                has_pending_forget: pending_forget_count > 0,
                global_pause: global_pause_raw,
                destination_busy: dest_busy,
                destination_reachable: avail.reachable,
            },
            &self.system_env,
            now,
            &self.timezone,
        );
        let last_maintenance_run = self
            .runs_cache
            .iter()
            .find(|r| r.destination_id == dest.id && matches!(r.kind, RunKind::Maintenance))
            .cloned();
        let maintenance_running = self
            .active_runs
            .values()
            .any(|e| e.run.destination_id == dest.id && matches!(e.run.kind, RunKind::Maintenance));
        let maint_failed = last_maintenance_run
            .as_ref()
            .is_some_and(|r| r.status == RunStatus::Error);
        let has_problem = !avail.reachable || password_rejected || has_stale_lock || maint_failed;
        let maintenance_pending_reason = (!maintenance_running
            && !is_pause_active(global_pause_raw, now))
        .then(|| self.maintenance_pending_code(&m_eval, avail.reachable, dest_busy))
        .flatten();
        let maintenance_retry_at_unix = (maintenance_pending_reason
            == Some(PendingReasonCode::Backoff))
        .then(|| ds.maintenance_backoff_until.map(|t| t.unix_timestamp()))
        .flatten();

        DestinationSummaryDto {
            destination: dest.clone(),
            revision: revision_of(dest),
            backup_count,
            availability: avail,
            password_rejected,
            has_stale_lock,
            has_problem,
            next_prune_at_unix: m_eval.next_prune_at.map(|t| t.unix_timestamp()),
            next_check_at_unix: m_eval.next_check_at.map(|t| t.unix_timestamp()),
            last_prune_at_unix: ds.last_prune_at.map(|t| t.unix_timestamp()),
            last_check_at_unix: ds.last_check_at.map(|t| t.unix_timestamp()),
            last_maintenance_run,
            maintenance_running,
            pending_forget_count,
            stale_lock_count,
            maintenance_pending_reason,
            maintenance_retry_at_unix,
        }
    }

    /// Why due maintenance work waits, with the backup pending-reason codes:
    /// `destination_unreachable`, `on_battery`, `metered`, `queue`, `backoff`.
    fn maintenance_pending_code(
        &self,
        m_eval: &kestic_core::scheduler::DestinationMaintenanceNeed,
        reachable: bool,
        dest_busy: bool,
    ) -> Option<PendingReasonCode> {
        // An open `forget` alone is not due maintenance work.
        let any_work = m_eval.prune_due || m_eval.check_due;
        if !any_work || m_eval.due_now {
            None
        } else if !reachable {
            Some(PendingReasonCode::DestinationUnreachable)
        } else if self.system_env.on_battery {
            Some(PendingReasonCode::OnBattery)
        } else if self.system_env.network_metered {
            Some(PendingReasonCode::Metered)
        } else if dest_busy {
            Some(PendingReasonCode::Queue)
        } else if m_eval.in_backoff {
            Some(PendingReasonCode::Backoff)
        } else {
            None
        }
    }

    fn build_tray_state(&self, input: TrayInput<'_>) -> KesticTrayState {
        let TrayInput {
            backups,
            overall,
            blocking,
            restic_status,
            global_pause,
            paused_backup_window,
            now,
        } = input;
        let tray_backups = backups
            .iter()
            .map(|b| {
                let is_running_dest = self
                    .active_runs
                    .values()
                    .any(|r| r.run.destination_id == b.backup.destination_id && !r.is_restore());
                let label = match b.availability.blocked_reason {
                    Some(reason) if !b.availability.available => {
                        let reason = tr(&reason.message(b.availability.source_status));
                        format!("{} ({reason})", b.backup.name)
                    }
                    _ => b.backup.name.clone(),
                };
                TrayBackupItem {
                    backup_id: b.backup.id.as_str().to_string(),
                    label,
                    enabled: b.availability.available && !is_running_dest,
                }
            })
            .collect();

        let tray_paused_line = match (self.simulated_status, global_pause) {
            (Some(_), _) if overall.paused => Some(tr(&schedules_paused_message())),
            (_, Some(p)) => Some(self.format_pause_label(p.paused_since, p.paused_until, now)),
            _ => paused_backup_window.map(|(ps, pu)| self.format_pause_label(ps, pu, now)),
        };
        let tray_next_backup_line = overall
            .next_scheduled_backup_name
            .as_ref()
            .zip(overall.next_scheduled_at_unix)
            .and_then(|(name, ts)| {
                OffsetDateTime::from_unix_timestamp(ts).ok().map(|dt| {
                    let when = format_local_slot_hint(dt, now, &self.timezone);
                    i18n("Next backup: %1 (%2)").subs(&[name, &when])
                })
            });
        let headline = self
            .simulated_status
            .and_then(simulated_headline)
            .unwrap_or_else(|| {
                let active = overall.active_run_info();
                tr(&overall_headline(
                    &overall.counts(backups.len()),
                    active.as_ref(),
                    blocking_message(blocking, restic_status),
                ))
            });

        KesticTrayState {
            status: overall.tray_status,
            icon_name: "kestic-symbolic".to_string(),
            icon_theme_path: self.icon_theme_path.clone(),
            overlay_icon_name: overall.overlay_icon.clone(),
            tooltip_title: "Kestic".to_string(),
            tooltip_description: headline.clone(),
            status_line: headline,
            running_run: overall.active_run_progress.clone(),
            paused_line: tray_paused_line,
            next_backup_line: tray_next_backup_line,
            global_paused: global_pause.is_some(),
            backups: tray_backups,
            blocking_active: blocking.is_some(),
            has_destinations: !self.config.destinations.is_empty(),
        }
    }

    fn compute_overall_status(
        &self,
        backups: &[BackupSummaryDto],
        destinations: &[DestinationSummaryDto],
        blocking: Option<&BlockingStateDto>,
        global_pause: Option<&PauseState>,
        paused_backup_window: Option<(OffsetDateTime, Option<OffsetDateTime>)>,
        now: OffsetDateTime,
    ) -> OverallStatusDto {
        // A backup or maintenance comes first: «Cancel Running Backup» in the tray acts on it.
        let active_run_progress = self
            .active_runs
            .values()
            .min_by_key(|e| e.is_restore())
            .map(|e| e.progress.clone());

        if let Some(sim) = self.simulated_status {
            return self.compute_simulated_overall_status(sim, active_run_progress);
        }

        let counts =
            collect_overall_counts(backups, self.active_runs.len(), global_pause.is_some());
        let worst_kind = select_worst_overall_kind(blocking, backups.is_empty(), &counts);
        let (tray_status, overlay) =
            compute_tray_presentation(worst_kind, counts.running_count > 0, counts.any_paused);

        let paused_since_unix = global_pause
            .map(|p| p.paused_since.unix_timestamp())
            .or_else(|| paused_backup_window.map(|(ps, _)| ps.unix_timestamp()));
        let paused_until_unix = global_pause
            .and_then(|p| p.paused_until.map(|u| u.unix_timestamp()))
            .or_else(|| paused_backup_window.and_then(|(_, pu)| pu.map(|u| u.unix_timestamp())));

        let earliest_next = backups
            .iter()
            .filter(|b| !b.is_paused)
            .filter_map(|b| {
                b.next_scheduled_at_unix
                    .map(|ts| (ts, b.backup.name.clone()))
            })
            .min_by_key(|(ts, _)| *ts);
        let (next_scheduled_at_unix, next_scheduled_backup_name) = match earliest_next {
            Some((ts, name)) => (Some(ts), Some(name)),
            None => (None, None),
        };

        OverallStatusDto {
            kind: worst_kind,
            tray_status,
            overlay_icon: overlay.to_string(),
            failed_count: counts.failed_count,
            warning_count: counts.warning_count,
            overdue_count: counts.overdue_count,
            running_count: counts.running_count,
            waiting_count: counts.waiting_count,
            paused: counts.global_paused,
            now_unix: now.unix_timestamp(),
            paused_since_unix,
            paused_until_unix,
            next_scheduled_backup_name,
            next_scheduled_at_unix,
            active_run_progress,
            backups_have_attention: counts.failed_count > 0 || counts.overdue_count > 0,
            destinations_have_attention: destinations.iter().any(|d| d.has_problem),
        }
    }

    fn compute_simulated_overall_status(
        &self,
        sim: SimulatedOverallStatus,
        active_run_progress: Option<ActiveRunProgressDto>,
    ) -> OverallStatusDto {
        let (kind, is_running, is_paused) = match sim {
            SimulatedOverallStatus::Overdue => (OverallStatusKind::Overdue, false, false),
            SimulatedOverallStatus::Paused => (OverallStatusKind::Paused, false, true),
            SimulatedOverallStatus::Waiting => (OverallStatusKind::Pending, false, false),
            SimulatedOverallStatus::PausedAndWaiting => (OverallStatusKind::Pending, false, true),
            SimulatedOverallStatus::MaintenanceRunning | SimulatedOverallStatus::RestoreRunning => {
                (OverallStatusKind::Running, true, false)
            }
        };
        let (tray_status, overlay) = compute_tray_presentation(kind, is_running, is_paused);
        OverallStatusDto {
            kind,
            tray_status,
            overlay_icon: overlay.to_string(),
            failed_count: 0,
            warning_count: 0,
            overdue_count: usize::from(matches!(sim, SimulatedOverallStatus::Overdue)),
            running_count: usize::from(is_running),
            waiting_count: usize::from(matches!(
                sim,
                SimulatedOverallStatus::Waiting | SimulatedOverallStatus::PausedAndWaiting
            )),
            paused: is_paused,
            now_unix: self.clock.now_unix_secs(),
            paused_since_unix: None,
            paused_until_unix: None,
            next_scheduled_backup_name: None,
            next_scheduled_at_unix: None,
            active_run_progress,
            backups_have_attention: matches!(sim, SimulatedOverallStatus::Overdue),
            destinations_have_attention: false,
        }
    }
}

fn collect_overall_counts(
    backups: &[BackupSummaryDto],
    running_count: usize,
    global_paused: bool,
) -> OverallStatusCounts {
    OverallStatusCounts {
        failed_count: backups.iter().filter(|b| b.has_error).count(),
        overdue_count: backups.iter().filter(|b| b.is_overdue).count(),
        warning_count: backups
            .iter()
            .filter(|b| {
                b.last_run
                    .as_ref()
                    .is_some_and(|r| r.status == RunStatus::Warning)
            })
            .count(),
        running_count,
        waiting_count: backups
            .iter()
            .filter(|b| {
                b.pending_primary_code.is_some()
                    || (b.active_run.is_none()
                        && !b.is_paused
                        && !b.has_error
                        && b.last_run
                            .as_ref()
                            .is_some_and(|r| r.status == RunStatus::Pending))
            })
            .count(),
        global_paused,
        any_paused: global_paused || backups.iter().any(|b| b.is_paused),
        all_paused: !backups.is_empty() && backups.iter().all(|b| b.is_paused),
        has_interrupted: backups.iter().any(|b| {
            b.last_run
                .as_ref()
                .is_some_and(|r| r.status == RunStatus::Interrupted)
        }),
        has_cancelled: backups.iter().any(|b| {
            b.last_run
                .as_ref()
                .is_some_and(|r| r.status == RunStatus::Cancelled)
        }),
        has_never_run: backups
            .iter()
            .any(|b| b.active_run.is_none() && b.last_run.is_none()),
        all_manual: !backups.is_empty()
            && backups
                .iter()
                .all(|b| matches!(b.backup.schedule, Schedule::ManualOnly)),
    }
}

fn select_worst_overall_kind(
    blocking: Option<&BlockingStateDto>,
    no_backups: bool,
    c: &OverallStatusCounts,
) -> OverallStatusKind {
    if blocking.is_some() {
        OverallStatusKind::Blocked
    } else if c.failed_count > 0 {
        OverallStatusKind::Error
    } else if c.overdue_count > 0 {
        OverallStatusKind::Overdue
    } else if c.warning_count > 0 {
        OverallStatusKind::Warning
    } else if c.running_count > 0 {
        OverallStatusKind::Running
    } else if c.waiting_count > 0 {
        OverallStatusKind::Pending
    } else if c.global_paused || c.all_paused {
        OverallStatusKind::Paused
    } else if c.has_interrupted {
        OverallStatusKind::Interrupted
    } else if c.has_cancelled {
        OverallStatusKind::Cancelled
    } else if c.has_never_run || no_backups {
        OverallStatusKind::NeverRun
    } else if c.all_manual {
        OverallStatusKind::ManualOnly
    } else {
        OverallStatusKind::Ok
    }
}

/// Tray headline of the simulated running states, which have no counts behind them (the
/// other simulated states use the regular headline).
fn simulated_headline(sim: SimulatedOverallStatus) -> Option<String> {
    match sim {
        SimulatedOverallStatus::MaintenanceRunning => Some(tr(&maintenance_running_message())),
        SimulatedOverallStatus::RestoreRunning => Some(i18n("Restore running").into()),
        _ => None,
    }
}

/// The code of a run kind for progress DTOs.
pub(super) fn run_kind_code(kind: &RunKind) -> RunKindCode {
    match kind {
        RunKind::Backup { .. } => RunKindCode::Backup,
        RunKind::Maintenance => RunKindCode::Maintenance,
        RunKind::Restore { .. } => RunKindCode::Restore,
        RunKind::Meta => RunKindCode::Meta,
    }
}
