//! Schema migration runner. Reads the current schema version, runs
//! any un-applied migrations in order, and bumps the version. Idempotent
//! — running on a fully-migrated DB is a no-op (just one `SELECT`).
//!
//! Migrations are append-only. See `schema.rs` for the convention.
//!
//! See the Part B plan §2.6 for the design rationale: a hand-rolled
//! runner is cheaper than `refinery` (async-first) or `sqlx-migrate`
//! (requires the `sqlx` runtime), and we have at most a handful of
//! migrations.

#![allow(missing_docs)]

use crate::error::{Result, StoreError};
use crate::schema::{
    CURRENT_SCHEMA_VERSION, MIGRATION_001_INITIAL, MIGRATION_002_FTS5_INTERNAL_CONTENT,
};
use rusqlite::Connection;

/// A single migration: a version number and the SQL to run. The
/// migration runner applies them in ascending order.
struct Migration {
    version: u32,
    sql: &'static str,
}

/// The full migration list. Append-only — never reorder, never edit.
/// When the next migration lands, push it to the end and bump
/// `CURRENT_SCHEMA_VERSION` in `schema.rs`.
const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: MIGRATION_001_INITIAL,
    },
    Migration {
        version: 2,
        sql: MIGRATION_002_FTS5_INTERNAL_CONTENT,
    },
];

/// Run all un-applied migrations. Safe to call on every project open.
/// The `db::open` path calls this on the first connection from the
/// pool after the pool is constructed.
pub fn run(conn: &Connection) -> Result<()> {
    let current = current_version(conn)?;

    if current == CURRENT_SCHEMA_VERSION {
        return Ok(()); // up to date
    }
    if current > CURRENT_SCHEMA_VERSION {
        return Err(StoreError::Invalid(format!(
            "project database is at schema version {} but this Talon only knows up to {}. \
             the project was created by a newer version of Talon — please upgrade.",
            current, CURRENT_SCHEMA_VERSION
        )));
    }

    // Run each migration we haven't yet seen, wrapped in a transaction
    // so a mid-migration failure rolls back cleanly. We use
    // `unchecked_transaction` so we don't fight rusqlite over the
    // active-savepoint behavior on a fresh connection.
    for m in MIGRATIONS {
        if m.version > current {
            let tx = conn.unchecked_transaction()?;
            tx.execute_batch(m.sql)
                .map_err(|e| StoreError::Migration(m.version, e.to_string()))?;
            tx.execute(
                "INSERT OR IGNORE INTO schema_version (version) VALUES (?1)",
                rusqlite::params![m.version],
            )?;
            tx.commit()?;
        }
    }
    Ok(())
}

/// Read the current schema version. A fresh database (just opened,
/// never migrated) returns 0.
fn current_version(conn: &Connection) -> Result<u32> {
    // The schema_version table may not exist yet on a brand-new DB.
    // We use a guarded COUNT(*) so the absent-table case returns 0
    // rows (the query plan short-circuits) instead of an error.
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='schema_version'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if exists == 0 {
        return Ok(0);
    }
    let v: i64 = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_version",
        [],
        |r| r.get(0),
    )?;
    Ok(v as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn run_on_empty_db_creates_all_tables() {
        let conn = Connection::open_in_memory().unwrap();
        run(&conn).unwrap();
        // After migration, every table should exist.
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap();
        let tables: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|r| r.unwrap_or_default())
            .collect();
        for expected in &[
            "exchange_fts",
            "exchange_tags",
            "exchanges",
            "projects",
            "schema_version",
            "tags",
        ] {
            assert!(
                tables.contains(&expected.to_string()),
                "missing table: {expected} (have: {tables:?})"
            );
        }
    }

    #[test]
    fn run_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        run(&conn).unwrap();
        run(&conn).unwrap(); // second run should be a no-op
        let v = current_version(&conn).unwrap();
        assert_eq!(v, CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn fresh_db_starts_at_version_zero() {
        let conn = Connection::open_in_memory().unwrap();
        assert_eq!(current_version(&conn).unwrap(), 0);
    }

    #[test]
    fn fts5_module_is_loadable() {
        // The exchange_fts virtual table relies on the FTS5 SQLite
        // extension. FTS5 is enabled by rusqlite's `bundled` feature
        // (which pulls in `modern_sqlite`). If this test ever fails,
        // check that `bundled` is still in the features list and
        // that `libsqlite3-sys` is being built from source.
        let conn = Connection::open_in_memory().unwrap();
        let result = conn.execute_batch("CREATE VIRTUAL TABLE t USING fts5(x);");
        assert!(result.is_ok(), "FTS5 unavailable: {:?}", result.err());
    }
}
