//! Typed CRUD for `HttpExchange`. The proxy and the replay tabs call
//! these functions; the FTS5 sync happens here too, not in a trigger.
//!
//! Storage shape: `Request` and `Response` are serialized as JSON blobs
//! in `exchanges.request_json` / `exchanges.response_json`. The indexed
//! columns are only the ones we filter or sort by (timestamp, starred,
//! scope_state). The full rationale is in the Part B plan §2.7
//! ("Why JSON for the body and not BLOB").

#![allow(missing_docs)]

use crate::error::{Result, StoreError};
use crate::DbPool;
use bk_core::{ExchangeId, HttpExchange, ProjectId, ScopeState};
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension, Row};

/// Insert a new exchange. Returns the inserted ID (always equal to
/// `exchange.meta.id`, but the return type is convenient for chaining).
///
/// The FTS5 row is inserted in the same transaction. If FTS5 sync
/// fails, the whole insert rolls back so we never have a row in
/// `exchanges` that's missing from the search index.
pub fn insert(pool: &DbPool, exchange: &HttpExchange) -> Result<ExchangeId> {
    let mut conn = pool.get()?;
    let tx = conn.transaction()?;

    let request_json = serde_json::to_string(&exchange.request)?;
    let response_json = exchange
        .response
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;

    tx.execute(
        r#"INSERT INTO exchanges
            (id, project_id, timestamp, duration_ns, summary, scope_state, notes, starred, blocked_reason, request_json, response_json)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"#,
        params![
            exchange.meta.id.to_string(),
            exchange.meta.project_id.to_string(),
            exchange.meta.timestamp.to_rfc3339(),
            exchange.meta.duration_ns as i64,
            exchange.meta.summary,
            scope_state_to_str(exchange.meta.scope_state),
            exchange.meta.notes,
            exchange.meta.starred as i64,
            exchange.blocked_reason,
            request_json,
            response_json,
        ],
    )?;

    crate::fts::index_exchange(&tx, exchange)?;
    tx.commit()?;
    Ok(exchange.meta.id)
}

/// Look up an exchange by ID. Returns `Ok(None)` if not found.
pub fn get(pool: &DbPool, id: ExchangeId) -> Result<Option<HttpExchange>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        r#"SELECT id, project_id, timestamp, duration_ns, summary, scope_state, notes, starred, blocked_reason, request_json, response_json
            FROM exchanges WHERE id = ?1"#,
    )?;
    // In rusqlite 0.32, `query_row().optional()` returns
    // `Result<Option<T>, E>` (not `Option<Result<T, E>>`), so the `?`
    // unwraps the rusqlite error and we're left with `Option<HttpExchange>`.
    stmt.query_row(params![id.to_string()], row_to_exchange)
        .optional()
        .map_err(StoreError::from)
}

/// List the most recent N exchanges for a project, newest first.
/// Used by the UI's exchange list on initial load.
pub fn list_recent(pool: &DbPool, project_id: ProjectId, limit: u32) -> Result<Vec<HttpExchange>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        r#"SELECT id, project_id, timestamp, duration_ns, summary, scope_state, notes, starred, blocked_reason, request_json, response_json
            FROM exchanges
            WHERE project_id = ?1
            ORDER BY timestamp DESC
            LIMIT ?2"#,
    )?;
    let rows = stmt.query_map(
        params![project_id.to_string(), limit as i64],
        row_to_exchange,
    )?;
    let mut out = Vec::with_capacity(limit as usize);
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// Update the free-form notes on an exchange. Used by the right-rail
/// notes editor in the UI.
pub fn update_notes(pool: &DbPool, id: ExchangeId, notes: &str) -> Result<()> {
    let conn = pool.get()?;
    let n = conn.execute(
        "UPDATE exchanges SET notes = ?1 WHERE id = ?2",
        params![notes, id.to_string()],
    )?;
    if n == 0 {
        return Err(StoreError::NotFound(id.to_string()));
    }
    Ok(())
}

/// Set the starred flag. Used by the ⭐ button on each row.
pub fn set_starred(pool: &DbPool, id: ExchangeId, starred: bool) -> Result<()> {
    let conn = pool.get()?;
    let n = conn.execute(
        "UPDATE exchanges SET starred = ?1 WHERE id = ?2",
        params![starred as i64, id.to_string()],
    )?;
    if n == 0 {
        return Err(StoreError::NotFound(id.to_string()));
    }
    Ok(())
}

/// Delete an exchange. The `ON DELETE CASCADE` on `exchange_tags` cleans
/// up tag joins; the FTS5 row uses the exchange's rowid, which we look
/// up before the row is gone.
pub fn delete(pool: &DbPool, id: ExchangeId) -> Result<()> {
    let mut conn = pool.get()?;
    let tx = conn.transaction()?;
    // Look up the FTS5 rowid before deleting the exchanges row.
    // The FTS5 table is contentless; we use the exchanges rowid as
    // the FTS rowid for stable cross-references.
    let rowid: Option<i64> = tx
        .query_row(
            "SELECT rowid FROM exchanges WHERE id = ?1",
            params![id.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(r) = rowid {
        tx.execute("DELETE FROM exchange_fts WHERE rowid = ?1", params![r])?;
    }
    tx.execute(
        "DELETE FROM exchanges WHERE id = ?1",
        params![id.to_string()],
    )?;
    tx.commit()?;
    Ok(())
}

// --- helpers ---

fn scope_state_to_str(s: ScopeState) -> &'static str {
    match s {
        ScopeState::InScope => "in_scope",
        ScopeState::OutOfScope => "out_of_scope",
        ScopeState::Blocked => "blocked",
        ScopeState::Unscoped => "unscoped",
    }
}

fn scope_state_from_str(s: &str) -> Result<ScopeState> {
    Ok(match s {
        "in_scope" => ScopeState::InScope,
        "out_of_scope" => ScopeState::OutOfScope,
        "blocked" => ScopeState::Blocked,
        "unscoped" => ScopeState::Unscoped,
        other => return Err(StoreError::Invalid(format!("unknown scope_state: {other}"))),
    })
}

fn row_to_exchange(row: &Row<'_>) -> rusqlite::Result<HttpExchange> {
    let id_str: String = row.get(0)?;
    let pid_str: String = row.get(1)?;
    let ts_str: String = row.get(2)?;
    let duration_ns: i64 = row.get(3)?;
    let summary: String = row.get(4)?;
    let scope_state: String = row.get(5)?;
    let notes: String = row.get(6)?;
    let starred: i64 = row.get(7)?;
    let blocked_reason: Option<String> = row.get(8)?;
    let request_json: String = row.get(9)?;
    let response_json: Option<String> = row.get(10)?;

    let id: ExchangeId = id_str.parse().map_err(|e: uuid::Error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let project_id: ProjectId = pid_str.parse().map_err(|e: uuid::Error| {
        rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let timestamp: DateTime<Utc> = DateTime::parse_from_rfc3339(&ts_str)
        .map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(e))
        })?
        .with_timezone(&Utc);
    let scope = scope_state_from_str(&scope_state).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let request: bk_core::Request = serde_json::from_str(&request_json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(9, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let response: Option<bk_core::Response> = response_json
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(10, rusqlite::types::Type::Text, Box::new(e))
        })?;

    Ok(HttpExchange {
        meta: bk_core::ExchangeMeta {
            id,
            project_id,
            timestamp,
            duration_ns: duration_ns as u64,
            summary,
            scope_state: scope,
            notes,
            starred: starred != 0,
        },
        request,
        response,
        blocked_reason,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bk_core::{
        Body, ExchangeMeta, HeaderMap, HttpExchange, Method, Request, Response, Url, Version,
    };
    use tempfile::TempDir;

    fn fresh_pool() -> (TempDir, DbPool) {
        let tmp = TempDir::new().unwrap();
        let pool = crate::db::open(tmp.path().join("test.db")).unwrap();
        (tmp, pool)
    }

    /// Insert a minimal `projects` row so the FK constraint on
    /// `exchanges.project_id` is satisfied. We bypass the full
    /// `bk_core::Project` type because the storage layer cares only
    /// about the columns in `schema.rs::MIGRATION_001_INITIAL`. The
    /// typed `Project` model is the engine's job (§2.10).
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

    fn make_exchange(project_id: ProjectId, path: &str) -> HttpExchange {
        let url: Url = format!("https://acme.bb{path}").parse().unwrap();
        let request = Request {
            method: Method::GET,
            url,
            version: Version::HTTP_11,
            headers: HeaderMap::new(),
            body: Body::empty(),
        };
        let response = Response {
            version: Version::HTTP_11,
            status: 200,
            status_text: "OK".to_string(),
            headers: HeaderMap::new(),
            body: Body::from_bytes(r#"{"hello":"world"}"#),
        };
        HttpExchange {
            meta: ExchangeMeta {
                id: ExchangeId::new(),
                project_id,
                timestamp: Utc::now(),
                duration_ns: 1234,
                summary: format!("GET {path}"),
                scope_state: ScopeState::InScope,
                notes: String::new(),
                starred: false,
            },
            request,
            response: Some(response),
            blocked_reason: None,
        }
    }

    #[test]
    fn insert_then_get_roundtrips() {
        let (_tmp, pool) = fresh_pool();
        let project_id = ProjectId::new();
        insert_project_row(&pool, project_id);
        let ex = make_exchange(project_id, "/api/users");
        let id = insert(&pool, &ex).unwrap();
        let back = get(&pool, id).unwrap().expect("exchange should exist");
        assert_eq!(back.meta.id, ex.meta.id);
        assert_eq!(back.meta.summary, "GET /api/users");
        assert_eq!(back.response.as_ref().unwrap().status, 200);
    }

    #[test]
    fn list_recent_returns_newest_first() {
        let (_tmp, pool) = fresh_pool();
        let project_id = ProjectId::new();
        insert_project_row(&pool, project_id);
        // Insert 3 exchanges; the last one inserted should come back first.
        insert(&pool, &make_exchange(project_id, "/a")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        insert(&pool, &make_exchange(project_id, "/b")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        insert(&pool, &make_exchange(project_id, "/c")).unwrap();

        let list = list_recent(&pool, project_id, 10).unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].meta.summary, "GET /c");
        assert_eq!(list[2].meta.summary, "GET /a");
    }

    #[test]
    fn list_recent_respects_limit() {
        let (_tmp, pool) = fresh_pool();
        let project_id = ProjectId::new();
        insert_project_row(&pool, project_id);
        for i in 0..5 {
            insert(&pool, &make_exchange(project_id, &format!("/{i}"))).unwrap();
        }
        let list = list_recent(&pool, project_id, 3).unwrap();
        assert_eq!(list.len(), 3);
    }

    #[test]
    fn update_notes_persists() {
        let (_tmp, pool) = fresh_pool();
        let project_id = ProjectId::new();
        insert_project_row(&pool, project_id);
        let id = insert(&pool, &make_exchange(project_id, "/x")).unwrap();
        update_notes(&pool, id, "found the admin endpoint").unwrap();
        let back = get(&pool, id).unwrap().unwrap();
        assert_eq!(back.meta.notes, "found the admin endpoint");
    }

    #[test]
    fn set_starred_toggles() {
        let (_tmp, pool) = fresh_pool();
        let project_id = ProjectId::new();
        insert_project_row(&pool, project_id);
        let id = insert(&pool, &make_exchange(project_id, "/x")).unwrap();
        assert!(!get(&pool, id).unwrap().unwrap().meta.starred);
        set_starred(&pool, id, true).unwrap();
        assert!(get(&pool, id).unwrap().unwrap().meta.starred);
        set_starred(&pool, id, false).unwrap();
        assert!(!get(&pool, id).unwrap().unwrap().meta.starred);
    }

    #[test]
    fn delete_removes_exchange() {
        let (_tmp, pool) = fresh_pool();
        let project_id = ProjectId::new();
        insert_project_row(&pool, project_id);
        let id = insert(&pool, &make_exchange(project_id, "/x")).unwrap();
        delete(&pool, id).unwrap();
        assert!(get(&pool, id).unwrap().is_none());
    }

    #[test]
    fn update_notes_on_missing_id_errors() {
        let (_tmp, pool) = fresh_pool();
        let res = update_notes(&pool, ExchangeId::new(), "nope");
        assert!(matches!(res, Err(StoreError::NotFound(_))));
    }

    #[test]
    fn blocked_exchange_persists_blocked_reason() {
        let (_tmp, pool) = fresh_pool();
        let project_id = ProjectId::new();
        insert_project_row(&pool, project_id);
        let mut ex = make_exchange(project_id, "/x");
        ex.response = None;
        ex.blocked_reason = Some("scope rule: analytics".to_string());
        ex.meta.scope_state = ScopeState::Blocked;
        let id = insert(&pool, &ex).unwrap();
        let back = get(&pool, id).unwrap().unwrap();
        assert!(back.response.is_none());
        assert_eq!(
            back.blocked_reason.as_deref(),
            Some("scope rule: analytics")
        );
        assert_eq!(back.meta.scope_state, ScopeState::Blocked);
    }
}
