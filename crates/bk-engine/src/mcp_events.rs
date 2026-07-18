//! MCP-narrowed event bus. The MCP server (§3.5b) and the internal
//! agent (§3.5c) subscribe to **this** bus, not the full bus in
//! `events.rs`. The MCP events are a strict subset / re-shaping of
//! the full events — they have smaller payloads (e.g.,
//! `ExchangeCaptured` doesn't carry the full request/response, just
//! the summary) so the LLM prompt stays small.
//!
//! The demux from `EngineEvent` → `McpEvent` happens inside the
//! `Engine` (in `engine.rs`), not in the bus. The bus is
//! dumb-pipe: it carries the MCP-shaped events the engine
//! publishes. This is the same pattern as the UI event bus in
//! `tauri::Manager::emit_to` — the demux is at the source.
//!
//! Per the design contract
//! (`2026-07-15_phase-03.5-agent-mcp.md` §3.5a), the v1 MCP bus
//! carries **5 event types**:
//!   - `ProxyStarted` (Phase 3 wire-in; stub now)
//!   - `ProxyStopped` (Phase 3 wire-in; stub now)
//!   - `ExchangeCaptured` (demuxed from `ExchangeInserted`)
//!   - `TagAdded` (demuxed from `TagUpserted`)
//!   - `FuzzStarted` (Phase 7 stub)

#![allow(missing_docs)]

use bk_core::{ExchangeId, ProjectId, TagId};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// The MCP-visible subset of the engine's events. Each variant
/// has a smaller payload than the corresponding `EngineEvent` so
/// the LLM prompt stays manageable. The mapping is:
///
/// | `McpEvent`             | From `EngineEvent`                  |
/// |------------------------|-------------------------------------|
/// | `ExchangeCaptured`     | `ExchangeInserted`                  |
/// | `TagAdded`             | `TagUpserted`                       |
/// | `ProxyStarted`         | (Phase 3 wire-in, stub now)         |
/// | `ProxyStopped`         | (Phase 3 wire-in, stub now)         |
/// | `FuzzStarted`          | (Phase 7 stub)                      |
///
/// `#[non_exhaustive]` so future phases can add new MCP-visible
/// events (e.g., a `ScanCompleted` when Phase 6's scope engine
/// gets too clever) without breaking the v1 shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum McpEvent {
    /// Demuxed from `EngineEvent::ExchangeInserted`. The LLM uses
    /// this to react to new traffic. Note the smaller payload:
    /// the `summary` is already pre-formatted, the request/response
    /// bodies are not included (the LLM can call `talon_get_exchange`
    /// to fetch them if it needs the full content).
    ExchangeCaptured {
        id: ExchangeId,
        project_id: ProjectId,
        method: String,
        url: String,
        status: Option<u16>,
    },
    /// Demuxed from `EngineEvent::TagUpserted`. Fires for both
    /// "created" and "already-existed" (idempotent upsert), same
    /// as the source event.
    TagAdded {
        id: TagId,
        project_id: ProjectId,
        name: String,
    },
    /// Phase 3 wire-in: when the MITM proxy starts, the engine
    /// publishes this so the LLM knows the proxy is up. The actual
    /// emit happens in the §3.6/§3.7 work (per-direction events).
    /// In §3.5a, this variant exists for forward-compat but the
    /// engine never sends it yet.
    ProxyStarted { listener_addr: String },
    /// Phase 3 wire-in. The proxy is stopping (graceful shutdown,
    /// error, or signal). The LLM should stop scheduling work
    /// that requires the proxy.
    ProxyStopped { reason: String },
    /// Phase 7 stub. A fuzz job started. The LLM can subscribe to
    /// a (future) `FuzzProgress` event to follow along.
    FuzzStarted {
        job_id: String,
        project_id: ProjectId,
    },
}

/// The MCP-narrowed event bus subscriber. Same semantics as
/// `events::EventReceiver` — the capacity is also 256; the
/// subscribers (MCP server, agent) are expected to keep up.
pub type McpEventReceiver = broadcast::Receiver<McpEvent>;

/// The MCP-narrowed event bus sender. Owned by the `Engine`.
pub type McpEventSender = broadcast::Sender<McpEvent>;

/// Build a fresh (sender, receiver) pair. Same capacity as the
/// full bus.
pub fn channel() -> (McpEventSender, McpEventReceiver) {
    broadcast::channel(256)
}

/// Causal ordering of `McpEvent`s, for the
/// `event_order_is_enforced_for_causal_chains` test in
/// `tests/capture_loop_smoke.rs`.
///
/// The "ordering" we promise is:
///   - `ExchangeCaptured` is preceded by the project's open state
///     (the LLM can't see an exchange for a project it doesn't
///     know is open — but the open event itself is on the FULL
///     bus, not the MCP bus, so the MCP order doesn't include
///     it; this is documented in the spec).
///   - `TagAdded` follows `ExchangeCaptured` when the LLM is
///     reasoning about "tag the captured exchange" workflows.
///     In v1 the test calls `insert_exchange` → `tag_upsert` and
///     observes the order as `(ExchangeCaptured, TagAdded)`.
///
/// Future phases may add more ordering constraints (e.g.,
/// `FuzzStarted` preceded by an `ExchangeCaptured` for the
/// template request). Those will extend this enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum McpEventOrder {
    /// An exchange was captured. Any subsequent `TagAdded` for a
    /// tag on this exchange is causally after this.
    ExchangeCaptured = 0,
    /// A tag was added. Any subsequent `TagAttached` for this tag
    /// is causally after this. (For §3.5a we only emit `TagAdded`
    /// from `tag_upsert`; the `TagAttached` event isn't on the
    /// MCP bus yet.)
    TagAdded = 1,
    /// Proxy started — order in v1 is unconstrained relative to
    /// engine events.
    ProxyStarted = 2,
    /// Fuzz started — order in v1 is unconstrained relative to
    /// engine events.
    FuzzStarted = 3,
    /// Proxy stopped — always last (the proxy is going down).
    ProxyStopped = 4,
}

impl McpEventOrder {
    /// Return the order key for an `McpEvent`, if it has one. The
    /// returned key is what `PartialOrd` compares on, so a
    /// sorted-by-key event stream is causally ordered.
    pub fn of(event: &McpEvent) -> Option<Self> {
        match event {
            McpEvent::ExchangeCaptured { .. } => Some(Self::ExchangeCaptured),
            McpEvent::TagAdded { .. } => Some(Self::TagAdded),
            McpEvent::ProxyStarted { .. } => Some(Self::ProxyStarted),
            McpEvent::ProxyStopped { .. } => Some(Self::ProxyStopped),
            McpEvent::FuzzStarted { .. } => Some(Self::FuzzStarted),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The order keys cover all 5 v1 variants. New variants
    /// added in future phases need a key added here (or this
    /// test should grow a wildcard arm, but explicit pinning is
    /// better for the LLM-prompt-keep-small invariant).
    #[test]
    fn order_keys_cover_all_v1_variants() {
        let exchange_id = ExchangeId::new();
        let project_id = ProjectId::new();
        let tag_id = TagId::new();
        // Each of these calls must return Some(_). If you add a
        // new variant, add its key here.
        assert_eq!(
            McpEventOrder::of(&McpEvent::ExchangeCaptured {
                id: exchange_id,
                project_id,
                method: "GET".into(),
                url: "/".into(),
                status: None,
            }),
            Some(McpEventOrder::ExchangeCaptured)
        );
        assert_eq!(
            McpEventOrder::of(&McpEvent::TagAdded {
                id: tag_id,
                project_id,
                name: "vuln".into(),
            }),
            Some(McpEventOrder::TagAdded)
        );
        assert_eq!(
            McpEventOrder::of(&McpEvent::ProxyStarted {
                listener_addr: "127.0.0.1:8080".into(),
            }),
            Some(McpEventOrder::ProxyStarted)
        );
        assert_eq!(
            McpEventOrder::of(&McpEvent::ProxyStopped {
                reason: "shutdown".into(),
            }),
            Some(McpEventOrder::ProxyStopped)
        );
        assert_eq!(
            McpEventOrder::of(&McpEvent::FuzzStarted {
                job_id: "stub".into(),
                project_id,
            }),
            Some(McpEventOrder::FuzzStarted)
        );
    }

    /// `McpEventOrder` is a total order, so `sort()` is stable and
    /// deterministic. This matters for the
    /// `event_order_is_enforced_for_causal_chains` integration
    /// test, which checks `events.windows(2).all(|w| event_order(w[0], w[1]).is_ok())`.
    #[test]
    fn order_is_total() {
        let keys = [
            McpEventOrder::ExchangeCaptured,
            McpEventOrder::TagAdded,
            McpEventOrder::ProxyStarted,
            McpEventOrder::FuzzStarted,
            McpEventOrder::ProxyStopped,
        ];
        let mut sorted = keys;
        sorted.sort();
        assert_eq!(sorted, keys, "the order is already total; sort is a no-op");
    }
}
