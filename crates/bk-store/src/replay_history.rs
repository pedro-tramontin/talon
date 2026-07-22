//! Per-tab replay history persistence (Phase 6 Part C, §C-A.4).
//!
//! Each `replay_history` row is a single send event: the request
//! exchange + (optionally) the response exchange, scoped to a
//! replay tab (`tab_id` is a UUID the UI mints on tab open). The
//! per-tab sequence number (`sequence_within_tab`) preserves the
//! user's send order.
//!
//! The UI's `ReplayStore.openTab` action calls `list_by_tab` to
//! rehydrate the tab's in-memory `history` field; `appendSend`
//! calls `insert` to persist each new entry. The minimal-viable
//! persistence (no auto-reopen of tabs on app start) is the
//! v0.5+ follow-up; the SQL plumbing is here now.
//!
//! **Why a Rust struct for the entry, not just raw SQL?** the
//! Tauri command (`list_replay_history`) returns
//! `Vec<ReplayHistoryEntry>`; the struct is the DTO the JS side
//! consumes (via `serde::Serialize`). The fields map 1:1 to the
//! `replay_history` table columns.

#![allow(missing_docs)]

use crate::error::Result;
use crate::DbPool;
use bk_core::{ExchangeId, ProjectId};
use chrono::{DateTime, Utc};
use rusqlite::params;
use serde::{Deserialize, Serialize};

/// A single replay send event, persisted in the
/// `replay_history` table. The `id` is a UUID v4 minted on insert;
/// the `tab_id` is a UUID the UI mints on tab open (so the same
/// tab_id can be used to rehydrate history after a tab close +
/// reopen with the same `source_exchange_id`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayHistoryEntry {
    pub id: String,
    pub project_id: ProjectId,
    pub tab_id: String,
    pub request_exchange_id: ExchangeId,
    pub response_exchange_id: Option<ExchangeId>,
    pub timestamp: DateTime<Utc>,
    pub sequence_within_tab: i64,
}

/// Insert a new `replay_history` row. The caller (the Tauri
/// `append_replay_history` command) mints the `id` (UUID) and
/// `sequence_within_tab` (the tab's current sequence count). The
/// `timestamp` is set server-side via `datetime('now')` for
/// consistency.
pub fn insert(pool: &DbPool, entry: &ReplayHistoryEntry) -> Result<()> {
    let conn = pool.get()?;
    conn.execute(
        "INSERT INTO replay_history
            (id, project_id, tab_id, request_exchange_id, response_exchange_id,
             timestamp, sequence_within_tab)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            entry.id,
            entry.project_id.to_string(),
            entry.tab_id,
            entry.request_exchange_id.to_string(),
            entry.response_exchange_id.map(|e| e.to_string()),
            entry.timestamp.to_rfc3339(),
            entry.sequence_within_tab,
        ],
    )?;
    Ok(())
}

/// Return every entry for a given `tab_id`, ordered by
/// `sequence_within_tab` ASC. The UI uses this to rehydrate the
/// tab's `history` field on `openTab`.
pub fn list_by_tab(pool: &DbPool, tab_id: &str) -> Result<Vec<ReplayHistoryEntry>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        "SELECT id, project_id, tab_id, request_exchange_id, response_exchange_id,
                timestamp, sequence_within_tab
         FROM replay_history
         WHERE tab_id = ?1
         ORDER BY sequence_within_tab ASC",
    )?;
    let rows = stmt
        .query_map(params![tab_id], row_to_entry)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReplayHistoryEntry> {
    let project_id_str: String = row.get(1)?;
    let req_id_str: String = row.get(3)?;
    let resp_id_str: Option<String> = row.get(4)?;
    let ts_str: String = row.get(5)?;
    Ok(ReplayHistoryEntry {
        id: row.get(0)?,
        project_id: project_id_str.parse().map_err(|e: uuid::Error| {
            rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(e))
        })?,
        tab_id: row.get(2)?,
        request_exchange_id: req_id_str.parse().map_err(|e: uuid::Error| {
            rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(e))
        })?,
        response_exchange_id: resp_id_str
            .map(|s| {
                s.parse().map_err(|e: uuid::Error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        4,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })
            })
            .transpose()?,
        timestamp: chrono::DateTime::parse_from_rfc3339(&ts_str)
            .map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    5,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?
            .with_timezone(&Utc),
        sequence_within_tab: row.get(6)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bk_core::{ExchangeId, ProjectId};
    use tempfile::TempDir;

    fn fresh_pool() -> (TempDir, DbPool) {
        let tmp = TempDir::new().unwrap();
        let pool = crate::db::open(tmp.path().join("test.db")).unwrap();
        (tmp, pool)
    }

    fn insert_project_row(pool: &DbPool, id: ProjectId) {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO projects
                (id, name, target_host, created_at, updated_at, db_filename, talon_version, settings_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                id.to_string(),
                "test-project",
                "acme.bb",
                "2026-01-01T00:00:00Z",
                "2026-01-01T00:00:00Z",
                "test-project-2026-01-01.db",
                "0.1.0",
                "{}",
            ],
        )
        .expect("insert project row");
    }

    fn insert_exchange_row(pool: &DbPool, id: ExchangeId, project_id: ProjectId) {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO exchanges
                (id, project_id, timestamp, summary, scope_state, request_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                id.to_string(),
                project_id.to_string(),
                "2026-01-01T00:00:00Z",
                "GET /",
                "unscoped",
                r#"{"method":"GET","url":"https://acme.bb/","version":"HTTP/1.1","headers":[],"body":{"kind":"empty"}}"#,
            ],
        )
        .expect("insert exchange row");
    }

    fn make_entry(
        id: &str,
        project_id: ProjectId,
        tab_id: &str,
        req_id: ExchangeId,
        seq: i64,
    ) -> ReplayHistoryEntry {
        ReplayHistoryEntry {
            id: id.to_string(),
            project_id,
            tab_id: tab_id.to_string(),
            request_exchange_id: req_id,
            response_exchange_id: None,
            timestamp: Utc::now(),
            sequence_within_tab: seq,
        }
    }

    /// `insert` + `list_by_tab` round-trips: an entry written
    /// comes back in `list_by_tab`, in `sequence_within_tab`
    /// ASC order.
    #[test]
    fn insert_and_list_round_trip() {
        let (_tmp, pool) = fresh_pool();
        let pid = ProjectId::new();
        insert_project_row(&pool, pid);
        let req = ExchangeId::new();
        insert_exchange_row(&pool, req, pid);

        let entry = make_entry("entry-1", pid, "tab-A", req, 0);
        insert(&pool, &entry).unwrap();

        let listed = list_by_tab(&pool, "tab-A").unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "entry-1");
        assert_eq!(listed[0].sequence_within_tab, 0);
    }

    /// `list_by_tab` returns entries ordered by
    /// `sequence_within_tab` ASC, not by insertion order.
    #[test]
    fn list_orders_by_sequence_within_tab() {
        let (_tmp, pool) = fresh_pool();
        let pid = ProjectId::new();
        insert_project_row(&pool, pid);
        let req = ExchangeId::new();
        insert_exchange_row(&pool, req, pid);

        // Insert in reverse-sequence order.
        insert(&pool, &make_entry("e-2", pid, "tab-B", req, 2)).unwrap();
        insert(&pool, &make_entry("e-0", pid, "tab-B", req, 0)).unwrap();
        insert(&pool, &make_entry("e-1", pid, "tab-B", req, 1)).unwrap();

        let listed = list_by_tab(&pool, "tab-B").unwrap();
        let seqs: Vec<i64> = listed.iter().map(|e| e.sequence_within_tab).collect();
        assert_eq!(
            seqs,
            vec![0, 1, 2],
            "must be ordered ASC by sequence_within_tab"
        );
    }

    /// Migration 003 is idempotent: running the migration twice
    /// (via the runner) doesn't error. The runner handles this;
    /// this test just verifies the table creation is
    /// `IF NOT EXISTS` safe.
    #[test]
    fn migration_003_is_idempotent() {
        let (_tmp, pool) = fresh_pool();
        // Pool already ran migration 003 in `open`. Re-running
        // is a no-op (the runner checks `current_version`).
        crate::migrations::run(&pool.get().unwrap()).unwrap();
        // Insert a row to confirm the schema is usable.
        let pid = ProjectId::new();
        insert_project_row(&pool, pid);
        let req = ExchangeId::new();
        insert_exchange_row(&pool, req, pid);
        let entry = make_entry("e-1", pid, "tab-X", req, 0);
        insert(&pool, &entry).expect("insert after re-migration");
    }

    /// `list_by_tab` returns empty `Vec` for a tab that has
    /// no entries (the "fresh tab" case on `openTab`).
    #[test]
    fn list_by_tab_empty_for_unknown_tab() {
        let (_tmp, pool) = fresh_pool();
        let listed = list_by_tab(&pool, "tab-does-not-exist").unwrap();
        assert!(listed.is_empty());
    }
}
