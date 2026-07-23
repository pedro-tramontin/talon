//! WebSocket proxy tests (2 cases).
//!
//! Per the v0.3.42 mode-B pre-trim rule, this file has
//! exactly 2 test cases. Do NOT exceed.

use std::time::Duration;

use bk_events::WireEvent;
use bk_server::WsHub;
use serde_json::json;
use tokio::time::timeout;

/// Build a no-op `WireEvent` for tests via JSON
/// deserialization (the struct is `#[non_exhaustive]`
/// so a struct expression won't work cross-crate).
fn test_event() -> WireEvent {
    serde_json::from_value(json!({
        "kind": "engine_event",
        "payload": {"test": true},
        "seq": 0u64
    }))
    .expect("test event JSON must deserialize")
}

#[tokio::test]
async fn ws_hub_broadcasts_to_single_subscriber() {
    // Single subscriber: broadcast a WireEvent, assert
    // the subscriber receives it within 100ms.
    let hub = WsHub::new();
    let mut rx = hub.subscribe();
    hub.broadcast(test_event());
    let event = timeout(Duration::from_millis(100), rx.recv())
        .await
        .expect("subscriber must receive within 100ms")
        .expect("subscriber must receive Ok");
    assert_eq!(event.payload, serde_json::json!({"test": true}));
}

#[tokio::test]
async fn ws_hub_broadcasts_to_multiple_subscribers() {
    // Two subscribers: broadcast, both receive (the
    // broadcast semantics, not unicast).
    let hub = WsHub::new();
    let mut rx1 = hub.subscribe();
    let mut rx2 = hub.subscribe();
    hub.broadcast(test_event());
    let e1 = timeout(Duration::from_millis(100), rx1.recv())
        .await
        .expect("rx1 must receive within 100ms")
        .expect("rx1 Ok");
    let e2 = timeout(Duration::from_millis(100), rx2.recv())
        .await
        .expect("rx2 must receive within 100ms")
        .expect("rx2 Ok");
    // Both should receive the same payload.
    assert_eq!(e1.payload, e2.payload);
}
