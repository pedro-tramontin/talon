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
        let exchange = HttpExchange {
            meta: ExchangeMeta {
                id: bk_core::ExchangeId::new(),
                project_id,
                timestamp: chrono::Utc::now(),
                duration_ns: 0,
                summary: "GET /admin".to_string(),
                scope_state: ScopeState::InScope,
                notes: String::new(),
                starred: false,
            },
            request: Request {
                method: Method::GET,
                url: "https://acme.bb/admin".parse().unwrap(),
                version: bk_core::Version::HTTP_11,
                headers: HeaderMap::new(),
                body: Body::empty(),
            },
            response: None,
            blocked_reason: None,
        };
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
        let exchange = HttpExchange {
            meta: ExchangeMeta {
                id: bk_core::ExchangeId::new(),
                project_id,
                timestamp: chrono::Utc::now(),
                duration_ns: 0,
                summary: "GET /x".to_string(),
                scope_state: ScopeState::InScope,
                notes: String::new(),
                starred: false,
            },
            request: Request {
                method: Method::GET,
                url: "https://acme.bb/x".parse().unwrap(),
                version: bk_core::Version::HTTP_11,
                headers: HeaderMap::new(),
                body: Body::empty(),
            },
            response: None,
            blocked_reason: None,
        };
        let res = engine.insert_exchange(project_id, &exchange);
        assert!(matches!(res, Err(EngineError::ProjectNotOpen(_))));
    }
}
