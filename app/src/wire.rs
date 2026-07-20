//! §4.0 Wire event helpers (additive `wire_event` emit).
//!
//! This module is the *only* place in the Tauri app that knows
//! about the `bk_events::WireEvent` envelope. The contract is:
//!
//! 1. Every existing typed emit (`agent_event`,
//!    `agent_confirm_request`, `agent_confirm_response`,
//!    `proxy_event`, `engine_event`) stays unchanged. The
//!    Zustand agent store, the proxy listener, and the engine
//!    listener continue to subscribe to their typed channels.
//! 2. NEXT to each existing emit, we also emit a
//!    `wire_event` whose payload is the new
//!    `bk_events::WireEvent` envelope. The two emits happen
//!    back-to-back; if the `wire_event` emit fails, we log
//!    and continue (the typed emit is the load-bearing one
//!    for v1 consumers).
//! 3. The `seq` is a process-global `Arc<AtomicU64>` held in
//!    `tauri::State` as [`WireEventSeq`]. Each `wire_event`
//!    emit fetches-and-increments the counter, so the seq is
//!    monotonic across the lifetime of the process.
//!
//! ## Why this is additive
//!
//! §4.0 does NOT migrate any consumer to the `WireEvent`
//! envelope. The migration is §4.2's job (the engine bus →
//! `bk_events::fan_in` → Tauri `wire_event`) and §4.3-4.4's
//! job (the React `WireClient` → consumer switch). This
//! module just provides the helpers that the existing emit
//! sites call.
//!
//! ## Why a process-global seq
//!
//! Phase 8's drop detection on the React side tracks `lastSeq`
//! and surfaces a "missed events" banner when a gap is
//! observed. The seq MUST be process-global (not per-source)
//! so a gap from the agent bus is visible even if the engine
//! bus is still streaming events with a fresh seq.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use bk_events::{WireEvent, WireEventKind};
use tauri::{AppHandle, Emitter};

/// Tauri event label for the §4.0 wire envelope. Pinned by
/// the React `WireClient` in `ui/src/lib/ws.ts` — both sides
/// must agree on this string.
pub(crate) const WIRE_EVENT_LABEL: &str = "wire_event";

/// The process-global seq counter. Held in
/// `tauri::State<WireEventSeq>` so every Tauri command and
/// every forwarder task can fetch-and-increment the same
/// counter. The starting value is `0`; the first emitted
/// `wire_event` has `seq: 1`.
///
/// `Arc<AtomicU64>` is the same shape the `fan_in` helper in
/// `bk_events` expects, so when §4.2 wires the engine bus
/// into the fan-in, it can pass this counter directly.
#[derive(Default, Clone)]
pub struct WireEventSeq(pub Arc<AtomicU64>);

impl WireEventSeq {
    /// Construct a fresh seq counter. Used by
    /// `app::run` in `lib.rs` via `manage(WireEventSeq::default())`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Stamp a fresh seq and return it. The seq is
    /// 1-based (the first emit is `seq: 1`).
    fn next_seq(&self) -> u64 {
        // fetch_add returns the PREVIOUS value, so add 1
        // to get the 1-based seq. The first call returns
        // 0+1=1, the second returns 1+1=2, etc.
        self.0.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Peek at the current seq without incrementing. Used
    /// by tests to assert post-conditions.
    #[allow(dead_code)] // exposed for tests + future consumers (§4.2 uses it)
    pub fn current(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

/// Build a `WireEvent` with a fresh seq. Helper for the
/// additive emit sites — wraps the source payload in the
/// envelope, stamps the seq, and returns the JSON value
/// that the Tauri `emit_to` will serialize.
///
/// `kind` is which source bus the event came from (the
/// `WireEventKind` enum, NOT the typed event label — the
/// envelope's discriminator is `"agent_event"`,
/// `"engine_event"`, or `"proxy_event"`).
///
/// `payload` is the source event already serialized to
/// `serde_json::Value` (e.g. the `AgentEvent` the
/// `run_forwarder` is about to emit to `agent_event`).
pub(crate) fn make_wire_event(
    seq_state: &WireEventSeq,
    kind: WireEventKind,
    payload: serde_json::Value,
) -> WireEvent {
    let seq = seq_state.next_seq();
    WireEvent::new(kind, payload, seq)
}

/// Additive `wire_event` emit. Calls `app.emit_to(WEBVIEW_LABEL, WIRE_EVENT_LABEL, ...)`
/// with the `WireEvent` payload. Errors are logged but not
/// propagated — the typed emit (which already happened) is
/// the load-bearing one for v1 consumers.
pub(crate) fn emit_wire_event(app: &AppHandle, webview_label: &str, wire: &WireEvent) {
    // The derived `Serialize` on `WireEvent` produces the
    // on-wire shape `{kind, payload, seq}` directly (the
    // `#[serde(default)]` on `seq` only affects
    // deserialization, not serialization). Tauri 2's
    // `emit_to` accepts a `Serialize` and round-trips it
    // through the IPC bridge.
    if let Err(e) = app.emit_to(webview_label, WIRE_EVENT_LABEL, wire) {
        tracing::error!(
            kind = %wire.kind,
            seq = wire.seq,
            error = %e,
            "emit wire_event failed"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The seq counter is monotonic. First call returns 1,
    /// second returns 2, etc.
    #[test]
    fn seq_counter_starts_at_one() {
        let s = WireEventSeq::new();
        assert_eq!(s.next_seq(), 1);
        assert_eq!(s.next_seq(), 2);
        assert_eq!(s.next_seq(), 3);
        assert_eq!(s.current(), 3);
    }

    /// `make_wire_event` stamps the seq and produces a
    /// `WireEvent` with the right kind and payload.
    #[test]
    fn make_wire_event_stamps_seq() {
        let s = WireEventSeq::new();
        let payload = serde_json::json!({"event": "agent_started", "agent_id": "r1"});
        let ev = make_wire_event(&s, WireEventKind::AgentEvent, payload.clone());
        assert_eq!(ev.kind, WireEventKind::AgentEvent);
        assert_eq!(ev.payload, payload);
        assert_eq!(ev.seq, 1);

        // The counter advanced.
        assert_eq!(s.current(), 1);
    }

    /// `make_wire_event` calls are independent across
    /// clones of the seq counter (the inner `Arc` is
    /// shared).
    #[test]
    fn seq_counter_is_shared_across_clones() {
        let s1 = WireEventSeq::new();
        let s2 = s1.clone();
        assert_eq!(s1.next_seq(), 1);
        assert_eq!(s2.next_seq(), 2);
        assert_eq!(s1.next_seq(), 3);
    }
}
