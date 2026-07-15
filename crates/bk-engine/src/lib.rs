//! Public engine API for Talon. The Tauri shell and the axum browser-server
//! call into this crate.

pub mod engine;
pub mod error;
pub mod projects;

pub use engine::Engine;
pub use error::{EngineError, Result};
pub use projects::Projects;

#[cfg(test)]
mod tests {
    use super::*;
    use bk_core::{Body, ExchangeMeta, HeaderMap, HttpExchange, Method, Request, ScopeState};
    use tempfile::TempDir;

    /// Build a minimal `HttpExchange` for the given project + path.
    /// Tests use this to keep the per-test body to "the interesting
    /// part" (the assertion) rather than 25 lines of struct literals.
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
                url: format!("https://acme.bb{path}").parse().unwrap(),
                version: bk_core::Version::HTTP_11,
                headers: HeaderMap::new(),
                body: Body::empty(),
            },
            response: None,
            blocked_reason: None,
        }
    }

    /// End-to-end smoke: create an engine, open a project, insert an
    /// exchange, search for it, close the project, confirm it's gone
    /// from the open set. This is a thin integration test — the
    /// storage layer has its own fine-grained unit tests; here we
    /// just verify the engine's plumbing doesn't drop things.
    #[test]
    fn engine_smoke_test() {
        let tmp = TempDir::new().unwrap();
        let engine = Engine::new(tmp.path()).expect("create engine");
        let project = bk_core::Project::new("acme.bb", "acme.bb", "0.1.0");
        let project_id = project.info.id;

        // Open the project.
        let _pool = engine.open_project(&project).expect("open project");
        assert_eq!(engine.open_count(), 1);
        assert_eq!(engine.open_ids(), vec![project_id]);

        // Insert an exchange.
        let exchange = make_exchange(project_id, "/admin");
        engine
            .insert_exchange(project_id, &exchange)
            .expect("insert exchange");

        // Search for it.
        let hits = engine.search(project_id, "admin", 10).expect("search");
        assert_eq!(hits.len(), 1);

        // Get it back.
        let back = engine
            .get_exchange(project_id, exchange.meta.id)
            .expect("get")
            .expect("exchange exists");
        assert_eq!(back.meta.summary, "GET /admin");

        // Close the project.
        engine.close_project(project_id);
        assert_eq!(engine.open_count(), 0);
    }

    /// Closing a non-open project is idempotent.
    #[test]
    fn close_non_open_project_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let engine = Engine::new(tmp.path()).unwrap();
        engine.close_project(bk_core::ProjectId::new());
        assert_eq!(engine.open_count(), 0);
    }

    /// Inserting into a non-open project returns `ProjectNotOpen`.
    #[test]
    fn insert_into_non_open_project_errors() {
        let tmp = TempDir::new().unwrap();
        let engine = Engine::new(tmp.path()).unwrap();
        let project_id = bk_core::ProjectId::new();
        let exchange = make_exchange(project_id, "/x");
        let res = engine.insert_exchange(project_id, &exchange);
        assert!(matches!(res, Err(EngineError::ProjectNotOpen(_))));
    }

    /// Part B §2.11 — the deepest smoke test: prove the whole stack
    /// (bk-core model → bk-store SQL → on-disk SQLite → reopen →
    /// re-read) round-trips. Two engines point at the same config
    /// dir, the first inserts and closes, the second reopens and
    /// reads back. If anything in the JSON serialization, the FTS5
    /// sync, or the FK layout is wrong, this test catches it.
    ///
    /// This is also a regression test for a real bug we hit during
    /// §2.10: the original `bk_store::projects::upsert` used
    /// `INSERT OR REPLACE`, which on a row with the same primary
    /// key DELETES the existing row and re-inserts. The
    /// `exchanges` table's `project_id ... ON DELETE CASCADE` then
    /// wiped every exchange in the project on every reopen. The
    /// fix was to switch to `INSERT ... ON CONFLICT DO UPDATE`,
    /// which is a true in-place upsert. The companion regression
    /// test lives in `bk_store::projects::tests`.
    #[test]
    fn engine_persists_across_restart() {
        let tmp = TempDir::new().unwrap();
        let project = bk_core::Project::new("acme.bb", "acme.bb", "0.1.0");
        let project_id = project.info.id;

        // Session 1: insert three exchanges, then drop the engine.
        {
            let engine = Engine::new(tmp.path()).expect("engine 1");
            engine.open_project(&project).expect("open");
            for path in ["/login", "/admin", "/logout"] {
                let ex = make_exchange(project_id, path);
                engine
                    .insert_exchange(project_id, &ex)
                    .unwrap_or_else(|e| panic!("insert {path}: {e}"));
            }
            // list_recent should return all 3 (all have the same
            // timestamp, but each gets its own id).
            let recent = engine.list_recent(project_id, 10).expect("list");
            assert_eq!(recent.len(), 3);
            engine.close_project(project_id);
            assert_eq!(engine.open_count(), 0);
        }

        // Session 2: brand-new engine, same config dir, reopen the
        // same project. The data must survive — that's the whole
        // point of SQLite-backed persistence.
        {
            let engine2 = Engine::new(tmp.path()).expect("engine 2");
            engine2.open_project(&project).expect("reopen");
            assert_eq!(engine2.open_count(), 1);

            let recent = engine2.list_recent(project_id, 10).expect("list");
            assert_eq!(recent.len(), 3, "all 3 exchanges survived restart");

            // FTS5 index also survived (it lives in the same DB file).
            let hits = engine2.search(project_id, "admin", 10).expect("search");
            assert_eq!(hits.len(), 1, "FTS5 index was rebuilt on reopen");
        }
    }

    // ── Deferred Phase 8 engine methods (post-PR #11 follow-up) ─────
    //
    // The Phase 2 Part B plan listed 6 methods that wrap existing
    // bk_store functions but were deferred to "Phase 8" (axum browser
    // server) when the engine was first designed. The proxy (Phase 3)
    // also benefits from a few of these — e.g. `set_starred` to flag
    // important requests as the user clicks the star, `tag_attach`
    // to auto-tag fuzzer findings, `update_notes` to persist notes
    // typed in the right rail. Adding them now keeps the proxy's
    // Tauri command layer thin and lets the test suite cover the
    // wrappers directly.
    //
    // All 6 methods are simple pass-throughs to bk_store, but they
    // each have the same project-open invariant the existing methods
    // enforce (ProjectNotOpen if the project isn't open). The tests
    // below cover both the happy path and the invariant.

    /// `set_starred` toggles the flag and persists across restart.
    #[test]
    fn engine_set_starred_toggles_persist() {
        let tmp = TempDir::new().unwrap();
        let project = bk_core::Project::new("acme.bb", "acme.bb", "0.1.0");
        let project_id = project.info.id;

        let engine = Engine::new(tmp.path()).unwrap();
        engine.open_project(&project).unwrap();
        let ex = make_exchange(project_id, "/x");
        let id = ex.meta.id;
        engine.insert_exchange(project_id, &ex).unwrap();

        // Star it.
        engine.set_starred(project_id, id, true).unwrap();
        let back = engine.get_exchange(project_id, id).unwrap().unwrap();
        assert!(back.meta.starred, "starred after set_starred(true)");

        // Unstar it.
        engine.set_starred(project_id, id, false).unwrap();
        let back = engine.get_exchange(project_id, id).unwrap().unwrap();
        assert!(!back.meta.starred, "not starred after set_starred(false)");
    }

    /// `update_notes` persists notes and keeps the FTS5 index in sync
    /// (the proxy will use this for the right-rail notes editor).
    /// The "FTS5 in sync" part is the regression we hit in §2.9's
    /// Copilot review fix #4 — if notes are searchable after update,
    /// the FTS5 row was rebuilt correctly.
    #[test]
    fn engine_update_notes_persists_and_reindexes_fts() {
        let tmp = TempDir::new().unwrap();
        let project = bk_core::Project::new("acme.bb", "acme.bb", "0.1.0");
        let project_id = project.info.id;

        let engine = Engine::new(tmp.path()).unwrap();
        engine.open_project(&project).unwrap();
        let ex = make_exchange(project_id, "/api/users");
        let id = ex.meta.id;
        engine.insert_exchange(project_id, &ex).unwrap();

        // Before the note update, "needle" is not in any searchable field.
        let before = engine.search(project_id, "needle", 10).unwrap();
        assert!(
            before.is_empty(),
            "no hits for 'needle' before update_notes"
        );

        // Update with a note that contains "needle".
        engine
            .update_notes(project_id, id, "found the needle in the haystack")
            .unwrap();
        let back = engine.get_exchange(project_id, id).unwrap().unwrap();
        assert_eq!(back.meta.notes, "found the needle in the haystack");

        // After the update, the FTS5 index should have re-indexed and
        // "needle" should now match.
        let after = engine.search(project_id, "needle", 10).unwrap();
        assert_eq!(after.len(), 1, "FTS5 re-indexed after update_notes");
        assert_eq!(after[0], id, "FTS5 hit is the right exchange");
    }

    /// `delete_exchange` removes the exchange, the FTS5 row, and
    /// any exchange_tags join rows. After delete, `get_exchange`
    /// returns None and the search no longer finds it.
    #[test]
    fn engine_delete_exchange_removes_from_storage_and_fts() {
        let tmp = TempDir::new().unwrap();
        let project = bk_core::Project::new("acme.bb", "acme.bb", "0.1.0");
        let project_id = project.info.id;

        let engine = Engine::new(tmp.path()).unwrap();
        engine.open_project(&project).unwrap();
        let ex = make_exchange(project_id, "/admin");
        let id = ex.meta.id;
        engine.insert_exchange(project_id, &ex).unwrap();

        // Sanity: present before delete.
        assert!(engine.get_exchange(project_id, id).unwrap().is_some());
        assert_eq!(engine.search(project_id, "admin", 10).unwrap().len(), 1);

        engine.delete_exchange(project_id, id).unwrap();

        assert!(engine.get_exchange(project_id, id).unwrap().is_none());
        assert_eq!(
            engine.search(project_id, "admin", 10).unwrap().len(),
            0,
            "FTS5 row removed by delete"
        );
    }

    /// `tag_upsert` + `tag_attach` + `list_tags` + `tag_detach`:
    /// the tag lifecycle at the engine layer. `tag_upsert` is
    /// idempotent within a project (same name → same id).
    #[test]
    fn engine_tag_lifecycle() {
        let tmp = TempDir::new().unwrap();
        let project = bk_core::Project::new("acme.bb", "acme.bb", "0.1.0");
        let project_id = project.info.id;

        let engine = Engine::new(tmp.path()).unwrap();
        engine.open_project(&project).unwrap();
        let ex = make_exchange(project_id, "/x");
        let exchange_id = ex.meta.id;
        engine.insert_exchange(project_id, &ex).unwrap();

        // Upsert twice — same name → same id (idempotent).
        let id1 = engine
            .tag_upsert(
                project_id,
                bk_store::tags::NewTag {
                    name: "vuln".into(),
                    color: Some("#ef4444".into()),
                },
            )
            .unwrap();
        let id2 = engine
            .tag_upsert(
                project_id,
                bk_store::tags::NewTag {
                    name: "vuln".into(),
                    color: Some("#ef4444".into()),
                },
            )
            .unwrap();
        assert_eq!(id1, id2, "tag upsert is idempotent within a project");

        // list_tags returns the tag with the color we set.
        let tags = engine.list_tags(project_id).unwrap();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].name, "vuln");
        assert_eq!(tags[0].color.as_deref(), Some("#ef4444"));

        // Attach + list_tags_for_exchange.
        engine.tag_attach(project_id, id1, exchange_id).unwrap();
        let on_ex = engine
            .list_tags_for_exchange(project_id, exchange_id)
            .unwrap();
        assert_eq!(on_ex.len(), 1);
        assert_eq!(on_ex[0].id, id1);

        // Detach.
        engine.tag_detach(project_id, id1, exchange_id).unwrap();
        let on_ex = engine
            .list_tags_for_exchange(project_id, exchange_id)
            .unwrap();
        assert!(on_ex.is_empty(), "tag detached");
    }

    /// The project-open invariant: every method on a closed project
    /// returns `ProjectNotOpen`. The existing `insert_into_non_open_project_errors`
    /// covers `insert_exchange`; this test covers the 6 new methods.
    #[test]
    fn engine_methods_error_on_non_open_project() {
        let tmp = TempDir::new().unwrap();
        let engine = Engine::new(tmp.path()).unwrap();
        let project_id = bk_core::ProjectId::new();
        let exchange_id = bk_core::ExchangeId::new();
        let tag_id = bk_core::TagId::new();

        // No `open_project` call — all of these should error.
        let results: Vec<(&str, crate::Result<()>)> = vec![
            (
                "delete_exchange",
                engine.delete_exchange(project_id, exchange_id),
            ),
            (
                "update_notes",
                engine.update_notes(project_id, exchange_id, "n"),
            ),
            (
                "set_starred",
                engine.set_starred(project_id, exchange_id, true),
            ),
            (
                "tag_attach",
                engine.tag_attach(project_id, tag_id, exchange_id),
            ),
            (
                "tag_detach",
                engine.tag_detach(project_id, tag_id, exchange_id),
            ),
        ];
        for (name, r) in &results {
            assert!(
                matches!(r, Err(EngineError::ProjectNotOpen(_))),
                "{name} on non-open project should be ProjectNotOpen, got {r:?}"
            );
        }
        // `tag_upsert` and `list_tags` return values, not ().
        assert!(matches!(
            engine.tag_upsert(
                project_id,
                bk_store::tags::NewTag {
                    name: "x".into(),
                    color: None
                }
            ),
            Err(EngineError::ProjectNotOpen(_))
        ));
        assert!(matches!(
            engine.list_tags(project_id),
            Err(EngineError::ProjectNotOpen(_))
        ));
        assert!(matches!(
            engine.list_tags_for_exchange(project_id, exchange_id),
            Err(EngineError::ProjectNotOpen(_))
        ));
    }
}
