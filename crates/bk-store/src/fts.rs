//! FTS5 full-text search over exchanges.
//!
//! The `exchange_fts` table is an **internal-content** FTS5 virtual
//! table (migration 002 dropped the original contentless variant —
//! that one had a fundamental issue where the FTS5 'delete' command
//! didn't actually remove entries from the inverted index, making
//! `update_notes` leave the search results stale). The FTS table now
//! stores the indexed columns directly, so we can use `REPLACE INTO`
//! (in `index_exchange`) and `DELETE FROM` (in `rebuild` and
//! `exchanges::delete`) to keep the index in sync.
//!
//! The exchanges module is responsible for calling `index_exchange`
//! inside the same transaction as every `exchanges` write (insert,
//! update). If the FTS write fails, the whole exchange write rolls back.
//!
//! Indexed fields:
//!   - url              (request URL, string)
//!   - method           (GET/POST/etc, uppercased)
//!   - request_headers  (all request headers joined as "Name: value" lines)
//!   - response_headers (same for response)
//!   - request_body     (request body as a UTF-8 lossy string)
//!   - response_body    (response body as a UTF-8 lossy string)
//!   - notes            (free-form notes)
//!
//! Non-text fields (status code, host, content-type) are queryable via
//! the regular `exchanges` table — see exchanges.rs. FTS5 is for the
//! "find me any token" case.

#![allow(missing_docs)]

use crate::error::Result;
use crate::DbPool;
use bk_core::{HttpExchange, ProjectId};
use rusqlite::params;

/// Index a single exchange. Called from `exchanges::insert` and
/// `exchanges::update_notes` inside the same transaction. Uses
/// `REPLACE INTO` (internal-content FTS5, migration 002) so it's
/// idempotent: a fresh rowid inserts, an existing rowid replaces.
///
/// Bodies are decoded with `String::from_utf8_lossy` — invalid UTF-8
/// bytes are replaced with U+FFFD, so indexing always proceeds
/// (rather than bailing on a decode error). The lossy replacement
/// is recorded in the FTS index and will match future searches the
/// same way it matches the stored value.
///
/// Takes a `&Transaction` (not `&Connection`) so the FTS write is
/// part of the same atomic unit as the `exchanges` insert/update.
/// If the FTS write fails, the whole insert/update rolls back.
pub fn index_exchange(conn: &rusqlite::Transaction<'_>, ex: &HttpExchange) -> Result<()> {
    let rowid: i64 = conn.query_row(
        "SELECT rowid FROM exchanges WHERE id = ?1",
        params![ex.meta.id.to_string()],
        |r| r.get(0),
    )?;
    let url_str = ex.request.url.as_str();
    let method = ex.request.method.as_str();
    let req_headers = headers_to_string(&ex.request.headers);
    let resp_headers = ex
        .response
        .as_ref()
        .map(|r| headers_to_string(&r.headers))
        .unwrap_or_default();
    let req_body = body_to_string(&ex.request.body);
    let resp_body = ex
        .response
        .as_ref()
        .map(|r| body_to_string(&r.body))
        .unwrap_or_default();

    // REPLACE INTO is idempotent: if a row with this rowid exists
    // (from a prior insert or a previous index_exchange call), it's
    // replaced; otherwise a new row is inserted. This is what makes
    // `update_notes` work cleanly — we can just re-call
    // `index_exchange` after the UPDATE without first issuing a
    // 'delete' command. Requires internal-content FTS5 (migration 002).
    conn.execute(
        r#"REPLACE INTO exchange_fts
           (rowid, url, method, request_headers, response_headers, request_body, response_body, notes)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"#,
        rusqlite::params![
            rowid,
            url_str,
            method,
            req_headers,
            resp_headers,
            req_body,
            resp_body,
            ex.meta.notes,
        ],
    )?;
    Ok(())
}

/// Search for a query string. Returns the matching `exchange_id` values
/// ranked by FTS5's BM25 score (best first). The caller is responsible
/// for hydrating the full exchanges from these IDs via
/// `exchanges::get` (or a future batch helper).
///
/// The query syntax is FTS5's MATCH syntax: tokens, phrases ("..."),
/// AND/OR/NOT, column filters (e.g., `url:admin`). We do not sanitize
/// user input — the UI is expected to escape special characters or
/// pass a simple tokenized query. This is safe from SQL injection
/// because we use a prepared statement, but malformed FTS5 queries
/// return an error which the UI surfaces to the user.
pub fn search(
    pool: &DbPool,
    project_id: ProjectId,
    query: &str,
    limit: u32,
) -> Result<Vec<bk_core::ExchangeId>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        r#"SELECT exchanges.id
           FROM exchange_fts
           INNER JOIN exchanges ON exchanges.rowid = exchange_fts.rowid
           WHERE exchange_fts MATCH ?1
             AND exchanges.project_id = ?2
           ORDER BY bm25(exchange_fts) ASC
           LIMIT ?3"#,
    )?;
    let rows = stmt.query_map(params![query, project_id.to_string(), limit as i64], |r| {
        let id_str: String = r.get(0)?;
        Ok(id_str)
    })?;
    let mut out = Vec::new();
    for r in rows {
        let id_str = r?;
        out.push(id_str.parse()?);
    }
    Ok(out)
}

/// Rebuild the FTS5 index for an entire project. Used after a schema
/// upgrade that adds a new indexed field, or as a recovery tool from
/// a `talon repair-index <project>` CLI command.
///
/// With internal-content FTS5 (migration 002), this is straightforward:
/// delete every FTS row in the project, then re-insert from the
/// `exchanges` table.
pub fn rebuild(pool: &DbPool, project_id: ProjectId) -> Result<usize> {
    let mut conn = pool.get()?;
    let tx = conn.transaction()?;
    // Wipe this project's FTS rows. We need the rowids to filter by
    // project, then delete by rowid.
    let mut existing = tx.prepare("SELECT rowid FROM exchanges WHERE project_id = ?1")?;
    let rowids: Vec<i64> = existing
        .query_map(params![project_id.to_string()], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    drop(existing);
    for rowid in &rowids {
        tx.execute("DELETE FROM exchange_fts WHERE rowid = ?1", params![rowid])?;
    }
    // Re-insert from the exchanges table.
    let mut stmt = tx.prepare(
        "SELECT id, project_id, timestamp, duration_ns, summary, scope_state, notes, starred, blocked_reason, request_json, response_json \
         FROM exchanges WHERE project_id = ?1",
    )?;
    let exchanges: Vec<HttpExchange> = stmt
        .query_map(
            params![project_id.to_string()],
            crate::exchanges::row_to_exchange,
        )?
        .collect::<rusqlite::Result<_>>()?;
    drop(stmt);
    for ex in &exchanges {
        index_exchange(&tx, ex)?;
    }
    tx.commit()?;
    Ok(exchanges.len())
}

fn headers_to_string(headers: &bk_core::HeaderMap) -> String {
    headers
        .iter()
        .map(|(k, v)| format!("{}: {}", k, v.to_str().unwrap_or("<binary>")))
        .collect::<Vec<_>>()
        .join("\n")
}

fn body_to_string(body: &bk_core::Body) -> String {
    use bk_core::Body;
    match body {
        Body::Complete { data } => String::from_utf8_lossy(data).into_owned(),
        Body::Empty => String::new(),
        Body::Streaming { .. } => String::new(), // can't index what we haven't read
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bk_core::{
        Body, ExchangeMeta, HeaderMap, HttpExchange, Method, ProjectId, Request, Response,
        ScopeState, Url, Version,
    };
    use chrono::Utc;
    use tempfile::TempDir;

    fn fresh_pool() -> (TempDir, DbPool) {
        let tmp = TempDir::new().unwrap();
        let pool = crate::db::open(tmp.path().join("test.db")).unwrap();
        (tmp, pool)
    }

    /// Insert a minimal `projects` row so the FK constraint on
    /// `exchanges.project_id` is satisfied.
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

    fn make_exchange(project_id: ProjectId, path: &str, body: &str) -> HttpExchange {
        let url: Url = format!("https://acme.bb{path}").parse().unwrap();
        let mut req_headers = HeaderMap::new();
        req_headers.insert("user-agent", "talon-test/0.1".parse().unwrap());
        let mut resp_headers = HeaderMap::new();
        resp_headers.insert("content-type", "application/json".parse().unwrap());

        HttpExchange {
            meta: ExchangeMeta {
                id: bk_core::ExchangeId::new(),
                project_id,
                timestamp: Utc::now(),
                duration_ns: 0,
                summary: format!("GET {path}"),
                scope_state: ScopeState::InScope,
                notes: String::new(),
                starred: false,
            },
            request: Request {
                method: Method::GET,
                url,
                version: Version::HTTP_11,
                headers: req_headers,
                body: Body::empty(),
            },
            response: Some(Response {
                version: Version::HTTP_11,
                status: 200,
                status_text: "OK".to_string(),
                headers: resp_headers,
                // The plan passes `body: &str` and tries to use it
                // directly as `body.as_bytes()`; `Body::from_bytes` wants
                // an owned type. Convert here so the test fixture's
                // owned String is moved into the response, not borrowed.
                body: Body::from_bytes(body.as_bytes().to_vec()),
            }),
            blocked_reason: None,
        }
    }

    #[test]
    fn search_finds_url_token() {
        let (_tmp, pool) = fresh_pool();
        let project_id = ProjectId::new();
        insert_project_row(&pool, project_id);
        crate::exchanges::insert(&pool, &make_exchange(project_id, "/api/users", "[]")).unwrap();
        crate::exchanges::insert(&pool, &make_exchange(project_id, "/api/orders", "[]")).unwrap();
        crate::exchanges::insert(
            &pool,
            &make_exchange(project_id, "/static/app.js", "/* js */"),
        )
        .unwrap();

        let hits = search(&pool, project_id, "users", 10).unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn search_finds_body_token() {
        let (_tmp, pool) = fresh_pool();
        let project_id = ProjectId::new();
        insert_project_row(&pool, project_id);
        crate::exchanges::insert(
            &pool,
            &make_exchange(project_id, "/api", r#"{"secret":"hunter2"}"#),
        )
        .unwrap();
        crate::exchanges::insert(
            &pool,
            &make_exchange(project_id, "/api", r#"{"public":"data"}"#),
        )
        .unwrap();

        let hits = search(&pool, project_id, "hunter2", 10).unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn search_does_not_cross_projects() {
        let (_tmp, pool) = fresh_pool();
        let p1 = ProjectId::new();
        let p2 = ProjectId::new();
        insert_project_row(&pool, p1);
        insert_project_row(&pool, p2);
        crate::exchanges::insert(&pool, &make_exchange(p1, "/admin", "[]")).unwrap();
        crate::exchanges::insert(&pool, &make_exchange(p2, "/admin", "[]")).unwrap();

        let p1_hits = search(&pool, p1, "admin", 10).unwrap();
        let p2_hits = search(&pool, p2, "admin", 10).unwrap();
        assert_eq!(p1_hits.len(), 1);
        assert_eq!(p2_hits.len(), 1);
        assert_ne!(p1_hits[0], p2_hits[0]);
    }

    #[test]
    fn search_respects_limit() {
        let (_tmp, pool) = fresh_pool();
        let project_id = ProjectId::new();
        insert_project_row(&pool, project_id);
        for i in 0..5 {
            crate::exchanges::insert(
                &pool,
                &make_exchange(project_id, &format!("/api/{i}"), "[]"),
            )
            .unwrap();
        }
        let hits = search(&pool, project_id, "api", 3).unwrap();
        assert_eq!(hits.len(), 3);
    }

    #[test]
    fn rebuild_reindexes_all_exchanges() {
        let (_tmp, pool) = fresh_pool();
        let project_id = ProjectId::new();
        insert_project_row(&pool, project_id);
        crate::exchanges::insert(&pool, &make_exchange(project_id, "/users", "alice")).unwrap();
        crate::exchanges::insert(&pool, &make_exchange(project_id, "/orders", "bob")).unwrap();
        crate::exchanges::insert(&pool, &make_exchange(project_id, "/payments", "carol")).unwrap();

        // Sanity: all three are searchable before rebuild.
        let before = search(&pool, project_id, "alice OR bob OR carol", 10).unwrap();
        assert_eq!(before.len(), 3);

        // Rebuild and re-query. With internal-content FTS5
        // (migration 002), `rebuild` does DELETE + REPLACE for each
        // row, so the post-rebuild search must find the same 3 hits.
        let count = rebuild(&pool, project_id).unwrap();
        assert_eq!(count, 3);

        let after = search(&pool, project_id, "alice OR bob OR carol", 10).unwrap();
        assert_eq!(after.len(), 3);
    }
}
