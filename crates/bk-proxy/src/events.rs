//! Proxy lifecycle events.
//!
//! §3.1 only ships two variants. Later sections will add per-request
//! and per-connection events for the Tauri UI to consume.

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// Why the proxy stopped.
///
/// Marked `#[non_exhaustive]` per the Phase 10 plugin-system
/// design contract (§5.1 item 1): v2 may add a `PluginPanic`
/// or `HostFunctionTrap` variant without breaking every `match`
/// on this type in v1 code.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum StopReason {
    /// A SIGINT / SIGTERM was received.
    Signal,
    /// A fatal error occurred.
    Error(String),
}

/// Lifecycle and per-connection events emitted by the proxy.
///
/// §3.1 defined `ProxyStarted` and `ProxyStopped`. §3.3 adds
/// `RequestForwarded` so the Tauri UI (and tests) can observe a
/// successful upstream round-trip. The per-direction
/// `RequestCaptured` / `ResponseCaptured` variants land in §3.6.
///
/// **Marked `#[non_exhaustive]` per the Phase 10 plugin-system
/// design contract (§5.1 item 1):** v2 will add `PluginLoaded`,
/// `PluginUnloaded`, `PluginTrapped` (and possibly more) without
/// breaking every `match` on this type in v1 code. **v1 callers
/// must add a wildcard arm (`_ =>`) to every match on this enum.**
/// This is enforced by `cargo clippy --all-targets` if you enable
/// the `clippy::exhaustive_enums` lint at the workspace level
/// (deferred — see Phase 10 §5.2 item 7 for why).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ProxyEvent {
    /// The TCP listener is bound and accepting.
    ProxyStarted {
        /// The address the listener is bound to.
        listener_addr: std::net::SocketAddr,
        /// SHA-256 fingerprint of the CA cert, hex-encoded, colon-separated.
        /// §3.1 shipped a placeholder; §3.3 wires the real `RootCa`
        /// fingerprint.
        ca_fingerprint: String,
    },
    /// The proxy is stopping.
    ProxyStopped {
        /// Why we're stopping.
        reason: StopReason,
    },
    /// A browser-side request was handled by the proxy.
    ///
    /// This event is emitted once per request from inside the
    /// service closure (so keep-alive connections emit N events
    /// for N requests, not one batched event per connection).
    /// It fires for **every** terminal state of the request:
    /// a successful upstream forward, a 501 rejection (the
    /// method is non-GET and §3.3 only forwards GETs), or a
    /// 502 (the upstream dial or the upstream HTTP forward
    /// failed). The `status` field carries the response status
    /// in all three cases — the success path returns the real
    /// upstream status, while the 501/502 paths return the
    /// proxy-generated status.
    ///
    /// The host is the SNI from the CONNECT request — it is the
    /// single source of truth for the upstream hostname (NOT the
    /// `Host:` header, which a malicious client can spoof — design
    /// contract gotcha #1).
    ///
    /// The proxy's h2 server can serve both HTTP/1.1 and HTTP/2
    /// requests on the browser side (whichever the client
    /// selected via ALPN); this event covers both protocols.
    RequestForwarded {
        /// Lowercased SNI / CONNECT target host (no port).
        host: String,
        /// HTTP status code from the upstream response.
        status: u16,
        /// Bytes received from the browser (request headers + body).
        bytes_in: u64,
        /// Bytes sent back to the browser (response headers + body).
        bytes_out: u64,
        /// Wall-clock duration of the CONNECT + TLS + upstream request
        /// + response-stream round-trip, in milliseconds.
        duration_ms: u64,
    },
}

/// A cloneable, broadcast-style handle to the event bus.
#[derive(Clone)]
pub struct ProxyEventBus {
    tx: broadcast::Sender<ProxyEvent>,
}

impl ProxyEventBus {
    /// Create a new event bus. The buffer is small (§3.1 only emits
    /// two events per process lifetime) but generous enough for the
    /// §3.5 UI to miss a few ticks without backpressure.
    pub fn new() -> Self {
        let (tx, _rx) = broadcast::channel(64);
        Self { tx }
    }

    /// Publish an event. Subscribers that fall behind will get a
    /// `RecvError::Lagged`; we swallow that on the send side because
    /// the event bus is best-effort.
    pub fn send(&self, ev: ProxyEvent) {
        // It's OK if no one is listening.
        let _ = self.tx.send(ev);
    }

    /// Subscribe to the event bus. Returns a `broadcast::Receiver`
    /// which yields each [`ProxyEvent`] in order. Lagged subscribers
    /// see a `RecvError::Lagged` and can decide whether to keep up or
    /// give up.
    pub fn subscribe(&self) -> broadcast::Receiver<ProxyEvent> {
        self.tx.subscribe()
    }
}

impl Default for ProxyEventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;

    /// Regression for the Phase 10 plugin-system design contract
    /// (§5.1 item 1): v1 callers must write a wildcard arm when
    /// matching on `ProxyEvent` so that v2's added variants
    /// (e.g. `PluginLoaded`) don't break the match. This test
    /// documents the discipline by example: it pattern-matches
    /// with a wildcard and would still compile if v2 added
    /// `PluginLoaded { name: String }` to the enum.
    ///
    /// Sync (not `#[tokio::test]`) because there's no async
    /// work to do — the value is the compile-time proof that
    /// the wildcard arm is present.
    ///
    /// The wildcard arm is `unreachable_patterns`-warned in v1
    /// (all 3 v1 variants are matched above). In v2, when
    /// `PluginLoaded` / `PluginUnloaded` / `PluginTrapped` are
    /// added to the enum, this arm becomes the catch-all that
    /// keeps the test compiling. The `#[allow]` documents the
    /// intent: "this arm exists for the future, not for now."
    #[allow(unreachable_patterns)]
    #[test]
    fn proxy_event_supports_wildcard_match_for_v2_extensibility() {
        let ev: ProxyEvent = ProxyEvent::ProxyStarted {
            listener_addr: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
            ca_fingerprint: "deadbeef".into(),
        };
        let label = match &ev {
            ProxyEvent::ProxyStarted { .. } => "started",
            ProxyEvent::ProxyStopped { .. } => "stopped",
            ProxyEvent::RequestForwarded { .. } => "forwarded",
            // Wildcard arm — required by `#[non_exhaustive]`.
            // v2's `PluginLoaded`/`PluginUnloaded`/`PluginTrapped`
            // fall here without breaking the build.
            _ => "other",
        };
        assert_eq!(label, "started");
    }
}
