//! Engine + Tauri command integration tests for Phase 6 Part C
//! (§C-A.1 — `Engine::save_settings` + the CRUD commands
//! persisting to SQLite, §C-A.2 — `start_proxy` wiring).
//!
//! These tests cover the full Rust-side surface of the v0.5
//! cleanup batch that lives below the `bk-store` layer. The
//! `bk-store` layer's `update_settings` / `read_settings` are
//! unit-tested in `crates/bk-store/src/projects.rs`; the
//! `replay_history` table is unit-tested in
//! `crates/bk-store/src/replay_history.rs`. This file covers
//! the engine wiring + the Tauri command surface.
//!
//! **Why a separate test file:** the `app` crate's Tauri
//! commands are hard to test in isolation (the `tauri::State`
//! wrapper is built by the macro). We test the
//! `Engine::save_settings` / `Engine::list_replay_history` /
//! `Engine::append_replay_history` methods directly via a
//! fresh `Engine` + a `Project`. The Tauri command wrappers
//! are thin pass-throughs (just call the engine method + map
//! errors to `String`); they're not separately tested.

use bk_core::{Project, ProjectId, ProjectSettings, Theme};
use bk_engine::Engine;
use std::sync::Arc;
use tempfile::TempDir;

/// Build a fresh engine with one open project. Returns
/// `(engine, project_id, _tmp_dir)`. Mirrors the helper in
/// `app/src/commands/scope.rs::tests` but lives here so the
/// file is self-contained.
fn fresh_engine() -> (Arc<Engine>, ProjectId, TempDir) {
    let tmp = TempDir::new().unwrap();
    let engine = Arc::new(Engine::new(tmp.path()).unwrap());
    let project = Project::new("acme.bb", "acme.bb", "0.1.0");
    let id = project.info.id;
    engine.open_project(&project).unwrap();
    (engine, id, tmp)
}

// ---------------------------------------------------------------------------
// §C-A.1 — Engine::save_settings
// ---------------------------------------------------------------------------

/// `Engine::save_settings` round-trips: update a rule,
/// restart the engine, the rule is there.
#[test]
fn save_settings_round_trips_across_restart() {
    let tmp = TempDir::new().unwrap();
    let project = Project::new("acme.bb", "acme.bb", "0.1.0");
    let id = project.info.id;

    // First "session": add a rule and persist.
    {
        let engine = Arc::new(Engine::new(tmp.path()).unwrap());
        engine.open_project(&project).unwrap();
        let mut project = engine.get_project(id).unwrap();
        project.settings.theme = Theme::Dark;
        project.settings.proxy_enabled = false;
        engine.save_settings(id, &project.settings).unwrap();
    }

    // Second "session": reopen, the settings are there.
    {
        let engine = Arc::new(Engine::new(tmp.path()).unwrap());
        engine.open_project(&project).unwrap();
        let project = engine.get_project(id).unwrap();
        assert_eq!(project.settings.theme, Theme::Dark);
        assert!(!project.settings.proxy_enabled);
    }
}

/// `Engine::save_settings` errors on a project that isn't
/// open (the `ProjectNotOpen` engine error).
#[test]
fn save_settings_errors_on_project_not_open() {
    let tmp = TempDir::new().unwrap();
    let engine = Arc::new(Engine::new(tmp.path()).unwrap());
    let unknown_id = ProjectId::new();
    let res = engine.save_settings(unknown_id, &ProjectSettings::default());
    assert!(res.is_err());
    // `EngineError::ProjectNotOpen(String)` formats as
    // "project not open: <id>" via `thiserror`.
    let err = res.unwrap_err();
    assert!(
        matches!(err, bk_engine::EngineError::ProjectNotOpen(_)),
        "got: {err:?}"
    );
}

/// `Engine::save_settings` is idempotent: calling it twice
/// with the same settings succeeds and the second call is a
/// no-op on the wire (the JSON is identical).
#[test]
fn save_settings_is_idempotent() {
    let (engine, id, _tmp) = fresh_engine();
    let settings = ProjectSettings {
        theme: Theme::Light,
        ..Default::default()
    };
    engine.save_settings(id, &settings).unwrap();
    engine.save_settings(id, &settings).unwrap();
    let read = engine.get_project(id).unwrap();
    assert_eq!(read.settings.theme, Theme::Light);
}

// ---------------------------------------------------------------------------
// §C-A.2 — start_proxy wiring (the "lookup the active project's
// rules and pass them to ProxyHandle" path)
// ---------------------------------------------------------------------------

/// The active project's scope rules survive a
/// `Engine::save_settings` call and are readable via
/// `Engine::get_project` in the same session (the in-memory
/// path the `start_proxy` Tauri command uses).
#[test]
fn start_proxy_wiring_active_project_has_rules() {
    let (engine, id, _tmp) = fresh_engine();
    let mut project = engine.get_project(id).unwrap();
    project.settings.scope_rules.push(bk_core::ScopeRule {
        kind: bk_core::ScopeRuleKind::Host,
        pattern: "acme.bb".to_string(),
        action: bk_core::MatchAction::InScope,
        label: "primary".to_string(),
        priority: 0,
    });
    engine.save_settings(id, &project.settings).unwrap();
    let project = engine.get_project(id).unwrap();
    assert_eq!(project.settings.scope_rules.len(), 1);
    assert_eq!(project.settings.scope_rules[0].label, "primary");
}

/// The "no active project" fallback: when no project is
/// open, `engine.open_ids()` returns empty and the `start_proxy`
/// Tauri command falls back to empty `Vec`s. We assert the
/// engine returns the empty list directly.
#[test]
fn start_proxy_wiring_falls_back_when_no_active_project() {
    let tmp = TempDir::new().unwrap();
    let engine = Arc::new(Engine::new(tmp.path()).unwrap());
    // No project opened.
    let open = engine.open_ids();
    assert!(open.is_empty(), "fresh engine has no open projects");
}

// ---------------------------------------------------------------------------
// §C-A.4 — replay history commands
// ---------------------------------------------------------------------------

/// `Engine::append_replay_history` + `Engine::list_replay_history`
/// round-trip. Insert an entry, list returns it. Insert in
/// reverse-sequence order, list orders by `sequence_within_tab`
/// ASC.
#[test]
fn append_and_list_replay_history_round_trip() {
    let (engine, project_id, _tmp) = fresh_engine();
    let tab_id = "tab-A";
    let req_id = bk_core::ExchangeId::new();
    // We don't need a real exchange row to insert a history
    // entry — the FK is enforced, but the FK ON DELETE CASCADE
    // means we can insert with a known-bad exchange id and
    // expect the FK to fail. So insert a real exchange first.
    let exchange = bk_core::HttpExchange {
        meta: bk_core::ExchangeMeta {
            id: req_id,
            project_id,
            timestamp: chrono::Utc::now(),
            duration_ns: 0,
            summary: "GET /".to_string(),
            scope_state: bk_core::ScopeState::Unscoped,
            notes: String::new(),
            starred: false,
            // v0.6 P2 #6: defaults for the new fields.
            method: "GET".to_string(),
            status: 200,
            tags: Vec::new(),
        },
        request: bk_core::Request {
            method: bk_core::Method::GET,
            url: "https://acme.bb/".parse().unwrap(),
            version: bk_core::Version::HTTP_11,
            headers: bk_core::HeaderMap::new(),
            body: bk_core::Body::empty(),
        },
        response: None,
        blocked_reason: None,
    };
    engine
        .insert_exchange(project_id, &exchange)
        .expect("insert_exchange should succeed");

    // Insert in reverse order.
    engine
        .append_replay_history(
            project_id,
            &bk_engine::ReplayHistoryEntry {
                id: "entry-2".to_string(),
                project_id,
                tab_id: tab_id.to_string(),
                request_exchange_id: req_id,
                response_exchange_id: None,
                timestamp: chrono::Utc::now(),
                sequence_within_tab: 2,
            },
        )
        .unwrap();
    engine
        .append_replay_history(
            project_id,
            &bk_engine::ReplayHistoryEntry {
                id: "entry-0".to_string(),
                project_id,
                tab_id: tab_id.to_string(),
                request_exchange_id: req_id,
                response_exchange_id: None,
                timestamp: chrono::Utc::now(),
                sequence_within_tab: 0,
            },
        )
        .unwrap();
    engine
        .append_replay_history(
            project_id,
            &bk_engine::ReplayHistoryEntry {
                id: "entry-1".to_string(),
                project_id,
                tab_id: tab_id.to_string(),
                request_exchange_id: req_id,
                response_exchange_id: None,
                timestamp: chrono::Utc::now(),
                sequence_within_tab: 1,
            },
        )
        .unwrap();
    let listed = engine.list_replay_history(project_id, tab_id).unwrap();
    assert_eq!(listed.len(), 3);
    let seqs: Vec<i64> = listed.iter().map(|e| e.sequence_within_tab).collect();
    assert_eq!(
        seqs,
        vec![0, 1, 2],
        "must be ordered ASC by sequence_within_tab"
    );
}

/// `Engine::list_replay_history` returns empty for a tab
/// that has no entries (the "fresh tab" case on `openTab`).
#[test]
fn list_replay_history_empty_for_unknown_tab() {
    let (engine, project_id, _tmp) = fresh_engine();
    let listed = engine
        .list_replay_history(project_id, "tab-does-not-exist")
        .unwrap();
    assert!(listed.is_empty());
}
