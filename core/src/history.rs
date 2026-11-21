// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! SQLite run history, redacted per-run logs, startup recovery, and log retention.

use crate::clock::Clock;
use crate::migrations::{MigrationError, history_migrations, run_migrations};
use crate::model::{
    BackupId, CommandRecord, DestinationId, MaintenanceStats, PendingReason, Run, RunId, RunKind,
    RunStats, RunStatus,
};
use crate::restic::redact_secrets;
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use thiserror::Error;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// Retention duration after which old logs beyond the newest 100 runs are pruned.
pub const LOG_RETENTION_DAYS: i64 = 90;
/// Minimum number of latest runs per backup whose logs are always kept.
pub const LOG_KEEP_MIN_RUNS: usize = 100;

/// Errors returned by [`RunHistory`].
#[derive(Debug, Error)]
pub enum HistoryError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("database schema migration error: {0}")]
    Migration(#[from] MigrationError),
    #[error("JSON serialization error in run history: {0}")]
    Json(#[from] serde_json::Error),
    #[error("timestamp formatting/parsing error: {0}")]
    Time(String),
    #[error("model validation error in run history: {0}")]
    Model(#[from] crate::model::ModelError),
    #[error("I/O error in run history: {0}")]
    Io(#[from] std::io::Error),
}

/// Persistent SQLite-backed run history and log store (`glossary: RunHistory`, `$XDG_STATE_HOME/kestic/`).
#[derive(Debug)]
pub struct RunHistory {
    state_dir: PathBuf,
    conn: Connection,
}

/// Returns the timestamp of the latest `Ok` or `Warning` backup run for `backup_id`.
///
/// `runs` must be ordered newest first, as returned by [`RunHistory::list_runs`].
pub fn last_successful_backup_at(runs: &[Run], backup_id: &BackupId) -> Option<OffsetDateTime> {
    runs.iter()
        .find(|r| {
            r.backup_id() == Some(backup_id)
                && matches!(r.kind, RunKind::Backup { .. })
                && matches!(r.status, RunStatus::Ok | RunStatus::Warning)
        })
        .map(|r| r.finished_at.unwrap_or(r.started_at))
}

impl RunHistory {
    /// Opens (or initializes) `<state_dir>/history.db` and ensures `<state_dir>/logs` exists.
    pub fn open(state_dir: &Path) -> Result<Self, HistoryError> {
        fs::create_dir_all(state_dir)?;
        fs::create_dir_all(state_dir.join("logs"))?;
        let db_path = state_dir.join("history.db");
        let mut conn = Connection::open(db_path)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        run_migrations(&mut conn, history_migrations())?;
        Ok(Self {
            state_dir: state_dir.to_path_buf(),
            conn,
        })
    }

    /// Returns the state directory backing this history store.
    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    /// Writes redacted log lines for `run_id` into `<state_dir>/logs/<run_id>.log`.
    pub fn write_redacted_log(
        &self,
        run_id: &RunId,
        lines: &[String],
        secrets: &[&str],
    ) -> Result<PathBuf, HistoryError> {
        let log_dir = self.state_dir.join("logs");
        fs::create_dir_all(&log_dir)?;
        let log_path = log_dir.join(format!("{run_id}.log"));
        let mut f = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&log_path)?;
        fs::set_permissions(&log_path, fs::Permissions::from_mode(0o600))?;
        for line in lines {
            let safe = redact_secrets(line, secrets);
            f.write_all(safe.as_bytes())?;
            f.write_all(b"\n")?;
        }
        f.sync_all()?;
        Ok(log_path)
    }

    /// Inserts or updates a [`Run`] and its executed [`CommandRecord`] list.
    pub fn upsert_run(&mut self, run: &Run, secrets: &[&str]) -> Result<(), HistoryError> {
        let (kind_str, backup_id_str) = match &run.kind {
            RunKind::Backup { backup_id } => ("backup", Some(backup_id.as_str())),
            RunKind::Maintenance => ("maintenance", None),
            RunKind::Restore { backup_id } => ("restore", backup_id.as_ref().map(|b| b.as_str())),
            RunKind::Meta => ("meta", None),
        };

        let started_rfc = run
            .started_at
            .format(&Rfc3339)
            .map_err(|e| HistoryError::Time(e.to_string()))?;
        let finished_rfc = run
            .finished_at
            .map(|t| t.format(&Rfc3339))
            .transpose()
            .map_err(|e| HistoryError::Time(e.to_string()))?;

        let summary_redacted = run
            .summary_message
            .as_deref()
            .map(|s| redact_secrets(s, secrets));
        let stats_json = encode_stats_json(run)?;
        let log_path_str = run
            .log_path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned());

        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO runs (
                id, kind, backup_id, destination_id, status, pending_reason,
                started_at_unix, started_at, finished_at, duration_ms, exit_code,
                summary_message, stats_json, log_path
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
            ON CONFLICT(id) DO UPDATE SET
                kind = excluded.kind,
                backup_id = excluded.backup_id,
                destination_id = excluded.destination_id,
                status = excluded.status,
                pending_reason = excluded.pending_reason,
                started_at_unix = excluded.started_at_unix,
                started_at = excluded.started_at,
                finished_at = excluded.finished_at,
                duration_ms = excluded.duration_ms,
                exit_code = excluded.exit_code,
                summary_message = excluded.summary_message,
                stats_json = excluded.stats_json,
                log_path = excluded.log_path",
            params![
                run.id.as_str(),
                kind_str,
                backup_id_str,
                run.destination_id.as_str(),
                run.status.wire_name(),
                run.pending_reason.map(PendingReason::wire_name),
                run.started_at.unix_timestamp(),
                started_rfc,
                finished_rfc,
                run.duration_ms.map(to_sql_int).transpose()?,
                run.exit_code,
                summary_redacted,
                stats_json,
                log_path_str,
            ],
        )?;

        tx.execute(
            "DELETE FROM run_commands WHERE run_id = ?1",
            params![run.id.as_str()],
        )?;
        for (idx, cmd) in run.commands.iter().enumerate() {
            let safe_args: Vec<String> = cmd
                .args
                .iter()
                .map(|a| redact_secrets(a, secrets))
                .collect();
            let args_json = serde_json::to_string(&safe_args)?;
            let env_keys_json = serde_json::to_string(&cmd.env_keys)?;
            let cmd_started = cmd
                .started_at
                .format(&Rfc3339)
                .map_err(|e| HistoryError::Time(e.to_string()))?;
            tx.execute(
                "INSERT INTO run_commands (
                    run_id, seq, executable, args_json, env_keys_json,
                    started_at, duration_ms, exit_code
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    run.id.as_str(),
                    to_sql_int(idx)?,
                    redact_secrets(&cmd.executable, secrets),
                    args_json,
                    env_keys_json,
                    cmd_started,
                    to_sql_int(cmd.duration_ms)?,
                    cmd.exit_code,
                ],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    /// Loads a single [`Run`] by [`RunId`].
    pub fn get_run(&self, id: &RunId) -> Result<Option<Run>, HistoryError> {
        let row = self
            .conn
            .query_row(
                "SELECT id, kind, backup_id, destination_id, status, pending_reason,
                        started_at, finished_at, duration_ms, exit_code,
                        summary_message, stats_json, log_path
                 FROM runs WHERE id = ?1",
                params![id.as_str()],
                read_run_row,
            )
            .optional()?;

        match row {
            Some(mut run) => {
                run.commands = self.load_commands(id)?;
                Ok(Some(run))
            }
            None => Ok(None),
        }
    }

    /// Lists all runs ordered newest first without loading child `commands`.
    pub fn list_runs_without_commands(&self) -> Result<Vec<Run>, HistoryError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, backup_id, destination_id, status, pending_reason,
                    started_at, finished_at, duration_ms, exit_code,
                    summary_message, stats_json, log_path
             FROM runs
             ORDER BY started_at_unix DESC, rowid DESC",
        )?;
        let rows = stmt.query_map([], read_run_row)?;
        let mut runs = Vec::new();
        for row in rows {
            runs.push(row?);
        }
        Ok(runs)
    }

    /// Lists all runs ordered newest first (batch-loads all `run_commands` in a single query).
    pub fn list_runs(&self) -> Result<Vec<Run>, HistoryError> {
        let mut runs = self.list_runs_without_commands()?;
        if runs.is_empty() {
            return Ok(runs);
        }
        let mut cmds_by_run = self.load_all_commands_grouped()?;
        for run in &mut runs {
            if let Some(cmds) = cmds_by_run.remove(run.id.as_str()) {
                run.commands = cmds;
            }
        }
        Ok(runs)
    }

    /// Marks any runs stuck in [`RunStatus::Running`] as [`RunStatus::Interrupted`] on daemon startup
    pub fn mark_interrupted_on_startup(&self, clock: &dyn Clock) -> Result<usize, HistoryError> {
        let now_rfc = clock
            .now_utc()
            .format(&Rfc3339)
            .map_err(|e| HistoryError::Time(e.to_string()))?;
        let changed = self.conn.execute(
            "UPDATE runs
             SET status = 'interrupted',
                 finished_at = COALESCE(finished_at, ?1)
             WHERE status = 'running'",
            params![now_rfc],
        )?;
        Ok(changed)
    }

    /// Deletes log files for `backup_id` that are BOTH older than 90 days AND not among the
    /// newest 100 runs of that backup. Returns number of deleted log files.
    pub fn prune_logs_for_backup(
        &mut self,
        backup_id: &BackupId,
        clock: &dyn Clock,
    ) -> Result<usize, HistoryError> {
        let cutoff = clock.now_utc() - time::Duration::days(LOG_RETENTION_DAYS);
        let cutoff_unix = cutoff.unix_timestamp();

        let mut stmt = self.conn.prepare(
            "SELECT id, started_at_unix, log_path
             FROM runs
             WHERE backup_id = ?1
             ORDER BY started_at_unix DESC, rowid DESC",
        )?;

        let rows: Vec<(String, i64, Option<String>)> = stmt
            .query_map(params![backup_id.as_str()], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(stmt);

        let keep_newest: HashSet<&str> = rows
            .iter()
            .take(LOG_KEEP_MIN_RUNS)
            .map(|(id, _, _)| id.as_str())
            .collect();

        let mut deleted_count = 0usize;
        let tx = self.conn.transaction()?;
        for (id, started_unix, log_path_opt) in &rows {
            if keep_newest.contains(id.as_str()) {
                continue;
            }
            if *started_unix >= cutoff_unix {
                continue;
            }
            if let Some(path_str) = log_path_opt {
                let path = Path::new(path_str);
                match fs::remove_file(path) {
                    Ok(()) => deleted_count += 1,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => deleted_count += 1,
                    Err(e) => return Err(e.into()),
                }
                tx.execute("UPDATE runs SET log_path = NULL WHERE id = ?1", params![id])?;
            }
        }
        tx.commit()?;
        Ok(deleted_count)
    }

    /// Sets or clears the persistent `pending_forget` flag for `backup_id`.
    pub fn set_pending_forget(
        &self,
        backup_id: &BackupId,
        pending: bool,
    ) -> Result<(), HistoryError> {
        let flag: i64 = if pending { 1 } else { 0 };
        self.conn.execute(
            "INSERT INTO backup_state (backup_id, pending_forget)
             VALUES (?1, ?2)
             ON CONFLICT(backup_id) DO UPDATE SET pending_forget = excluded.pending_forget",
            params![backup_id.as_str(), flag],
        )?;
        Ok(())
    }

    /// Returns whether `backup_id` currently has a pending `forget` («forget pending»).
    pub fn is_pending_forget(&self, backup_id: &BackupId) -> Result<bool, HistoryError> {
        let mut stmt = self
            .conn
            .prepare("SELECT pending_forget FROM backup_state WHERE backup_id = ?1")?;
        let mut rows = stmt.query(params![backup_id.as_str()])?;
        if let Some(row) = rows.next()? {
            let val: i64 = row.get(0)?;
            Ok(val != 0)
        } else {
            Ok(false)
        }
    }

    /// Removes per-backup state row when a backup is deleted.
    pub fn clear_backup_state(&self, backup_id: &BackupId) -> Result<(), HistoryError> {
        self.conn.execute(
            "DELETE FROM backup_state WHERE backup_id = ?1",
            params![backup_id.as_str()],
        )?;
        Ok(())
    }

    /// Loads scheduling, pause, backoff, and overdue-notification state for `backup_id`.
    pub fn get_backup_runtime_state(
        &self,
        backup_id: &BackupId,
    ) -> Result<BackupRuntimeState, HistoryError> {
        let row = self
            .conn
            .query_row(
                "SELECT pending_forget, baseline_at_unix, paused_since_unix, paused_until_unix,
                        backoff_attempt, backoff_until_unix, last_overdue_notified_at_unix,
                        overdue_anchor_at_unix, last_failure_at_unix, backoff_kind
                 FROM backup_state WHERE backup_id = ?1",
                params![backup_id.as_str()],
                |r| {
                    let pf: i64 = r.get(0)?;
                    let baseline_at_unix: Option<i64> = r.get(1)?;
                    let paused_since_unix: Option<i64> = r.get(2)?;
                    let paused_until_unix: Option<i64> = r.get(3)?;
                    let backoff_attempt: i64 = r.get(4)?;
                    let backoff_until_unix: Option<i64> = r.get(5)?;
                    let last_overdue_notified_at_unix: Option<i64> = r.get(6)?;
                    let overdue_anchor_at_unix: Option<i64> = r.get(7)?;
                    let last_failure_at_unix: Option<i64> = r.get(8)?;
                    let backoff_kind_raw: Option<String> = r.get(9)?;
                    Ok(BackupRuntimeState {
                        pending_forget: pf != 0,
                        baseline_at: baseline_at_unix.and_then(unix_to_utc),
                        pause: paused_since_unix.map(|since| PauseState {
                            paused_since: OffsetDateTime::from_unix_timestamp(since)
                                .unwrap_or(OffsetDateTime::UNIX_EPOCH),
                            paused_until: paused_until_unix.and_then(unix_to_utc),
                        }),
                        backoff_attempt: from_sql_int(4, backoff_attempt)?,
                        backoff_until: backoff_until_unix.and_then(unix_to_utc),
                        backoff_kind: backoff_kind_raw
                            .as_deref()
                            .and_then(BackoffKind::from_db_str),
                        last_overdue_notified_at: last_overdue_notified_at_unix
                            .and_then(unix_to_utc),
                        overdue_anchor_at: overdue_anchor_at_unix.and_then(unix_to_utc),
                        last_failure_at: last_failure_at_unix.and_then(unix_to_utc),
                    })
                },
            )
            .optional()?;
        Ok(row.unwrap_or_default())
    }

    /// Persists scheduling, pause, backoff, and overdue-notification state for `backup_id`.
    pub fn upsert_backup_runtime_state(
        &self,
        backup_id: &BackupId,
        state: &BackupRuntimeState,
    ) -> Result<(), HistoryError> {
        self.conn.execute(
            "INSERT INTO backup_state (
                backup_id, pending_forget, baseline_at_unix,
                paused_since_unix, paused_until_unix,
                backoff_attempt, backoff_until_unix, backoff_kind, last_overdue_notified_at_unix,
                overdue_anchor_at_unix, last_failure_at_unix
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(backup_id) DO UPDATE SET
                pending_forget = excluded.pending_forget,
                baseline_at_unix = excluded.baseline_at_unix,
                paused_since_unix = excluded.paused_since_unix,
                paused_until_unix = excluded.paused_until_unix,
                backoff_attempt = excluded.backoff_attempt,
                backoff_until_unix = excluded.backoff_until_unix,
                backoff_kind = excluded.backoff_kind,
                last_overdue_notified_at_unix = excluded.last_overdue_notified_at_unix,
                overdue_anchor_at_unix = excluded.overdue_anchor_at_unix,
                last_failure_at_unix = excluded.last_failure_at_unix",
            params![
                backup_id.as_str(),
                if state.pending_forget { 1i64 } else { 0i64 },
                state.baseline_at.map(|t| t.unix_timestamp()),
                state
                    .pause
                    .as_ref()
                    .map(|p| p.paused_since.unix_timestamp()),
                state
                    .pause
                    .as_ref()
                    .and_then(|p| p.paused_until.map(|t| t.unix_timestamp())),
                state.backoff_attempt as i64,
                state.backoff_until.map(|t| t.unix_timestamp()),
                state.backoff_kind.map(BackoffKind::as_str),
                state.last_overdue_notified_at.map(|t| t.unix_timestamp()),
                state.overdue_anchor_at.map(|t| t.unix_timestamp()),
                state.last_failure_at.map(|t| t.unix_timestamp()),
            ],
        )?;
        Ok(())
    }

    /// Loads maintenance timestamps and backoff state for `destination_id`.
    pub fn get_destination_maintenance_state(
        &self,
        destination_id: &DestinationId,
    ) -> Result<DestinationMaintenanceState, HistoryError> {
        let row = self
            .conn
            .query_row(
                "SELECT baseline_at_unix, last_prune_at_unix, last_check_at_unix,
                        maintenance_backoff_attempt, maintenance_backoff_until_unix,
                        last_maintenance_failure_at_unix
                 FROM destination_state WHERE destination_id = ?1",
                params![destination_id.as_str()],
                |r| {
                    let baseline: Option<i64> = r.get(0)?;
                    let prune: Option<i64> = r.get(1)?;
                    let check: Option<i64> = r.get(2)?;
                    let maint_attempt: i64 = r.get(3)?;
                    let maint_until: Option<i64> = r.get(4)?;
                    let maint_failed: Option<i64> = r.get(5)?;
                    Ok(DestinationMaintenanceState {
                        baseline_at: baseline.and_then(unix_to_utc),
                        last_prune_at: prune.and_then(unix_to_utc),
                        last_check_at: check.and_then(unix_to_utc),
                        maintenance_backoff_attempt: from_sql_int(3, maint_attempt)?,
                        maintenance_backoff_until: maint_until.and_then(unix_to_utc),
                        last_maintenance_failure_at: maint_failed.and_then(unix_to_utc),
                    })
                },
            )
            .optional()?;
        Ok(row.unwrap_or_default())
    }

    /// Persists maintenance timestamps and backoff state for `destination_id`.
    pub fn upsert_destination_maintenance_state(
        &self,
        destination_id: &DestinationId,
        state: &DestinationMaintenanceState,
    ) -> Result<(), HistoryError> {
        self.conn.execute(
            "INSERT INTO destination_state (
                destination_id, baseline_at_unix, last_prune_at_unix, last_check_at_unix,
                maintenance_backoff_attempt, maintenance_backoff_until_unix,
                last_maintenance_failure_at_unix
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(destination_id) DO UPDATE SET
                baseline_at_unix = excluded.baseline_at_unix,
                last_prune_at_unix = excluded.last_prune_at_unix,
                last_check_at_unix = excluded.last_check_at_unix,
                maintenance_backoff_attempt = excluded.maintenance_backoff_attempt,
                maintenance_backoff_until_unix = excluded.maintenance_backoff_until_unix,
                last_maintenance_failure_at_unix = excluded.last_maintenance_failure_at_unix",
            params![
                destination_id.as_str(),
                state.baseline_at.map(|t| t.unix_timestamp()),
                state.last_prune_at.map(|t| t.unix_timestamp()),
                state.last_check_at.map(|t| t.unix_timestamp()),
                state.maintenance_backoff_attempt as i64,
                state.maintenance_backoff_until.map(|t| t.unix_timestamp()),
                state
                    .last_maintenance_failure_at
                    .map(|t| t.unix_timestamp()),
            ],
        )?;
        Ok(())
    }

    /// Removes per-destination state row when a destination is deleted.
    pub fn clear_destination_state(
        &self,
        destination_id: &DestinationId,
    ) -> Result<(), HistoryError> {
        self.conn.execute(
            "DELETE FROM destination_state WHERE destination_id = ?1",
            params![destination_id.as_str()],
        )?;
        Ok(())
    }

    /// Loads global schedule pause state.
    pub fn get_global_pause(&self) -> Result<Option<PauseState>, HistoryError> {
        let row = self
            .conn
            .query_row(
                "SELECT global_paused_since_unix, global_paused_until_unix
                 FROM app_state WHERE id = 1",
                [],
                |r| {
                    let since: Option<i64> = r.get(0)?;
                    let until: Option<i64> = r.get(1)?;
                    Ok(since.map(|s| PauseState {
                        paused_since: OffsetDateTime::from_unix_timestamp(s)
                            .unwrap_or(OffsetDateTime::UNIX_EPOCH),
                        paused_until: until.and_then(unix_to_utc),
                    }))
                },
            )
            .optional()?;
        Ok(row.flatten())
    }

    /// Persists or clears global schedule pause state.
    pub fn set_global_pause(&self, pause: Option<&PauseState>) -> Result<(), HistoryError> {
        self.conn.execute(
            "INSERT INTO app_state (id, global_paused_since_unix, global_paused_until_unix)
             VALUES (1, ?1, ?2)
             ON CONFLICT(id) DO UPDATE SET
                global_paused_since_unix = excluded.global_paused_since_unix,
                global_paused_until_unix = excluded.global_paused_until_unix",
            params![
                pause.map(|p| p.paused_since.unix_timestamp()),
                pause.and_then(|p| p.paused_until.map(|t| t.unix_timestamp())),
            ],
        )?;
        Ok(())
    }

    /// Deletes any leftover `Pending` rows for `backup_id` (e.g., when transitioning to `Running` or manual-only).
    pub fn delete_pending_runs_for_backup(&self, backup_id: &BackupId) -> Result<(), HistoryError> {
        self.conn.execute(
            "DELETE FROM runs WHERE backup_id = ?1 AND kind = 'backup' AND status = 'pending'",
            params![backup_id.as_str()],
        )?;
        Ok(())
    }

    fn load_commands(&self, run_id: &RunId) -> Result<Vec<CommandRecord>, HistoryError> {
        let mut stmt = self.conn.prepare(
            "SELECT executable, args_json, env_keys_json, started_at, duration_ms, exit_code
             FROM run_commands
             WHERE run_id = ?1
             ORDER BY seq ASC",
        )?;
        let rows = stmt.query_map(params![run_id.as_str()], |row| {
            let executable: String = row.get(0)?;
            let args_json: String = row.get(1)?;
            let env_keys_json: String = row.get(2)?;
            let started_at_raw: String = row.get(3)?;
            let duration_ms: i64 = row.get(4)?;
            let exit_code: i32 = row.get(5)?;
            Ok((
                executable,
                args_json,
                env_keys_json,
                started_at_raw,
                duration_ms,
                exit_code,
            ))
        })?;

        let mut commands = Vec::new();
        for item in rows {
            let (executable, args_json, env_keys_json, started_at_raw, duration_ms, exit_code) =
                item?;
            let args: Vec<String> = serde_json::from_str(&args_json)?;
            let env_keys: Vec<String> = serde_json::from_str(&env_keys_json)?;
            let started_at = OffsetDateTime::parse(&started_at_raw, &Rfc3339)
                .map_err(|e| HistoryError::Time(e.to_string()))?;
            commands.push(CommandRecord {
                executable,
                args,
                env_keys,
                started_at,
                duration_ms: from_sql_int(4, duration_ms)?,
                exit_code,
            });
        }
        Ok(commands)
    }

    fn load_all_commands_grouped(
        &self,
    ) -> Result<HashMap<String, Vec<CommandRecord>>, HistoryError> {
        let mut stmt = self.conn.prepare(
            "SELECT run_id, executable, args_json, env_keys_json, started_at, duration_ms, exit_code
             FROM run_commands
             ORDER BY run_id ASC, seq ASC",
        )?;
        let rows = stmt.query_map([], read_grouped_command_row)?;
        collect_grouped_commands(rows)
    }
}

type RawGroupedCommandRow = (String, String, String, String, String, i64, i32);

fn read_grouped_command_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawGroupedCommandRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
    ))
}

fn collect_grouped_commands<I>(rows: I) -> Result<HashMap<String, Vec<CommandRecord>>, HistoryError>
where
    I: Iterator<Item = rusqlite::Result<RawGroupedCommandRow>>,
{
    let mut map: HashMap<String, Vec<CommandRecord>> = HashMap::new();
    for item in rows {
        let (run_id, executable, args_json, env_keys_json, started_at_raw, duration_ms, exit_code) =
            item?;
        let args: Vec<String> = serde_json::from_str(&args_json)?;
        let env_keys: Vec<String> = serde_json::from_str(&env_keys_json)?;
        let started_at = OffsetDateTime::parse(&started_at_raw, &Rfc3339)
            .map_err(|e| HistoryError::Time(e.to_string()))?;
        map.entry(run_id).or_default().push(CommandRecord {
            executable,
            args,
            env_keys,
            started_at,
            duration_ms: from_sql_int(5, duration_ms)?,
            exit_code,
        });
    }
    Ok(map)
}

/// Persistent pause interval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PauseState {
    pub paused_since: OffsetDateTime,
    /// `None` means paused indefinitely ("bis ich fortsetze").
    pub paused_until: Option<OffsetDateTime>,
}

/// Cause of active transient backoff so network reconnects only reset network/unreachable backoffs
/// and never bypass Exit-11 repository lock backoffs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackoffKind {
    Network,
    Lock,
}

impl BackoffKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Network => "network",
            Self::Lock => "lock",
        }
    }

    pub fn from_db_str(raw: &str) -> Option<Self> {
        match raw {
            "network" => Some(Self::Network),
            "lock" => Some(Self::Lock),
            _ => None,
        }
    }
}

/// Persistent per-backup scheduling/pause/backoff state in `history.db`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BackupRuntimeState {
    pub pending_forget: bool,
    pub baseline_at: Option<OffsetDateTime>,
    pub pause: Option<PauseState>,
    pub backoff_attempt: u32,
    pub backoff_until: Option<OffsetDateTime>,
    pub backoff_kind: Option<BackoffKind>,
    pub last_overdue_notified_at: Option<OffsetDateTime>,
    pub overdue_anchor_at: Option<OffsetDateTime>,
    pub last_failure_at: Option<OffsetDateTime>,
}

/// Persistent per-destination maintenance timestamps & backoff state in `history.db`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DestinationMaintenanceState {
    pub baseline_at: Option<OffsetDateTime>,
    pub last_prune_at: Option<OffsetDateTime>,
    pub last_check_at: Option<OffsetDateTime>,
    pub maintenance_backoff_attempt: u32,
    pub maintenance_backoff_until: Option<OffsetDateTime>,
    pub last_maintenance_failure_at: Option<OffsetDateTime>,
}

fn unix_to_utc(ts: i64) -> Option<OffsetDateTime> {
    OffsetDateTime::from_unix_timestamp(ts).ok()
}

/// Converts a value for an SQLite `INTEGER` column, which holds a signed 64-bit number.
/// A value above `i64::MAX` cannot be stored, so the write fails instead of wrapping.
fn to_sql_int<T>(value: T) -> rusqlite::Result<i64>
where
    T: TryInto<i64> + Copy + std::fmt::Display,
{
    value.try_into().map_err(|_| {
        rusqlite::Error::ToSqlConversionFailure(
            format!("{value} exceeds the SQLite integer range").into(),
        )
    })
}

/// Reads an SQLite `INTEGER` from `column` into a narrower or unsigned type. A stored value
/// outside that range means the row is corrupted, so the read fails instead of clamping.
fn from_sql_int<T: TryFrom<i64>>(column: usize, value: i64) -> rusqlite::Result<T> {
    T::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(column, value))
}

/// The `stats_json` column holds [`MaintenanceStats`] for maintenance runs and [`RunStats`]
/// for every other run kind.
fn encode_stats_json(run: &Run) -> Result<Option<String>, serde_json::Error> {
    match run.kind {
        RunKind::Maintenance => run.maintenance.as_ref().map(serde_json::to_string),
        _ => run.stats.as_ref().map(serde_json::to_string),
    }
    .transpose()
}

fn decode_stats_json(
    kind: &RunKind,
    raw: Option<&str>,
) -> Result<(Option<RunStats>, Option<MaintenanceStats>), serde_json::Error> {
    let Some(raw) = raw else {
        return Ok((None, None));
    };
    match kind {
        RunKind::Maintenance => Ok((None, Some(serde_json::from_str(raw)?))),
        _ => Ok((Some(serde_json::from_str(raw)?), None)),
    }
}

fn read_run_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Run> {
    let id_raw: String = row.get(0)?;
    let kind_raw: String = row.get(1)?;
    let backup_id_raw: Option<String> = row.get(2)?;
    let dest_id_raw: String = row.get(3)?;
    let status_raw: String = row.get(4)?;
    let pending_raw: Option<String> = row.get(5)?;
    let started_raw: String = row.get(6)?;
    let finished_raw: Option<String> = row.get(7)?;
    let duration_ms: Option<i64> = row.get(8)?;
    let exit_code: Option<i32> = row.get(9)?;
    let summary_message: Option<String> = row.get(10)?;
    let stats_json: Option<String> = row.get(11)?;
    let log_path: Option<String> = row.get(12)?;

    let to_sql_err = |msg: String| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, msg)),
        )
    };

    let id = RunId::parse(id_raw).map_err(|e| to_sql_err(e.to_string()))?;
    let destination_id =
        DestinationId::parse(dest_id_raw).map_err(|e| to_sql_err(e.to_string()))?;
    let backup_id = backup_id_raw
        .map(BackupId::parse)
        .transpose()
        .map_err(|e| to_sql_err(e.to_string()))?;

    let kind = match kind_raw.as_str() {
        "backup" => RunKind::Backup {
            backup_id: backup_id
                .ok_or_else(|| to_sql_err("backup run missing backup_id".into()))?,
        },
        "maintenance" => RunKind::Maintenance,
        "restore" => RunKind::Restore { backup_id },
        "meta" => RunKind::Meta,
        other => return Err(to_sql_err(format!("unknown run kind '{other}'"))),
    };

    let status = RunStatus::from_wire_name(&status_raw)
        .ok_or_else(|| to_sql_err(format!("unknown run status '{status_raw}'")))?;
    let pending_reason = pending_raw
        .as_deref()
        .map(|raw| {
            PendingReason::from_wire_name(raw)
                .ok_or_else(|| to_sql_err(format!("unknown pending reason '{raw}'")))
        })
        .transpose()?;

    let started_at =
        OffsetDateTime::parse(&started_raw, &Rfc3339).map_err(|e| to_sql_err(e.to_string()))?;
    let finished_at = finished_raw
        .as_deref()
        .map(|s| OffsetDateTime::parse(s, &Rfc3339))
        .transpose()
        .map_err(|e| to_sql_err(e.to_string()))?;
    let (stats, maintenance) =
        decode_stats_json(&kind, stats_json.as_deref()).map_err(|e| to_sql_err(e.to_string()))?;

    Ok(Run {
        id,
        kind,
        destination_id,
        status,
        pending_reason,
        started_at,
        finished_at,
        duration_ms: duration_ms.map(|v| from_sql_int(8, v)).transpose()?,
        exit_code,
        summary_message,
        commands: Vec::new(),
        stats,
        maintenance,
        log_path: log_path.map(PathBuf::from),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_integer_outside_the_target_range_is_a_corrupted_row() {
        let dir = tempfile::tempdir().expect("tempdir");
        let history = RunHistory::open(dir.path()).expect("open history");
        let backup_id = BackupId::generate().expect("backup id");
        history
            .conn
            .execute(
                "INSERT INTO backup_state (backup_id, backoff_attempt) VALUES (?1, -1)",
                params![backup_id.as_str()],
            )
            .expect("insert corrupted row");

        let err = history
            .get_backup_runtime_state(&backup_id)
            .expect_err("a negative retry count must not load");
        assert!(
            matches!(
                err,
                HistoryError::Sqlite(rusqlite::Error::IntegralValueOutOfRange(4, -1))
            ),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn value_beyond_the_sqlite_integer_range_is_not_written() {
        assert_eq!(
            to_sql_int(u64::from(u32::MAX)).ok(),
            Some(i64::from(u32::MAX))
        );
        let err = to_sql_int(u64::MAX).expect_err("u64::MAX does not fit into i64");
        assert!(
            matches!(err, rusqlite::Error::ToSqlConversionFailure(_)),
            "unexpected error: {err:?}"
        );
    }
}
