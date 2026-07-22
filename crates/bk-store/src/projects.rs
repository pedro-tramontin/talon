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
//!
//! ## `update_settings` (Phase 6 Part C, §C-A.1)
//!
//! `update_settings` persists the `ProjectSettings` blob (scope
//! rules + M&R rules + theme + proxy_enabled) to the
//! `projects.settings_json` column. The engine's
//! `Engine::save_settings` method calls this on every CRUD mutation
//! to the in-memory `ProjectSettings` cache. The on-disk
//! representation is the same JSON the in-memory
//! `ProjectSettings` serializes to via `serde_json` (the column's
//! `DEFAULT '{}'` matches `ProjectSettings::default()`).

#![allow(missing_docs)]

use crate::error::Result;
use crate::DbPool;
use bk_core::{ProjectId, ProjectInfo, ProjectSettings};
use rusqlite::params;
use rusqlite::OptionalExtension;

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
///
/// **`created_at` is intentionally not updated on conflict** — it
/// represents the project's creation timestamp (see
/// `bk_core::ProjectInfo`) and must be immutable. The first INSERT
/// sets it; subsequent re-opens preserve it. Other fields
/// (`name`, `target_host`, `updated_at`, `db_filename`,
/// `talon_version`, `ca_fingerprint`) are refreshed on conflict.
pub fn upsert(pool: &DbPool, info: &ProjectInfo) -> Result<()> {
    let conn = pool.get()?;
    conn.execute(
        "INSERT INTO projects
            (id, name, target_host, created_at, updated_at, db_filename, talon_version, ca_fingerprint)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            target_host = excluded.target_host,
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

/// Persist the `ProjectSettings` blob to the `settings_json`
/// column for a given project. The engine's
/// `Engine::save_settings` calls this on every CRUD mutation.
///
/// **Idempotency:** the UPDATE is a full-replace of the
/// `settings_json` column. Re-running with the same `settings`
/// is a no-op (the JSON is identical). The `ProjectInfo` side
/// is NOT touched — the `upsert` method owns the info columns;
/// this method owns the settings column.
///
/// **Why not in the `upsert` `ON CONFLICT` clause?** because the
/// settings are owned by the engine, not by `ProjectInfo`. A
/// user who calls `upsert` with a freshly-stamped `ProjectInfo`
/// shouldn't accidentally clobber their settings with the
/// `ProjectSettings::default()` they passed in.
pub fn update_settings(
    pool: &DbPool,
    project_id: ProjectId,
    settings: &ProjectSettings,
) -> Result<()> {
    let conn = pool.get()?;
    let settings_json = serde_json::to_string(settings)
        .map_err(|e| crate::StoreError::Invalid(format!("settings JSON serialize: {e}")))?;
    let updated = conn.execute(
        "UPDATE projects
         SET settings_json = ?1
         WHERE id = ?2",
        params![settings_json, project_id.to_string()],
    )?;
    if updated == 0 {
        // No row matched: the project isn't in the `projects`
        // table yet. Caller (the engine) should ensure the
        // project is `upsert`ed before calling
        // `update_settings`.
        return Err(crate::StoreError::Invalid(format!(
            "update_settings: project {project_id} not found in projects table"
        )));
    }
    Ok(())
}

/// Read the `ProjectSettings` blob from the `settings_json`
/// column. Returns `ProjectSettings::default()` if the column
/// is empty (the schema's `DEFAULT '{}'` guarantees this for
/// Read the `ProjectSettings` blob from the `settings_json`
/// column. Returns `ProjectSettings::default()` if the column
/// is empty or unparseable (the schema's `DEFAULT '{}'` gives
/// us `'{}'` for fresh rows; the engine path that goes through
/// `update_settings` writes the proper default JSON, so a
/// `'{}'` value only appears for pre-migration-001 DBs).
pub fn read_settings(pool: &DbPool, project_id: ProjectId) -> Result<ProjectSettings> {
    let conn = pool.get()?;
    let json: Option<String> = conn
        .query_row(
            "SELECT settings_json FROM projects WHERE id = ?1",
            params![project_id.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    match json {
        None => Err(crate::StoreError::Invalid(format!(
            "read_settings: project {project_id} not found"
        ))),
        Some(s) => {
            // The schema's DEFAULT is `'{}'` (set by migration 001
            // before `ProjectSettings` existed). The current
            // `ProjectSettings::default()` serializes to a longer
            // JSON with all 4 fields. For a `'{}'` value, return
            // `ProjectSettings::default()` so callers don't have
            // to special-case the empty blob.
            if s.is_empty() || s == "{}" {
                return Ok(ProjectSettings::default());
            }
            serde_json::from_str(&s)
                .map_err(|e| crate::StoreError::Invalid(format!("settings JSON parse: {e}")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exchanges;
    use bk_core::{
        Body, ExchangeMeta, HeaderMap, HttpExchange, Method, ProjectSettings, Request, ScopeState,
        Theme,
    };
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

    /// Regression: `upsert` previously included `created_at` in the
    /// `ON CONFLICT DO UPDATE` SET clause, so a re-upsert with a
    /// freshly-stamped `Project` would overwrite the original
    /// creation timestamp. `created_at` must be immutable: the
    /// first INSERT sets it, and subsequent re-opens preserve it.
    #[test]
    fn upsert_preserves_created_at_across_reopen() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.db");
        let mut info = make_info();
        // Pin the original creation timestamp to a known value.
        info.created_at = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let pool = crate::open(&path).unwrap();
        upsert(&pool, &info).unwrap();

        // Re-upsert with a different `created_at` (simulating a
        // caller that re-stamps the timestamp on reopen, which is
        // a bug pattern).
        info.created_at = chrono::DateTime::parse_from_rfc3339("2099-12-31T23:59:59Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        upsert(&pool, &info).unwrap();

        // Read the row back and assert the original `created_at` survived.
        let conn = pool.get().unwrap();
        let stored: String = conn
            .query_row(
                "SELECT created_at FROM projects WHERE id = ?1",
                rusqlite::params![info.id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            stored, "2026-01-01T00:00:00+00:00",
            "created_at must be immutable across re-upserts"
        );
    }

    // -----------------------------------------------------------------------
    // §C-A.1 tests — `update_settings` / `read_settings` round-trip
    // -----------------------------------------------------------------------

    /// `update_settings` round-trips: write a non-default
    /// `ProjectSettings`, read it back, the scope_rules +
    /// M&R rules are preserved.
    #[test]
    fn update_settings_round_trips() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.db");
        let info = make_info();
        let pid = info.id;
        let pool = crate::open(&path).unwrap();
        upsert(&pool, &info).unwrap();

        let settings = ProjectSettings {
            theme: Theme::Dark,
            proxy_enabled: false,
            scope_rules: vec![bk_core::ScopeRule {
                kind: bk_core::ScopeRuleKind::Host,
                pattern: "acme.bb".to_string(),
                action: bk_core::MatchAction::InScope,
                label: "primary".to_string(),
                priority: 10,
            }],
            match_replace_rules: vec![bk_core::MatchReplaceRule {
                target: bk_core::MatchReplaceTarget::RequestUrl,
                pattern: "old".to_string(),
                replace: "new".to_string(),
                is_regex: false,
                case_insensitive: false,
                enabled: true,
                priority: 0,
            }],
        };
        update_settings(&pool, pid, &settings).unwrap();

        let read = read_settings(&pool, pid).unwrap();
        assert_eq!(read.theme, Theme::Dark);
        assert!(!read.proxy_enabled);
        assert_eq!(read.scope_rules.len(), 1);
        assert_eq!(read.scope_rules[0].label, "primary");
        assert_eq!(read.scope_rules[0].priority, 10);
        assert_eq!(read.match_replace_rules.len(), 1);
        assert_eq!(read.match_replace_rules[0].pattern, "old");
    }

    /// `update_settings` is idempotent: calling it twice with
    /// the same settings is a no-op (no error, same result).
    #[test]
    fn update_settings_idempotent() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.db");
        let info = make_info();
        let pid = info.id;
        let pool = crate::open(&path).unwrap();
        upsert(&pool, &info).unwrap();

        let settings = ProjectSettings {
            theme: Theme::Light,
            ..Default::default()
        };
        update_settings(&pool, pid, &settings).unwrap();
        update_settings(&pool, pid, &settings).unwrap();
        let read = read_settings(&pool, pid).unwrap();
        assert_eq!(read.theme, Theme::Light);
    }

    /// `update_settings` errors on a project that doesn't
    /// exist in the `projects` table (no row to UPDATE).
    #[test]
    fn update_settings_errors_on_missing_project() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.db");
        let pool = crate::open(&path).unwrap();
        let unknown_pid = bk_core::ProjectId::new();
        let res = update_settings(&pool, unknown_pid, &ProjectSettings::default());
        assert!(res.is_err());
        assert!(format!("{res:?}").contains("not found"));
    }

    /// `read_settings` on a freshly-upserted project (no
    /// explicit `update_settings` call) returns
    /// `ProjectSettings::default()` — the schema's
    /// `DEFAULT '{}'` makes this work.
    #[test]
    fn read_settings_returns_default_for_fresh_project() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.db");
        let info = make_info();
        let pid = info.id;
        let pool = crate::open(&path).unwrap();
        upsert(&pool, &info).unwrap();
        let read = read_settings(&pool, pid).unwrap();
        assert_eq!(read, ProjectSettings::default());
    }

    /// `update_settings` is independent of `ProjectInfo`:
    /// updating the settings doesn't clobber the info
    /// columns, and vice versa.
    #[test]
    fn update_settings_does_not_touch_info_columns() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.db");
        let info = make_info();
        let pid = info.id;
        let pool = crate::open(&path).unwrap();
        upsert(&pool, &info).unwrap();

        let settings = ProjectSettings {
            theme: Theme::Dark,
            ..Default::default()
        };
        update_settings(&pool, pid, &settings).unwrap();

        // Re-upsert info: the settings must NOT be reset.
        upsert(&pool, &info).unwrap();
        let read = read_settings(&pool, pid).unwrap();
        assert_eq!(
            read.theme,
            Theme::Dark,
            "upsert must not reset settings_json"
        );
    }
}
