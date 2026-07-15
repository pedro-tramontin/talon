//! Tag CRUD. Tags are per-project, named labels that can be attached
//! to exchanges. The pre-defined tags (🔴 vuln, 🔑 creds, 📌 interesting,
//! ⏭ ignore) are just regular tag rows seeded by the UI on first run.
//!
//! The "starred" flag on `exchanges` is intentionally separate from the
//! tags system — it's a fast boolean for the ⭐ column in the UI. If
//! you want a custom "starred" tag, create one and use it; don't
//! overload the `starred` column.

#![allow(missing_docs)]

use crate::error::{Result, StoreError};
use crate::DbPool;
use bk_core::{ExchangeId, ProjectId, Tag, TagId};
use rusqlite::{params, OptionalExtension};

#[derive(Debug, Clone)]
pub struct NewTag {
    pub name: String,
    /// e.g. "#ef4444" or null = default
    pub color: Option<String>,
}

/// Create a tag, or fetch the existing one if the name is taken
/// within the same project. Idempotent. Returns the tag's ID either way.
///
/// Concurrency: uses `INSERT OR IGNORE` with a pre-generated ID, then
/// falls back to a `SELECT` for the existing row. This is safe under
/// concurrent calls — if two threads both see "no existing row" and
/// both try to INSERT, one INSERT wins and the other is ignored (no
/// `UNIQUE` violation). Both threads then `SELECT` and converge on the
/// same ID. The previous read-then-INSERT pattern could race: both
/// threads could observe "no existing row" before either INSERT, and
/// the loser would hit the `UNIQUE (project_id, name)` constraint.
pub fn upsert(pool: &DbPool, project_id: ProjectId, new: &NewTag) -> Result<TagId> {
    let conn = pool.get()?;
    let new_id = TagId::new();
    let inserted = conn.execute(
        "INSERT OR IGNORE INTO tags (id, project_id, name, color) VALUES (?1, ?2, ?3, ?4)",
        params![
            new_id.to_string(),
            project_id.to_string(),
            new.name,
            new.color
        ],
    )?;
    if inserted == 1 {
        return Ok(new_id);
    }
    // Our INSERT was ignored — a concurrent caller won. Look up the
    // existing row by (project_id, name).
    let id_str: Option<String> = conn
        .query_row(
            "SELECT id FROM tags WHERE project_id = ?1 AND name = ?2",
            params![project_id.to_string(), new.name],
            |r| r.get(0),
        )
        .optional()?;
    let id_str = id_str.ok_or_else(|| {
        // Should be unreachable: INSERT was ignored *because* a row
        // already exists. If it doesn't, the row was deleted between
        // the INSERT and the SELECT — treat as a transient race and
        // surface NotFound rather than panicking.
        StoreError::NotFound(format!("tag '{}' in project {project_id}", new.name))
    })?;
    id_str.parse().map_err(StoreError::from)
}

/// List all tags for a project, alphabetical by name.
pub fn list(pool: &DbPool, project_id: ProjectId) -> Result<Vec<Tag>> {
    let conn = pool.get()?;
    let mut stmt =
        conn.prepare("SELECT id, name, color FROM tags WHERE project_id = ?1 ORDER BY name")?;
    let rows = stmt.query_map(params![project_id.to_string()], |r| {
        let id_str: String = r.get(0)?;
        let name: String = r.get(1)?;
        let color: Option<String> = r.get(2)?;
        Ok((id_str, name, color))
    })?;
    let mut out = Vec::new();
    for r in rows {
        let (id_str, name, color) = r?;
        let id: TagId = id_str.parse()?;
        out.push(Tag { id, name, color });
    }
    Ok(out)
}

/// Attach a tag to an exchange. Idempotent (no-op if already attached).
pub fn attach(pool: &DbPool, tag_id: TagId, exchange_id: ExchangeId) -> Result<()> {
    let conn = pool.get()?;
    conn.execute(
        "INSERT OR IGNORE INTO exchange_tags (exchange_id, tag_id) VALUES (?1, ?2)",
        params![exchange_id.to_string(), tag_id.to_string()],
    )?;
    Ok(())
}

/// Detach a tag from an exchange. No-op if not attached.
pub fn detach(pool: &DbPool, tag_id: TagId, exchange_id: ExchangeId) -> Result<()> {
    let conn = pool.get()?;
    conn.execute(
        "DELETE FROM exchange_tags WHERE exchange_id = ?1 AND tag_id = ?2",
        params![exchange_id.to_string(), tag_id.to_string()],
    )?;
    Ok(())
}

/// List the tags currently attached to an exchange. Used by the UI's
/// tag-picker to show the current state.
pub fn list_for_exchange(pool: &DbPool, exchange_id: ExchangeId) -> Result<Vec<Tag>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        r#"SELECT t.id, t.name, t.color
           FROM tags t
           INNER JOIN exchange_tags et ON et.tag_id = t.id
           WHERE et.exchange_id = ?1
           ORDER BY t.name"#,
    )?;
    let rows = stmt.query_map(params![exchange_id.to_string()], |r| {
        let id_str: String = r.get(0)?;
        let name: String = r.get(1)?;
        let color: Option<String> = r.get(2)?;
        Ok((id_str, name, color))
    })?;
    let mut out = Vec::new();
    for r in rows {
        let (id_str, name, color) = r?;
        let id: TagId = id_str.parse()?;
        out.push(Tag { id, name, color });
    }
    Ok(out)
}

/// Delete a tag and all its exchange attachments (CASCADE).
pub fn delete(pool: &DbPool, id: TagId) -> Result<()> {
    let conn = pool.get()?;
    let n = conn.execute("DELETE FROM tags WHERE id = ?1", params![id.to_string()])?;
    if n == 0 {
        return Err(StoreError::NotFound(id.to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bk_core::ExchangeId;
    use tempfile::TempDir;

    fn fresh_pool() -> (TempDir, DbPool) {
        let tmp = TempDir::new().unwrap();
        let pool = crate::db::open(tmp.path().join("test.db")).unwrap();
        (tmp, pool)
    }

    /// Insert a minimal `projects` row so the FK constraint on
    /// `exchanges.project_id` is satisfied. Same helper as in
    /// `exchanges::tests`; copied here to keep the test module
    /// self-contained (the storage crate's test modules don't share).
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

    /// Insert a minimal `exchanges` row directly via SQL — the
    /// `exchanges::insert` function wants a full `HttpExchange` and
    /// these tests only care about the tag join table, not the
    /// exchange content.
    fn insert_exchange_row(pool: &DbPool, id: ExchangeId, project_id: ProjectId) {
        let conn = pool.get().unwrap();
        conn.execute(
            r#"INSERT INTO exchanges
               (id, project_id, timestamp, summary, scope_state, request_json)
               VALUES (?1, ?2, ?3, 'GET /x', 'in_scope', '{}')"#,
            rusqlite::params![
                id.to_string(),
                project_id.to_string(),
                "2026-01-01T00:00:00Z"
            ],
        )
        .expect("insert exchange row");
    }

    #[test]
    fn upsert_is_idempotent_within_project() {
        let (_tmp, pool) = fresh_pool();
        let project_id = ProjectId::new();
        insert_project_row(&pool, project_id);
        let id1 = upsert(
            &pool,
            project_id,
            &NewTag {
                name: "vuln".into(),
                color: None,
            },
        )
        .unwrap();
        let id2 = upsert(
            &pool,
            project_id,
            &NewTag {
                name: "vuln".into(),
                color: None,
            },
        )
        .unwrap();
        assert_eq!(id1, id2);
        assert_eq!(list(&pool, project_id).unwrap().len(), 1);
    }

    #[test]
    fn attach_and_detach_modify_exchange_tags() {
        let (_tmp, pool) = fresh_pool();
        let project_id = ProjectId::new();
        insert_project_row(&pool, project_id);
        let exchange_id = ExchangeId::new();
        insert_exchange_row(&pool, exchange_id, project_id);

        let tag = upsert(
            &pool,
            project_id,
            &NewTag {
                name: "creds".into(),
                color: Some("#22d3ee".into()),
            },
        )
        .unwrap();
        attach(&pool, tag, exchange_id).unwrap();
        let attached = list_for_exchange(&pool, exchange_id).unwrap();
        assert_eq!(attached.len(), 1);
        assert_eq!(attached[0].name, "creds");
        assert_eq!(attached[0].color.as_deref(), Some("#22d3ee"));

        detach(&pool, tag, exchange_id).unwrap();
        assert_eq!(list_for_exchange(&pool, exchange_id).unwrap().len(), 0);
    }

    #[test]
    fn delete_tag_cascades_attachments() {
        let (_tmp, pool) = fresh_pool();
        let project_id = ProjectId::new();
        insert_project_row(&pool, project_id);
        let exchange_id = ExchangeId::new();
        insert_exchange_row(&pool, exchange_id, project_id);

        let tag = upsert(
            &pool,
            project_id,
            &NewTag {
                name: "vuln".into(),
                color: None,
            },
        )
        .unwrap();
        attach(&pool, tag, exchange_id).unwrap();
        delete(&pool, tag).unwrap();
        assert_eq!(list_for_exchange(&pool, exchange_id).unwrap().len(), 0);
    }

    #[test]
    fn upsert_across_projects_creates_separate_tags() {
        let (_tmp, pool) = fresh_pool();
        let p1 = ProjectId::new();
        let p2 = ProjectId::new();
        insert_project_row(&pool, p1);
        insert_project_row(&pool, p2);
        let id1 = upsert(
            &pool,
            p1,
            &NewTag {
                name: "vuln".into(),
                color: None,
            },
        )
        .unwrap();
        let id2 = upsert(
            &pool,
            p2,
            &NewTag {
                name: "vuln".into(),
                color: None,
            },
        )
        .unwrap();
        // Same name, but different projects → different IDs.
        assert_ne!(id1, id2);
    }

    /// Regression: the previous read-then-INSERT pattern could race
    /// when two threads both saw "no existing row" before either
    /// INSERTed, causing one to hit `UNIQUE (project_id, name)`. The
    /// fix uses `INSERT OR IGNORE` then `SELECT` to converge. All
    /// concurrent upserts for the same `(project_id, name)` must
    /// return the same ID and not error.
    #[test]
    fn upsert_concurrent_callers_converge_on_same_id() {
        use std::sync::{Arc, Barrier};
        use std::thread;

        let (_tmp, pool) = fresh_pool();
        let project_id = ProjectId::new();
        insert_project_row(&pool, project_id);

        let n = 8usize;
        let pool = Arc::new(pool);
        let barrier = Arc::new(Barrier::new(n));
        let mut handles = Vec::new();
        for _ in 0..n {
            let pool = Arc::clone(&pool);
            let b = Arc::clone(&barrier);
            let handle = thread::spawn(move || {
                b.wait();
                upsert(
                    &pool,
                    project_id,
                    &NewTag {
                        name: "race-target".into(),
                        color: None,
                    },
                )
            });
            handles.push(handle);
        }

        let mut iter = handles.into_iter();
        let first = iter.next().unwrap().join().expect("join").expect("upsert");
        for h in iter {
            let id = h.join().expect("join").expect("upsert");
            assert_eq!(id, first, "concurrent upserts must converge on the same ID");
        }
        // And only one row was created.
        assert_eq!(list(&pool, project_id).unwrap().len(), 1);
    }
}
