// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! SQLite run history, redacted per-run logs, startup recovery, and log retention
//! (`FR-18`, `F13`, `NFR-02`, `FR-21`, `M1-S08`, `M1-S10`).

use crate::clock::Clock;
use crate::model::{
    BackupId, CommandRecord, DestinationId, PendingReason, Run, RunId, RunKind, RunStats, RunStatus,
};
use crate::restic::redact_secrets;
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use thiserror::Error;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// Retention duration after which old logs beyond the newest 100 runs are pruned (`F13`, `AC-M1-08-4`).
pub const LOG_RETENTION_DAYS: i64 = 90;
/// Minimum number of latest runs per backup whose logs are always kept (`F13`, `AC-M1-08-4`).
pub const LOG_KEEP_MIN_RUNS: usize = 100;

/// Errors returned by [`RunHistory`].
#[derive(Debug, Error)]
pub enum HistoryError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
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

impl RunHistory {
    /// Opens (or initializes) `<state_dir>/history.db` and ensures `<state_dir>/logs` exists.
    pub fn open(state_dir: &Path) -> Result<Self, HistoryError> {
        fs::create_dir_all(state_dir)?;
        fs::create_dir_all(state_dir.join("logs"))?;
        let db_path = state_dir.join("history.db");
        let conn = Connection::open(db_path)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        init_schema(&conn)?;
        Ok(Self {
            state_dir: state_dir.to_path_buf(),
            conn,
        })
    }

    /// Returns the state directory backing this history store.
    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    /// Writes redacted log lines for `run_id` into `<state_dir>/logs/<run_id>.log` (`NFR-02`, `AC-M1-08-2`).
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
        let stats_json = run.stats.as_ref().map(serde_json::to_string).transpose()?;
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
                status_to_str(run.status),
                run.pending_reason.map(pending_reason_to_str),
                run.started_at.unix_timestamp(),
                started_rfc,
                finished_rfc,
                run.duration_ms.map(|v| v as i64),
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
                    idx as i64,
                    redact_secrets(&cmd.executable, secrets),
                    args_json,
                    env_keys_json,
                    cmd_started,
                    cmd.duration_ms as i64,
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

    /// Lists all runs ordered newest first.
    pub fn list_runs(&self) -> Result<Vec<Run>, HistoryError> {
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
            let mut run = row?;
            run.commands = self.load_commands(&run.id)?;
            runs.push(run);
        }
        Ok(runs)
    }

    /// Lists runs for a specific [`BackupId`], newest first.
    pub fn list_runs_for_backup(&self, backup_id: &BackupId) -> Result<Vec<Run>, HistoryError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, backup_id, destination_id, status, pending_reason,
                    started_at, finished_at, duration_ms, exit_code,
                    summary_message, stats_json, log_path
             FROM runs
             WHERE backup_id = ?1
             ORDER BY started_at_unix DESC, rowid DESC",
        )?;
        let rows = stmt.query_map(params![backup_id.as_str()], read_run_row)?;
        let mut runs = Vec::new();
        for row in rows {
            let mut run = row?;
            run.commands = self.load_commands(&run.id)?;
            runs.push(run);
        }
        Ok(runs)
    }

    /// Marks any runs stuck in [`RunStatus::Running`] as [`RunStatus::Interrupted`] on daemon startup
    /// (`domain-model.md`, `AC-M1-10-2`).
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
    /// newest 100 runs of that backup (`F13`, `AC-M1-08-4`). Returns number of deleted log files.
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

    /// Sets or clears the persistent `pending_forget` flag for `backup_id` (`M2-S07`, `P11`).
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

    /// Returns whether `backup_id` currently has a pending `forget` (`"forget offen"`, `M2-S07`).
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

    /// Lists all [`BackupId`]s currently flagged with `pending_forget = 1` (`M2-S07`).
    pub fn list_pending_forget_backup_ids(&self) -> Result<Vec<BackupId>, HistoryError> {
        let mut stmt = self.conn.prepare(
            "SELECT backup_id FROM backup_state WHERE pending_forget != 0 ORDER BY backup_id ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            let raw: String = row.get(0)?;
            Ok(raw)
        })?;
        let mut ids = Vec::new();
        for row in rows {
            let raw = row?;
            let id = BackupId::parse(&raw)?;
            ids.push(id);
        }
        Ok(ids)
    }

    /// Removes per-backup state row when a backup is deleted.
    pub fn clear_backup_state(&self, backup_id: &BackupId) -> Result<(), HistoryError> {
        self.conn.execute(
            "DELETE FROM backup_state WHERE backup_id = ?1",
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
                duration_ms: duration_ms.max(0) as u64,
                exit_code,
            });
        }
        Ok(commands)
    }
}

fn init_schema(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS runs (
            id TEXT PRIMARY KEY NOT NULL,
            kind TEXT NOT NULL,
            backup_id TEXT,
            destination_id TEXT NOT NULL,
            status TEXT NOT NULL,
            pending_reason TEXT,
            started_at_unix INTEGER NOT NULL,
            started_at TEXT NOT NULL,
            finished_at TEXT,
            duration_ms INTEGER,
            exit_code INTEGER,
            summary_message TEXT,
            stats_json TEXT,
            log_path TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_runs_backup_time
            ON runs (backup_id, started_at_unix DESC);
        CREATE TABLE IF NOT EXISTS run_commands (
            run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
            seq INTEGER NOT NULL,
            executable TEXT NOT NULL,
            args_json TEXT NOT NULL,
            env_keys_json TEXT NOT NULL,
            started_at TEXT NOT NULL,
            duration_ms INTEGER NOT NULL,
            exit_code INTEGER NOT NULL,
            PRIMARY KEY (run_id, seq)
        );
        CREATE TABLE IF NOT EXISTS backup_state (
            backup_id TEXT PRIMARY KEY NOT NULL,
            pending_forget INTEGER NOT NULL DEFAULT 0
        );",
    )
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

    let status = str_to_status(&status_raw).map_err(to_sql_err)?;
    let pending_reason = pending_raw
        .as_deref()
        .map(str_to_pending_reason)
        .transpose()
        .map_err(to_sql_err)?;

    let started_at =
        OffsetDateTime::parse(&started_raw, &Rfc3339).map_err(|e| to_sql_err(e.to_string()))?;
    let finished_at = finished_raw
        .as_deref()
        .map(|s| OffsetDateTime::parse(s, &Rfc3339))
        .transpose()
        .map_err(|e| to_sql_err(e.to_string()))?;
    let stats: Option<RunStats> = stats_json
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|e| to_sql_err(e.to_string()))?;

    Ok(Run {
        id,
        kind,
        destination_id,
        status,
        pending_reason,
        started_at,
        finished_at,
        duration_ms: duration_ms.map(|v| v.max(0) as u64),
        exit_code,
        summary_message,
        commands: Vec::new(),
        stats,
        log_path: log_path.map(PathBuf::from),
    })
}

fn status_to_str(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Pending => "pending",
        RunStatus::Running => "running",
        RunStatus::Ok => "ok",
        RunStatus::Warning => "warning",
        RunStatus::Error => "error",
        RunStatus::Cancelled => "cancelled",
        RunStatus::Interrupted => "interrupted",
    }
}

fn str_to_status(s: &str) -> Result<RunStatus, String> {
    match s {
        "pending" => Ok(RunStatus::Pending),
        "running" => Ok(RunStatus::Running),
        "ok" => Ok(RunStatus::Ok),
        "warning" => Ok(RunStatus::Warning),
        "error" => Ok(RunStatus::Error),
        "cancelled" => Ok(RunStatus::Cancelled),
        "interrupted" => Ok(RunStatus::Interrupted),
        other => Err(format!("unknown run status '{other}'")),
    }
}

fn pending_reason_to_str(reason: PendingReason) -> &'static str {
    match reason {
        PendingReason::ConditionNotMet => "condition_not_met",
        PendingReason::SourceUnavailable => "source_unavailable",
        PendingReason::DestinationUnreachable => "destination_unreachable",
        PendingReason::Queued => "queued",
        PendingReason::Backoff => "backoff",
    }
}

fn str_to_pending_reason(s: &str) -> Result<PendingReason, String> {
    match s {
        "condition_not_met" => Ok(PendingReason::ConditionNotMet),
        "source_unavailable" => Ok(PendingReason::SourceUnavailable),
        "destination_unreachable" => Ok(PendingReason::DestinationUnreachable),
        "queued" => Ok(PendingReason::Queued),
        "backoff" => Ok(PendingReason::Backoff),
        other => Err(format!("unknown pending reason '{other}'")),
    }
}
