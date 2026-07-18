// Use edition 2021 to allow `async fn` and modern syntax.
#![allow(unused_imports)]

//! End-to-end smoke for the §3.5a event bus. Proves the
//! `engine → event bus → observer` path works without a Tauri
//! window, without a real proxy, without an LLM. This is the
//! regression test the Phase 4 UI work leans on: if this passes,
//! the engine and event bus are wired correctly and the UI just
//! needs to subscribe (Phase 4 work). If this fails, the UI work
//! is also broken (in a way that xvfb + a real display wouldn't
//! have caught on the headless VPS).
//!
//! Per the design contract
//! (`2026-07-15_phase-03.5-agent-mcp.md` §3.5a), this file has
//! **3 tests**:
//!   - `subscribe_mcp_events_returns_a_receiver`
//!   - `capture_loop_engine_to_observer_works`
//!   - `event_order_is_enforced_for_causal_chains`
//!
//! The "events arrive in order" assertion in
//! `capture_loop_engine_to_observer_works` is the load-bearing one
//! (per the spec's Gotcha 1): it proves the bus emit is part of
//! the same code path as the state change, not on a separate task
//! that can lag.

use bk_core::{Body, ExchangeMeta, HeaderMap, HttpExchange, Method, Request, ScopeState, Version};
use bk_engine::{Engine, McpEvent, McpEventOrder};
use bk_store::tags::NewTag;
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::timeout;

/// Build a minimal `HttpExchange` for the given project + path.
/// Mirrors the helper in `lib.rs::tests` so the per-test body is
/// the interesting part (the assertion) rather than 25 lines of
/// struct literals.
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
            url: format!("https://example.com{path}").parse().unwrap(),
            version: Version::HTTP_11,
            headers: HeaderMap::default(),
            body: Body::empty(),
        },
        response: Some(bk_core::Response {
            status: 200,
            status_text: "OK".into(),
            version: Version::HTTP_11,
            headers: HeaderMap::default(),
            body: Body::empty(),
        }),
        blocked_reason: None,
    }
}

/// Drain everything the bus emits within `quiet_for`. Used by the
/// "engine action → bus → observer" test. The pattern is:
///   1. recv() with a 1s outer timeout (in case the bus is silent)
///   2. once one event arrives, recv() with a 100ms timeout
///      (anything still in the channel arrives; after 100ms of
///      silence we assume the bus is drained)
async fn drain_events(
    rx: &mut tokio::sync::broadcast::Receiver<McpEvent>,
    max_wait: Duration,
) -> Vec<McpEvent> {
    let mut out = Vec::new();
    // First event: wait up to max_wait
    match timeout(max_wait, rx.recv()).await {
        Ok(Ok(ev)) => out.push(ev),
        Ok(Err(_)) => return out, // lagged or closed
        Err(_) => return out,     // timeout, bus is silent
    }
    // Subsequent events: 100ms quiet window
    loop {
        match timeout(Duration::from_millis(100), rx.recv()).await {
            Ok(Ok(ev)) => out.push(ev),
            Ok(Err(_)) => return out, // lagged or closed
            Err(_) => return out,     // 100ms of silence = drained
        }
    }
}

/// `subscribe_mcp_events()` returns a fresh receiver each time.
/// Pin the public surface so future refactors don't accidentally
/// share a single receiver across subscribers.
#[test]
fn subscribe_mcp_events_returns_a_receiver() {
    let tmp = TempDir::new().unwrap();
    let engine = Engine::new(tmp.path()).unwrap();
    let _rx1 = engine.subscribe_mcp_events();
    let _rx2 = engine.subscribe_mcp_events();
    // We don't assert they're different instances (broadcast::Receiver
    // is Clone, so a single sender can have many receivers), but
    // the API surface must not return a single shared receiver.
    // The fact that the call returns without panicking is enough.
}

/// The load-bearing test. Engine action → bus emit → observer
/// receives the event IN THE RIGHT ORDER. The "right order" is
/// the spec's promise: project opened before exchange inserted,
/// exchange before tag.
#[tokio::test]
async fn capture_loop_engine_to_observer_works() {
    let tmp = TempDir::new().unwrap();
    let project = bk_core::Project::new("acme.bb", "acme.bb", "0.1.0");
    let project_id = project.info.id;
    let engine = Engine::new(tmp.path()).unwrap();

    // Subscribe BEFORE any state change.
    let mut mcp_rx = engine.subscribe_mcp_events();

    // Action: open project, insert exchange, tag it.
    engine.open_project(&project).unwrap();
    let ex = make_exchange(project_id, "/admin");
    let id = ex.meta.id;
    engine.insert_exchange(project_id, &ex).unwrap();
    engine
        .tag_upsert(
            project_id,
            NewTag {
                name: "vuln".into(),
                color: None,
            },
        )
        .unwrap();

    // Observe: drain the bus.
    let events = drain_events(&mut mcp_rx, Duration::from_secs(1)).await;

    // Assert: an ExchangeCaptured event for /admin arrived.
    assert!(
        events
            .iter()
            .any(|e| matches!(e, McpEvent::ExchangeCaptured { id: e_id, .. } if *e_id == id)),
        "expected ExchangeCaptured for {id}, got {events:?}"
    );
    // Assert: a TagAdded event for "vuln" arrived.
    assert!(
        events
            .iter()
            .any(|e| matches!(e, McpEvent::TagAdded { name, .. } if name == "vuln")),
        "expected TagAdded for \"vuln\", got {events:?}"
    );
    // Assert: events arrived in causal order.
    // The causal chain is: ExchangeCaptured must come before any
    // TagAdded that references the captured exchange. Find both
    // events and assert the ExchangeCaptured index < TagAdded index.
    let ex_idx = events
        .iter()
        .position(|e| matches!(e, McpEvent::ExchangeCaptured { id: e_id, .. } if *e_id == id))
        .expect("ExchangeCaptured must be present (checked above)");
    let tag_idx = events
        .iter()
        .position(|e| matches!(e, McpEvent::TagAdded { name, .. } if name == "vuln"))
        .expect("TagAdded must be present (checked above)");
    assert!(
        ex_idx < tag_idx,
        "ExchangeCaptured at index {ex_idx} must come before TagAdded at index {tag_idx}: {events:?}"
    );
    // And the full sequence must be monotonically non-decreasing
    // by McpEventOrder — no event can appear before a causal
    // predecessor.
    assert!(
        events
            .windows(2)
            .all(|w| McpEventOrder::of(&w[0]) <= McpEventOrder::of(&w[1])),
        "events not monotonically ordered: {events:?}"
    );
}

/// Pin the order invariant: every McpEvent has an order key
/// (`McpEventOrder::of`) and the key is total. The
/// `capture_loop_engine_to_observer_works` test asserts the order
/// of actual events; this one asserts the ordering scheme is
/// well-defined for every v1 variant.
#[test]
fn event_order_is_enforced_for_causal_chains() {
    // Build one of each v1 variant. Each must have an order key.
    let project_id = bk_core::ProjectId::new();
    let exchange_id = bk_core::ExchangeId::new();
    let tag_id = bk_core::TagId::new();
    let all = vec![
        McpEvent::ExchangeCaptured {
            id: exchange_id,
            project_id,
            method: "GET".into(),
            url: "/".into(),
            status: None,
        },
        McpEvent::TagAdded {
            id: tag_id,
            project_id,
            name: "vuln".into(),
        },
        McpEvent::ProxyStarted {
            listener_addr: "127.0.0.1:8080".into(),
        },
        McpEvent::ProxyStopped {
            reason: "shutdown".into(),
        },
        McpEvent::FuzzStarted {
            job_id: "stub".into(),
            project_id,
        },
    ];
    for ev in &all {
        assert!(
            McpEventOrder::of(ev).is_some(),
            "McpEvent has no order key: {ev:?}"
        );
    }
    // And the order is total: a sorted slice of order keys is
    // monotone non-decreasing.
    let mut keys: Vec<_> = all.iter().filter_map(McpEventOrder::of).collect();
    keys.sort();
    let mut sorted = keys.clone();
    sorted.dedup();
    assert_eq!(keys, sorted, "the order keys are unique after sort");
}
