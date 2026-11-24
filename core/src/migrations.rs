// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Versioned, transactional SQLite schema migrations built on `PRAGMA user_version`.
//!
//! Each migration runs inside a single SQLite transaction together with its `user_version` update,
//! ensuring atomic schema evolution without a separate metadata table or ORM overhead.

use rusqlite::{Connection, Transaction};
use thiserror::Error;

/// A single forward schema migration step.
#[derive(Clone, Copy)]
pub struct Migration {
    /// Strictly increasing schema version (`1, 2, 3, ...`).
    pub version: i32,
    /// Short identifier for diagnostics and logging.
    pub name: &'static str,
    /// Transactional migration body.
    pub apply: fn(&Transaction<'_>) -> Result<(), rusqlite::Error>,
}

/// Errors produced while validating or applying schema migrations.
#[derive(Debug, Error)]
pub enum MigrationError {
    #[error("invalid migration sequence at '{name}' (version {version}): {reason}")]
    InvalidSequence {
        version: i32,
        name: &'static str,
        reason: &'static str,
    },
    #[error("database schema version {current} is newer than supported version {max_supported}")]
    UnsupportedFutureVersion { current: i32, max_supported: i32 },
    #[error("migration v{version} ('{name}') failed: {source}")]
    ApplyFailed {
        version: i32,
        name: &'static str,
        #[source]
        source: rusqlite::Error,
    },
    #[error("SQLite migration metadata error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

/// Validates that `migrations` are strictly sequential starting from `1` without duplicates.
pub fn validate_migrations(migrations: &[Migration]) -> Result<(), MigrationError> {
    for (expected, m) in (1i32..).zip(migrations.iter()) {
        if m.version <= 0 {
            return Err(MigrationError::InvalidSequence {
                version: m.version,
                name: m.name,
                reason: "migration versions must be positive",
            });
        }
        if m.version != expected {
            return Err(MigrationError::InvalidSequence {
                version: m.version,
                name: m.name,
                reason: "migration versions must be contiguous starting at 1",
            });
        }
    }
    Ok(())
}

/// Reads the current SQLite `PRAGMA user_version`.
pub fn current_schema_version(conn: &Connection) -> Result<i32, MigrationError> {
    Ok(conn.pragma_query_value(None, "user_version", |row| row.get(0))?)
}

/// Applies all pending migrations in order; each migration + `PRAGMA user_version` bump is atomic.
pub fn run_migrations(
    conn: &mut Connection,
    migrations: &[Migration],
) -> Result<(), MigrationError> {
    validate_migrations(migrations)?;
    let current = current_schema_version(conn)?;
    let max_supported = migrations.last().map_or(0, |m| m.version);
    if current > max_supported {
        return Err(MigrationError::UnsupportedFutureVersion {
            current,
            max_supported,
        });
    }

    for m in migrations.iter().filter(|m| m.version > current) {
        let tx = conn.transaction()?;
        if let Err(source) = (m.apply)(&tx) {
            return Err(MigrationError::ApplyFailed {
                version: m.version,
                name: m.name,
                source,
            });
        }
        tx.pragma_update(None, "user_version", m.version)?;
        tx.commit()?;
    }

    Ok(())
}

/// Ordered schema migrations for Kestic's `history.db`.
pub fn history_migrations() -> &'static [Migration] {
    HISTORY_MIGRATIONS
}

const HISTORY_MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "m1_m2_base_schema",
        apply: apply_v1_base_schema,
    },
    Migration {
        version: 2,
        name: "m3_automation_state",
        apply: apply_v2_m3_automation_state,
    },
    Migration {
        version: 3,
        name: "m3_backoff_tracking",
        apply: apply_v3_backoff_tracking,
    },
    Migration {
        version: 4,
        name: "maintenance_schedule_anchors",
        apply: apply_v4_maintenance_schedule_anchors,
    },
    Migration {
        version: 5,
        name: "destination_repository_id",
        apply: apply_v5_destination_repository_id,
    },
];

fn apply_v1_base_schema(tx: &Transaction<'_>) -> Result<(), rusqlite::Error> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS runs (
            id TEXT PRIMARY KEY NOT NULL,
            kind TEXT NOT NULL,
            backup_id TEXT,
            destination_id TEXT NOT NULL,
            status TEXT NOT NULL,
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

fn apply_v2_m3_automation_state(tx: &Transaction<'_>) -> Result<(), rusqlite::Error> {
    add_column_if_missing(tx, "runs", "pending_reason", "TEXT")?;

    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS destination_state (
            destination_id TEXT PRIMARY KEY NOT NULL,
            baseline_at_unix INTEGER,
            last_prune_at_unix INTEGER,
            last_check_at_unix INTEGER
        );
        CREATE TABLE IF NOT EXISTS app_state (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            global_paused_since_unix INTEGER,
            global_paused_until_unix INTEGER
        );",
    )?;

    add_column_if_missing(tx, "backup_state", "baseline_at_unix", "INTEGER")?;
    add_column_if_missing(tx, "backup_state", "paused_since_unix", "INTEGER")?;
    add_column_if_missing(tx, "backup_state", "paused_until_unix", "INTEGER")?;
    add_column_if_missing(
        tx,
        "backup_state",
        "backoff_attempt",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(tx, "backup_state", "backoff_until_unix", "INTEGER")?;
    add_column_if_missing(
        tx,
        "backup_state",
        "last_overdue_notified_at_unix",
        "INTEGER",
    )?;
    add_column_if_missing(tx, "backup_state", "overdue_anchor_at_unix", "INTEGER")?;
    add_column_if_missing(tx, "backup_state", "last_failure_at_unix", "INTEGER")?;
    Ok(())
}

fn apply_v3_backoff_tracking(tx: &Transaction<'_>) -> Result<(), rusqlite::Error> {
    add_column_if_missing(tx, "backup_state", "backoff_kind", "TEXT")?;
    add_column_if_missing(
        tx,
        "destination_state",
        "maintenance_backoff_attempt",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(
        tx,
        "destination_state",
        "maintenance_backoff_until_unix",
        "INTEGER",
    )?;
    add_column_if_missing(
        tx,
        "destination_state",
        "last_maintenance_failure_at_unix",
        "INTEGER",
    )?;
    Ok(())
}

fn apply_v4_maintenance_schedule_anchors(tx: &Transaction<'_>) -> Result<(), rusqlite::Error> {
    add_column_if_missing(
        tx,
        "destination_state",
        "prune_schedule_set_at_unix",
        "INTEGER",
    )?;
    add_column_if_missing(
        tx,
        "destination_state",
        "check_schedule_set_at_unix",
        "INTEGER",
    )?;
    Ok(())
}

fn apply_v5_destination_repository_id(tx: &Transaction<'_>) -> Result<(), rusqlite::Error> {
    add_column_if_missing(tx, "destination_state", "repo_id", "TEXT")
}

/// Guard for upgrading pre-versioned databases that still had `user_version = 0`.
/// Future migrations (`v3+`) can execute `ALTER TABLE` directly inside the transaction.
fn add_column_if_missing(
    tx: &Transaction<'_>,
    table: &str,
    column: &str,
    column_def: &str,
) -> Result<(), rusqlite::Error> {
    let has_col = {
        let mut stmt = tx.prepare(&format!("PRAGMA table_info({table})"))?;
        let mut rows = stmt.query([])?;
        let mut found = false;
        while let Some(row) = rows.next()? {
            let name: String = row.get(1)?;
            if name == column {
                found = true;
                break;
            }
        }
        found
    };
    if !has_col {
        tx.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {column_def}"),
            [],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_migrations_are_valid_and_idempotent() {
        validate_migrations(history_migrations()).expect("valid migration sequence");
        let mut conn = Connection::open_in_memory().expect("open memory db");
        assert_eq!(current_schema_version(&conn).unwrap(), 0);

        run_migrations(&mut conn, history_migrations()).expect("migrate empty db");
        assert_eq!(
            current_schema_version(&conn).unwrap(),
            history_migrations().last().unwrap().version
        );

        // Re-running is a no-op.
        run_migrations(&mut conn, history_migrations()).expect("idempotent rerun");
        assert_eq!(
            current_schema_version(&conn).unwrap(),
            history_migrations().last().unwrap().version
        );
    }

    #[test]
    fn upgrades_legacy_unversioned_m2_database_preserving_data() {
        let mut conn = Connection::open_in_memory().expect("open memory db");
        conn.execute_batch(
            "CREATE TABLE runs (
                id TEXT PRIMARY KEY NOT NULL,
                kind TEXT NOT NULL,
                backup_id TEXT,
                destination_id TEXT NOT NULL,
                status TEXT NOT NULL,
                started_at_unix INTEGER NOT NULL,
                started_at TEXT NOT NULL,
                finished_at TEXT,
                duration_ms INTEGER,
                exit_code INTEGER,
                summary_message TEXT,
                stats_json TEXT,
                log_path TEXT
            );
            CREATE TABLE backup_state (
                backup_id TEXT PRIMARY KEY NOT NULL,
                pending_forget INTEGER NOT NULL DEFAULT 0
            );
            INSERT INTO runs (id, kind, backup_id, destination_id, status, started_at_unix, started_at)
            VALUES ('r1', 'backup', 'b1', 'd1', 'ok', 1700000000, '2023-11-14T22:13:20Z');
            INSERT INTO backup_state (backup_id, pending_forget) VALUES ('b1', 1);",
        )
        .expect("seed legacy M2 schema");
        assert_eq!(current_schema_version(&conn).unwrap(), 0);

        run_migrations(&mut conn, history_migrations()).expect("upgrade M2 -> current");
        assert_eq!(
            current_schema_version(&conn).unwrap(),
            history_migrations().last().unwrap().version
        );

        let pending_reason: Option<String> = conn
            .query_row("SELECT pending_reason FROM runs WHERE id = 'r1'", [], |r| {
                r.get(0)
            })
            .expect("pending_reason column exists");
        assert!(pending_reason.is_none());

        let (pf, backoff, bkind): (i64, i64, Option<String>) = conn
            .query_row(
                "SELECT pending_forget, backoff_attempt, backoff_kind FROM backup_state WHERE backup_id = 'b1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .expect("M2 row preserved with M3 default column");
        assert_eq!(pf, 1);
        assert_eq!(backoff, 0);
        assert!(bkind.is_none());
    }

    #[test]
    fn rejects_unsupported_future_schema_version() {
        let mut conn = Connection::open_in_memory().expect("open memory db");
        conn.pragma_update(None, "user_version", 999).unwrap();
        let err = run_migrations(&mut conn, history_migrations()).unwrap_err();
        let latest = history_migrations().last().unwrap().version;
        assert!(matches!(
            err,
            MigrationError::UnsupportedFutureVersion {
                current: 999,
                max_supported
            } if max_supported == latest
        ));
    }
}
