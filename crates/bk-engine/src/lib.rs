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
}
