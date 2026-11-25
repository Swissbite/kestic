// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Backup CRUD, run details, and source/exclude helpers.

use super::error::parse_backup_id;
use super::error::{KesticServiceErr, map_config_err};
use super::restore_runner::restore_error_kind;
use super::run_errors::{error_run_kind, unreadable_items_from_lines};
use super::state::DaemonCore;
use crate::dbus_api::{ExcludeRulePreviewDto, RunDetailDto};
use crate::i18n::tr;
use kestic_core::backup::{exclude_file_path, write_exclude_file};
use kestic_core::config::ConfigError;
use kestic_core::model::{
    Backup, BackupId, Destination, DestinationId, ExcludeLintWarning, ExcludePathTestResult,
    ExcludeRules, NestedSourceConflict, ResticProvider, Run, RunId, RunKind, RunStatus, Schedule,
    Source, SourceKind, check_nested_sources_for_candidate, evaluate_exclude_path,
    lint_exclude_rules, normalize_folder_rule,
};
use kestic_core::mount::{current_home_dir, detect_source_with};
use kestic_core::provider::resolve_and_check;
use kestic_core::restic::{ResticCommandSpec, classify_exit, parse_recorded_invocation};
use kestic_core::sync::LockUnpoisoned;
use kestic_i18n::codes::ErrorKind;
use kestic_i18n::messages::deleted_destination_message;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

impl DaemonCore {
    pub fn add_backup(&mut self, backup_json: &str) -> Result<String, KesticServiceErr> {
        let mut backup: Backup = serde_json::from_str(backup_json)
            .map_err(|e| KesticServiceErr::InvalidInput(format!("invalid backup JSON: {e}")))?;
        if self.config.backup(&backup.id).is_some() {
            backup.id = BackupId::generate_unique(self.config.backups.iter().map(|b| &b.id))
                .map_err(|e| KesticServiceErr::Internal(e.to_string()))?;
        }
        let home = current_home_dir();
        backup.excludes.normalize_folders(home.as_deref());

        let detected = detect_source_with(
            &backup.source.path,
            home.as_deref(),
            self.mount_inspector.as_ref(),
        );
        if backup.source.kind == SourceKind::Folder && detected.kind == SourceKind::Drive {
            backup.source = detected;
        } else if backup.source.kind == SourceKind::Drive
            && backup.source.identity.is_none()
            && detected.kind == SourceKind::Drive
        {
            backup.source.identity = detected.identity;
            if backup.source.drive_label.is_none() {
                backup.source.drive_label = detected.drive_label;
            }
        }

        backup
            .validate()
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;

        let new_id = backup.id.as_str().to_string();
        let backup_for_file = backup.clone();
        let mut next = self.config.clone();
        next.backups.push(backup);
        self.persist_config(next)?;
        self.ensure_baselines();
        write_exclude_file(&self.state_dir, &backup_for_file)
            .map_err(|e| KesticServiceErr::Internal(e.to_string()))?;
        Ok(new_id)
    }

    pub fn update_backup(&mut self, backup_json: &str) -> Result<(), KesticServiceErr> {
        let mut updated: Backup = serde_json::from_str(backup_json)
            .map_err(|e| KesticServiceErr::InvalidInput(format!("invalid backup JSON: {e}")))?;
        let home = current_home_dir();
        updated.excludes.normalize_folders(home.as_deref());

        let idx = self
            .config
            .backups
            .iter()
            .position(|b| b.id == updated.id)
            .ok_or_else(|| KesticServiceErr::NotFound(updated.id.as_str().to_string()))?;

        let existing = &self.config.backups[idx];
        let old_schedule = existing.schedule.clone();
        let detected = detect_source_with(
            &updated.source.path,
            home.as_deref(),
            self.mount_inspector.as_ref(),
        );
        if updated.source.kind == SourceKind::Folder && detected.kind == SourceKind::Drive {
            updated.source = detected;
        } else if updated.source.kind == SourceKind::Drive
            && (updated.source.identity.is_none() || updated.source.path != existing.source.path)
            && detected.kind == SourceKind::Drive
        {
            updated.source.identity = detected.identity;
            if updated.source.drive_label.is_none() || updated.source.path != existing.source.path {
                updated.source.drive_label = detected.drive_label;
            }
        }

        updated
            .validate()
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;

        let updated_for_file = updated.clone();
        let updated_schedule = updated.schedule.clone();
        let mut next = self.config.clone();
        next.backups[idx] = updated;
        self.persist_config(next)?;
        write_exclude_file(&self.state_dir, &updated_for_file)
            .map_err(|e| KesticServiceErr::Internal(e.to_string()))?;

        if old_schedule != updated_schedule {
            let now = self.clock.now_utc();
            let _ = self.delete_pending_runs_cached(&updated_for_file.id);
            let mut rs = self.load_backup_runtime_state(&updated_for_file.id);
            if matches!(updated_schedule, Schedule::ManualOnly) {
                kestic_core::scheduler::clear_backup_backoff(&mut rs);
                rs.last_overdue_notified_at = None;
                self.queued_backups.retain(|id| id != &updated_for_file.id);
            } else {
                kestic_core::scheduler::anchor_backup_schedule_at(&mut rs, now);
            }
            let _ = self.store_backup_runtime_state(&updated_for_file.id, &rs);
        }
        Ok(())
    }

    pub fn rename_backup(
        &mut self,
        backup_id: &str,
        new_name: &str,
    ) -> Result<(), KesticServiceErr> {
        let id = parse_backup_id(backup_id)?;
        let mut next = self.config.clone();
        let backup = next
            .backups
            .iter_mut()
            .find(|b| b.id == id)
            .ok_or_else(|| KesticServiceErr::NotFound(backup_id.to_string()))?;
        backup
            .rename(new_name)
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        self.persist_config(next)?;
        Ok(())
    }

    pub fn reassign_backup(
        &mut self,
        backup_id: &str,
        new_dest_id: &str,
    ) -> Result<(), KesticServiceErr> {
        let b_id = parse_backup_id(backup_id)?;
        let d_id = DestinationId::parse(new_dest_id)
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        let mut next = self.config.clone();
        next.reassign_backup(&b_id, &d_id)
            .map_err(|err| match err {
                ConfigError::Validation(msg) if msg.contains("not found") => {
                    KesticServiceErr::NotFound(msg)
                }
                other => map_config_err(other),
            })?;
        self.persist_config(next)?;
        Ok(())
    }

    pub fn remove_backup(&mut self, backup_id: &str) -> Result<(), KesticServiceErr> {
        let id = parse_backup_id(backup_id)?;
        let mut next = self.config.clone();
        next.remove_backup(&id).map_err(|err| match err {
            ConfigError::Validation(msg) if msg.contains("not found") => {
                KesticServiceErr::NotFound(msg)
            }
            other => map_config_err(other),
        })?;
        self.persist_config(next)?;
        self.source_availability.remove(&id);
        self.queued_backups.retain(|q| q != &id);
        self.runs_cache.retain(|r| r.backup_id() != Some(&id));
        let _ = fs::remove_file(exclude_file_path(&self.state_dir, &id));
        if let Err(err) = self.clear_backup_state_cached(&id) {
            eprintln!("kestic-daemon: failed to clear history state for backup {id}: {err}");
        }
        Ok(())
    }

    /// Collects everything a run detail needs under the `DaemonCore` lock (no file I/O, no
    /// `restic version`); see [`get_run_detail_unlocked`].
    fn run_detail_snapshot(&self, run_id: &str) -> Result<RunDetailSnapshot, KesticServiceErr> {
        let id = RunId::parse(run_id).map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        let run = if let Some(active) = self.active_runs.get(&id) {
            active.run.clone()
        } else {
            self.history
                .get_run(&id)
                .map_err(|e| KesticServiceErr::Internal(e.to_string()))?
                .ok_or_else(|| KesticServiceErr::NotFound(run_id.to_string()))?
        };
        let backup = run
            .backup_id()
            .and_then(|bid| self.config.backup(bid).cloned());
        let destination = self.config.destination(&run.destination_id).cloned();
        Ok(RunDetailSnapshot {
            backup_name: backup.as_ref().map(|b| b.name.clone()),
            destination_name: destination
                .as_ref()
                .map_or_else(|| tr(&deleted_destination_message()), |d| d.name.clone()),
            run,
            backup,
            destination,
            provider: self.config.restic_provider.clone(),
            kestic_binary: self.kestic_binary.clone(),
            state_dir: self.state_dir.clone(),
        })
    }

    pub fn detect_source(&self, path: &str) -> Result<Source, KesticServiceErr> {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            return Err(KesticServiceErr::InvalidInput(
                "source path must not be empty".to_string(),
            ));
        }
        let home = current_home_dir();
        Ok(detect_source_with(
            std::path::Path::new(trimmed),
            home.as_deref(),
            self.mount_inspector.as_ref(),
        ))
    }

    pub fn check_nested_sources(
        &self,
        candidate_id: &str,
        candidate_path: &str,
        destination_id: &str,
    ) -> Result<Vec<NestedSourceConflict>, KesticServiceErr> {
        if candidate_path.trim().is_empty() || destination_id.trim().is_empty() {
            return Ok(Vec::new());
        }
        let d_id = DestinationId::parse(destination_id.trim())
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        let c_id = if candidate_id.trim().is_empty() {
            None
        } else {
            Some(parse_backup_id(candidate_id.trim())?)
        };
        Ok(check_nested_sources_for_candidate(
            c_id.as_ref(),
            std::path::Path::new(candidate_path.trim()),
            &d_id,
            &self.config.backups,
        ))
    }

    pub fn lint_excludes(
        &self,
        excludes_json: &str,
    ) -> Result<Vec<ExcludeLintWarning>, KesticServiceErr> {
        let rules: ExcludeRules = serde_json::from_str(excludes_json)
            .map_err(|e| KesticServiceErr::InvalidInput(format!("invalid excludes JSON: {e}")))?;
        Ok(lint_exclude_rules(&rules))
    }

    pub fn test_exclude_path(
        &self,
        backup_id: &str,
        excludes_json: &str,
        test_path: &str,
    ) -> Result<ExcludePathTestResult, KesticServiceErr> {
        let b_id = parse_backup_id(backup_id)?;
        let backup = self
            .config
            .backup(&b_id)
            .ok_or_else(|| KesticServiceErr::NotFound(backup_id.to_string()))?;
        let rules: ExcludeRules = serde_json::from_str(excludes_json)
            .map_err(|e| KesticServiceErr::InvalidInput(format!("invalid excludes JSON: {e}")))?;
        let home = current_home_dir();
        Ok(evaluate_exclude_path(
            &rules,
            &backup.source.path,
            home.as_deref(),
            test_path,
        ))
    }

    pub fn format_exclude_rule_for_item(
        &self,
        item_path: &str,
    ) -> Result<ExcludeRulePreviewDto, KesticServiceErr> {
        let trimmed = item_path.trim();
        if trimmed.is_empty() {
            return Err(KesticServiceErr::InvalidInput(
                "item path must not be empty".to_string(),
            ));
        }
        let is_dir = fs::metadata(trimmed).is_ok_and(|m| m.is_dir());
        if is_dir {
            let rule = normalize_folder_rule(trimmed, current_home_dir().as_deref());
            Ok(ExcludeRulePreviewDto {
                is_dir: true,
                rule,
                is_expert_rule: false,
            })
        } else {
            Ok(ExcludeRulePreviewDto {
                is_dir: false,
                rule: trimmed.to_string(),
                is_expert_rule: true,
            })
        }
    }

    pub fn add_exclude_rule_for_item(
        &mut self,
        backup_id: &str,
        item_path: &str,
    ) -> Result<ExcludeRulePreviewDto, KesticServiceErr> {
        let preview = self.format_exclude_rule_for_item(item_path)?;
        let b_id = parse_backup_id(backup_id)?;
        let idx = self
            .config
            .backups
            .iter()
            .position(|b| b.id == b_id)
            .ok_or_else(|| KesticServiceErr::NotFound(backup_id.to_string()))?;

        let mut next = self.config.clone();
        if preview.is_dir {
            if !next.backups[idx].excludes.folders.contains(&preview.rule) {
                next.backups[idx]
                    .excludes
                    .folders
                    .push(preview.rule.clone());
            }
        } else if !next.backups[idx].excludes.lines.contains(&preview.rule) {
            next.backups[idx].excludes.lines.push(preview.rule.clone());
        }
        next.backups[idx]
            .validate()
            .map_err(|e| KesticServiceErr::InvalidInput(e.to_string()))?;
        let updated_for_file = next.backups[idx].clone();
        self.persist_config(next)?;
        write_exclude_file(&self.state_dir, &updated_for_file)
            .map_err(|e| KesticServiceErr::Internal(e.to_string()))?;
        Ok(preview)
    }
}

/// Run detail inputs captured under the `DaemonCore` lock.
struct RunDetailSnapshot {
    run: Run,
    backup_name: Option<String>,
    destination_name: String,
    backup: Option<Backup>,
    destination: Option<Destination>,
    provider: ResticProvider,
    kestic_binary: PathBuf,
    state_dir: PathBuf,
}

/// `GetRunDetail`: snapshots the run under the lock, then reads the log file and resolves
/// restic (transparent command) without holding the `DaemonCore` lock.
pub(crate) fn get_run_detail_unlocked(
    shared: &Arc<Mutex<DaemonCore>>,
    run_id: &str,
) -> Result<RunDetailDto, KesticServiceErr> {
    let snap = {
        let guard = shared.lock_unpoisoned();
        guard.run_detail_snapshot(run_id)?
    };
    let log_lines: Vec<String> = match &snap.run.log_path {
        Some(path) => {
            fs::read_to_string(path).map_or_default(|s| s.lines().map(ToOwned::to_owned).collect())
        }
        None => Vec::new(),
    };
    let transparent_commands = build_transparent_commands(&snap);
    let run = snap.run;

    let mut error_action = None;
    let mut error_kind = None;
    let mut error_raw_detail = run.summary_message.clone();
    let mut unreadable_files = Vec::new();

    if matches!(run.kind, RunKind::Restore { .. }) {
        error_kind = restore_error_kind(&run, &log_lines);
    } else if matches!(run.status, RunStatus::Error | RunStatus::Warning) {
        unreadable_files = unreadable_items_from_lines(log_lines.iter().map(String::as_str));
        if run.status == RunStatus::Error
            || run.exit_code == Some(3)
            || !unreadable_files.is_empty()
        {
            let pres = classify_exit(
                run.exit_code.unwrap_or(1),
                run.summary_message.as_deref(),
                unreadable_files.clone(),
                "",
            );
            error_action = pres.action;
            error_kind = Some(if run.status == RunStatus::Error {
                error_run_kind(&run)
            } else {
                ErrorKind::IncompleteSnapshot
            });
            if error_raw_detail.is_none() {
                error_raw_detail = pres.raw_detail;
            }
        } else {
            error_kind = Some(ErrorKind::RetentionPending);
        }
    }

    Ok(RunDetailDto {
        error_exit_code: error_kind.as_ref().and(run.exit_code),
        run,
        backup_name: snap.backup_name,
        destination_name: snap.destination_name,
        log_lines,
        transparent_commands,
        error_action,
        error_raw_detail,
        unreadable_files,
        error_kind,
    })
}

fn build_transparent_commands(snap: &RunDetailSnapshot) -> Vec<String> {
    let run = &snap.run;
    if !run.commands.is_empty() {
        return run
            .commands
            .iter()
            .map(|c| {
                if let Some(dest) = &snap.destination {
                    let inv = parse_recorded_invocation(&c.executable, &c.args);
                    let mut spec = ResticCommandSpec::for_destination(
                        &inv.restic_binary,
                        &snap.kestic_binary,
                        dest,
                        inv.subcommand_args,
                    );
                    spec.low_priority = inv.low_priority;
                    spec.json_output = inv.json_output;
                    spec.to_transparent_shell_command()
                } else {
                    format!("{} {}", c.executable, c.args.join(" "))
                }
            })
            .collect();
    }
    if let (Some(backup), Some(dest)) = (&snap.backup, &snap.destination)
        && let Ok(resolved) = resolve_and_check(&snap.provider)
    {
        return kestic_core::backup::build_backup_command_spec(
            backup,
            dest,
            &snap.state_dir,
            &resolved,
            &snap.kestic_binary,
        )
        .map_or_default(|s| vec![s.to_transparent_shell_command()]);
    }
    Vec::new()
}
