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
/// We use `INSERT OR REPLACE` rather than `INSERT OR IGNORE` so the
/// `updated_at` / `name` / `target_host` get refreshed when a project
/// is re-opened after edits. The other fields (created_at,
/// db_filename, talon_version, ca_fingerprint) are set on the first
/// insert and the `WHERE` semantics of `REPLACE` will overwrite them
/// — for the engine's first open that's exactly what we want.
pub fn upsert(pool: &DbPool, info: &ProjectInfo) -> Result<()> {
    let conn = pool.get()?;
    conn.execute(
        "INSERT OR REPLACE INTO projects
            (id, name, target_host, created_at, updated_at, db_filename, talon_version, ca_fingerprint)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
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
