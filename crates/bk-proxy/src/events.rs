//! Proxy lifecycle events.
//!
//! §3.1 only ships two variants. Later sections will add per-request
//! and per-connection events for the Tauri UI to consume.

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// Why the proxy stopped.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
