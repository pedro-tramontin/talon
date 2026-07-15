//! Typed CRUD for `ProjectInfo`. The engine's `Engine::open_project`
//! calls `upsert` so the `projects` row always exists when the first
//! `exchanges` insert fires (and the FK constraint on
//! `exchanges.project_id REFERENCES projects(id)` is satisfied).
//!
//! The plan (Part B §2.10) didn't separate this into its own module,
//! but §2.10's `engine_smoke_test` immediately hit a FK violation
//! without it — the engine owns the `Project` model, so the engine
//! is the right place to persist it. Keeping the SQL here mirrors
//! the `exchanges.rs` / `tags.rs` / `fts.rs` shape.

#![allow(missing_docs)]

use crate::error::Result;
use crate::DbPool;
use bk_core::ProjectInfo;
use rusqlite::params;

/// Insert or update the `projects` row for a given `ProjectInfo`.
/// Idempotent: calling it twice with the same `id` is fine.
///
/// Uses `INSERT ... ON CONFLICT DO UPDATE` — a true upsert that
/// only updates the existing row in place. We deliberately do NOT
/// use `INSERT OR REPLACE` here: `REPLACE` deletes the conflicting
/// row first, and the `exchanges` table has a
/// `project_id ... REFERENCES projects(id) ON DELETE CASCADE`
/// constraint. The cascading delete would wipe every exchange in
/// the project on every `open_project` call — and the
/// `engine_persists_across_restart` test caught exactly that.
pub fn upsert(pool: &DbPool, info: &ProjectInfo) -> Result<()> {
    let conn = pool.get()?;
    conn.execute(
        "INSERT INTO projects
            (id, name, target_host, created_at, updated_at, db_filename, talon_version, ca_fingerprint)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            target_host = excluded.target_host,
            created_at = excluded.created_at,
            updated_at = excluded.updated_at,
            db_filename = excluded.db_filename,
            talon_version = excluded.talon_version,
            ca_fingerprint = excluded.ca_fingerprint",
        params![
            info.id.to_string(),
            info.name,
            info.target_host,
            info.created_at.to_rfc3339(),
            info.updated_at.to_rfc3339(),
            info.db_filename,
            info.talon_version,
            info.ca_fingerprint,
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exchanges;
    use bk_core::{Body, ExchangeMeta, HeaderMap, HttpExchange, Method, Request, ScopeState};
    use tempfile::TempDir;

    fn make_info() -> ProjectInfo {
        let now = chrono::Utc::now();
        ProjectInfo {
            id: bk_core::ProjectId::new(),
            name: "test".into(),
            target_host: "test.bb".into(),
            created_at: now,
            updated_at: now,
            db_filename: "test.db".into(),
            talon_version: "0.1.0".into(),
            ca_fingerprint: None,
        }
    }

    fn make_exchange(project_id: bk_core::ProjectId, path: &str) -> HttpExchange {
        HttpExchange {
            meta: ExchangeMeta {
                id: bk_core::ExchangeId::new(),
                project_id,
                timestamp: chrono::Utc::now(),
                duration_ns: 0,
                summary: format!("GET {path}"),
                scope_state: ScopeState::InScope,
                notes: String::new(),
                starred: false,
            },
            request: Request {
                method: Method::GET,
                url: format!("https://test.bb{path}").parse().unwrap(),
                version: bk_core::Version::HTTP_11,
                headers: HeaderMap::new(),
                body: Body::empty(),
            },
            response: None,
            blocked_reason: None,
        }
    }

    /// Regression: a previous version of `upsert` used
    /// `INSERT OR REPLACE` which, combined with the
    /// `exchanges.project_id ... ON DELETE CASCADE` constraint,
    /// wiped every exchange on every reopen. This test asserts
    /// that calling `upsert` twice on the same `id` preserves
    /// existing exchanges.
    #[test]
    fn upsert_preserves_exchanges_across_reopen() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.db");
        let info = make_info();
        let pid = info.id;

        let pool = crate::open(&path).unwrap();
        upsert(&pool, &info).unwrap();
        for p in ["/a", "/b", "/c"] {
            exchanges::insert(&pool, &make_exchange(pid, p)).unwrap();
        }
        assert_eq!(exchanges::list_recent(&pool, pid, 10).unwrap().len(), 3);

        // Re-upsert: should NOT cascade-delete the exchanges.
        upsert(&pool, &info).unwrap();
        assert_eq!(
            exchanges::list_recent(&pool, pid, 10).unwrap().len(),
            3,
            "upsert must not wipe exchanges via ON DELETE CASCADE"
        );
    }
}
