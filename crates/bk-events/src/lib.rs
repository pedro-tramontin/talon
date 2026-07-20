//! Phase 8 wire-format event envelope.
//!
//! `bk-events` defines a single tagged envelope — [`WireEvent`] —
//! that carries the three kinds of events the Talon app/UI needs to
//! surface in a uniform shape on the JSON wire:
//!
//! * `engine_event` — full [`EngineEvent`](bk_core) state changes
//!   (project open/close, exchange insert/update, tag upsert, etc.)
//! * `agent_event`  — the streaming `AgentEvent` from the
//!   LLM-driven agent loop
//! * `proxy_event`  — proxy lifecycle + per-request events from the
//!   MITM listener
//!
//! The envelope is intentionally **type-erased at the payload**:
//! the `payload` field is a [`serde_json::Value`]. The Tauri shell
//! already serializes each source event with the correct `serde`
//! shape; the `WireEvent` wrapper just gives the wire a single
//! `{kind, payload, seq}` shape that the React side can pattern-
//! match on. New event variants in `bk-core` / `bk-agent` /
//! `bk-proxy` do NOT require a `bk-events` change — they flow
//! through as the inner `payload` JSON unchanged.
//!
//! ## On-wire shape
//!
//! ```json
//! {"kind": "engine_event", "payload": {...}, "seq": 42}
//! ```
//!
//! The discriminator is the `"kind"` string. The `seq` field is
//! **load-bearing for Phase 8 drop detection** — the React side
//! tracks `lastSeq` and surfaces a "missed events" banner if a gap
//! is observed. The seq is stamped on the **Rust side** at the
//! `WireEvent` construction site (or at the `fan_in` boundary),
//! not in the UI.
//!
//! ## What this crate is NOT
//!
//! * It is NOT a replacement for the existing typed Tauri emits
//!   (`agent_event`, `proxy_event`, `engine_event`). Those stay
//!   for the existing Zustand store / proxy listener / engine
//!   listener consumers. The `wire_event` emit is **additive**.
//! * It does NOT migrate any consumer. The `bk-engine` →
//!   `bk-events` → Tauri wiring is §4.2's job. This crate just
//!   ships the envelope + the `fan_in` helper that §4.2 will
//!   plug into the engine's `EventSender`.
//!
//! ## Module layout
//!
//! * [`WireEvent`] / [`WireEventKind`] — the envelope. Lives in
//!   this file so a `use bk_events::WireEvent;` is the only
//!   import consumers need.
//! * [`fan_in`] — the `tokio::select!`-driven helper that
//!   multiplexes N source `broadcast::Receiver`s into a single
//!   `WireEvent`-typed `broadcast::Sender` with a monotonic
//!   `seq`. Used by §4.2's engine wiring; exercised by tests.

#![deny(missing_docs)]
#![deny(unused_must_use)]

use serde::{Deserialize, Serialize};

/// The three top-level sources of events Talon exposes to the UI.
///
/// The variant names are PascalCase Rust; the on-wire string forms
/// are the **snake_case** tag values listed in the variant docs
/// (set via `#[serde(rename_all = "snake_case")]`). The v1 set is
/// pinned: the design contract is "3 kinds, 3 string tags". Future
/// phases can add new kinds (e.g. `mcp_event`, `fuzz_event`) by
/// appending a variant here AND updating
/// `ui/src/lib/ws.ts` to handle the new tag — the wire shape is
/// intentionally additive.
///
/// `#[non_exhaustive]` is on the enum so a downstream match that
/// forgets the new variant fails to compile at the consumer site
/// (the `WireClient` in `ui/src/lib/ws.ts` has a switch on `kind`
/// that will need a new arm).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum WireEventKind {
    /// An `EngineEvent` (project open/close, exchange insert,
    /// tag upsert, ...). On-wire tag: `"engine_event"`.
    EngineEvent,
    /// A streaming `AgentEvent` (agent started/thinking/tool
    /// call/finished/error). On-wire tag: `"agent_event"`.
    AgentEvent,
    /// A `ProxyEvent` (proxy started/stopped, request
    /// forwarded). On-wire tag: `"proxy_event"`.
    ProxyEvent,
}

impl WireEventKind {
    /// Stable string form of this kind, exactly as it appears on
    /// the wire. Useful for logs / metrics labels / switch arms in
    /// the React `WireClient` (which mirrors this constant).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::EngineEvent => "engine_event",
            Self::AgentEvent => "agent_event",
            Self::ProxyEvent => "proxy_event",
        }
    }
}

impl std::fmt::Display for WireEventKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The on-wire event envelope. Every event the Tauri shell sends
/// to the React UI's `wire_event` channel has this shape:
///
/// ```json
/// {"kind": "engine_event", "payload": {...}, "seq": 42}
/// ```
///
/// The `payload` is **type-erased**: it is whatever JSON the source
/// bus already produced. The `bk-engine` `EngineEvent` serializes
/// via its own `serde` derive; the `bk-agent` `AgentEvent` does
/// the same; the `bk-proxy` `ProxyEvent` is also `serde`-derived.
/// `bk-events` does not re-define any of those — it just wraps
/// them. This is deliberate: it means a new variant in
/// `bk-engine`'s `EngineEvent` flows through `WireEvent` with
/// **zero changes here**.
///
/// The `seq` field is the load-bearing piece for Phase 8 drop
/// detection: a monotonic counter stamped on the Rust side, the
/// React `WireClient` tracks `lastSeq` and surfaces a "missed
/// events" banner if a gap is observed. It is `#[serde(skip
///_serializing)]` because (a) the stamped value lives only on
/// the wire, and (b) the `seq` is stamped at the boundary
/// (`fan_in` or per-emit), not stored as a member of the source
/// event.
///
/// `#[non_exhaustive]` is on the struct so a future v2 can add
/// fields (e.g. `trace_id`, `timestamp`) without breaking v1
/// deserializers — the React side just ignores unknown fields
/// (the v1 type already does because `serde` defaults to ignore).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub struct WireEvent {
    /// The discriminator: which source bus the payload came
    /// from. Adjacently tagged via `#[serde(tag = "kind")]` on
    /// the struct.
    pub kind: WireEventKind,
    /// The event body, as JSON. Type-erased for forward-compat:
    /// the source bus's own `serde` derive produced the value.
    pub payload: serde_json::Value,
    /// Monotonic sequence number. Stamped on the Rust side at
    /// the `fan_in` boundary (or at the per-emit boundary in
    /// §4.0's additive mode). Skipped on serialize because
    /// callers may construct a `WireEvent` and then ask
    /// `serde_json::to_value` to re-stamp the seq — see the
    /// `from_event` helper below. The deserialize side
    /// accepts `seq` so the UI can read a stamped value.
    ///
    /// Default: `0` on deserialize if the field is absent. This
    /// keeps v1 readers compatible with v0 (pre-stamp) events
    /// that were sent without a seq — those events are treated
    /// as "seq 0" and the UI's drop-detection skips the gap
    /// check until it sees a non-zero seq.
    #[serde(default, skip_deserializing)]
    pub seq: u64,
}

impl WireEvent {
    /// Build a `WireEvent` from a kind + a payload value, with a
    /// fresh seq from the provided counter. Convenience wrapper
    /// for the §4.0 additive-emit sites (`agent.rs`).
    pub fn new(kind: WireEventKind, payload: serde_json::Value, seq: u64) -> Self {
        Self { kind, payload, seq }
    }

    /// Stamp a `WireEvent` for the wire. Returns a `serde_json::Value`
    /// that includes the `seq` field (which is skipped on
    /// `Serialize` derive because callers may want to inspect
    /// the seq separately; this helper produces the FINAL wire
    /// shape).
    pub fn to_wire_value(&self) -> serde_json::Value {
        serde_json::json!({
            "kind": self.kind,
            "payload": self.payload,
            "seq": self.seq,
        })
    }
}

pub mod fan_in;

#[cfg(test)]
mod tests {
    use super::*;

    /// The on-wire shape matches the design contract: the
    /// discriminator is the `"kind"` key (adjacently tagged) and
    /// the value uses the snake_case string. The `payload` is
    /// passed through verbatim, and the `seq` is preserved.
    #[test]
    fn wire_event_serializes_to_expected_shape() {
        let ev = WireEvent {
            kind: WireEventKind::EngineEvent,
            payload: serde_json::json!({"project_id": "abc", "summary": "GET /admin"}),
            seq: 7,
        };
        let v = serde_json::to_value(&ev).expect("serialize");
        assert_eq!(v["kind"], "engine_event");
        assert_eq!(v["payload"]["project_id"], "abc");
        assert_eq!(v["payload"]["summary"], "GET /admin");
        assert_eq!(v["seq"], 7);
    }

    /// `WireEventKind::as_str` returns the on-wire string form.
    /// Pinned by the design contract (the React `WireClient`
    /// switch on `kind` mirrors these exact strings).
    #[test]
    fn wire_event_kind_as_str_matches_design_contract() {
        assert_eq!(WireEventKind::EngineEvent.as_str(), "engine_event");
        assert_eq!(WireEventKind::AgentEvent.as_str(), "agent_event");
        assert_eq!(WireEventKind::ProxyEvent.as_str(), "proxy_event");
    }

    /// `to_wire_value` produces a shape the React side can
    /// `JSON.parse` directly. It is the helper the app uses to
    /// emit through `app.emit_to("main", "wire_event", value)`.
    #[test]
    fn to_wire_value_includes_seq() {
        let ev = WireEvent::new(
            WireEventKind::AgentEvent,
            serde_json::json!({"event": "agent_started", "agent_id": "r1"}),
            99,
        );
        let v = ev.to_wire_value();
        assert_eq!(v["kind"], "agent_event");
        assert_eq!(v["payload"]["event"], "agent_started");
        assert_eq!(v["seq"], 99);
    }

    /// A `WireEvent` constructed with `seq: 0` and serialized via
    /// `to_wire_value` still has `seq: 0` in the output. The
    /// `#[serde(skip_deserializing)]` is ONLY on deserialize; on
    /// serialize the value is preserved.
    #[test]
    fn to_wire_value_preserves_zero_seq() {
        let ev = WireEvent::new(
            WireEventKind::ProxyEvent,
            serde_json::json!({"proxy": "started"}),
            0,
        );
        let v = ev.to_wire_value();
        assert_eq!(v["seq"], 0);
    }
}
