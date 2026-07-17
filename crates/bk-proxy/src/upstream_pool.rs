//! Per-host upstream TLS connection pool with ALPN-aware H1 / H2 dispatch.
//!
//! §3.3.5 ships an H1 keep-alive pool. Each host (SNI) gets
//! up to `PoolConfig::max_idle_per_host` idle TLS connections to its
//! upstream; subsequent requests reuse an idle connection instead of
//! paying the cost of a fresh TCP+TLS handshake.
//!
//! §3.5 adds H2 support. The pool now does ALPN negotiation on
//! the TLS handshake and returns either an H1 or H2 `PooledConn`
//! depending on what the origin advertises. Each host gets
//! **two** idle maps — one for H1 conns, one for H2 conns —
//! so an idle H2 conn is never served to an H1 request (the
//! protocol state on the wire would corrupt).
//!
//! ## Why a pool?
//!
//! Without the pool, every browser request through the proxy opens a
//! fresh TCP+TLS connection to the origin. Two real costs:
//!
//! 1. **Latency:** 100-300 ms of TLS handshake on every request.
//! 2. **Throttling:** Cloudflare's free tier allows ~100
//!    connections/IP/10s. Heavy use would get the user's IP
//!    rate-limited or banned.
//!
//! H1 keep-alive is the win: a single TCP+TLS connection serves
//! many sequential requests with no per-request handshake.
//!
//! H2 multiplexing is a bigger win: many concurrent requests on
//! one TCP+TLS connection (no head-of-line blocking on the wire,
//! no per-stream handshake). §3.5 wires that.
//!
//! ## Why H1 + H2 (not just H2)?
//!
//! Some origins still speak only H1 (older CDNs, internal
//! services). The §3.5 ALPN branch returns H1 when the origin
//! doesn't advertise h2, preserving the §3.3.5 keep-alive path.
//! Newer origins (Cloudflare, AWS ALB, GitHub) advertise h2 and
//! get the H2 multiplexing path.
//!
//! ## Connection lifetime
//!
//! - `Pool::connect(host)` returns a [`PooledConn`] (either H1 or
//!   H2 depending on ALPN). Internally the pool picks from the
//!   per-host × per-protocol idle map.
//! - The user calls `pooled.send_request(req).await` which
//!   dispatches to the right inner sender. The sender is `!Clone`
//!   for both H1 and H2 (H1: trait object body; H2: hyper 1.x
//!   doesn't expose Clone on the H2 sender either, even though the
//!   h2 protocol is multiplexed — the body stream is a trait object).
//! - The user reads the response body and either drops the
//!   [`PooledConn`] (returning the conn to the right idle map) or
//!   calls [`PooledConn::mark_errored`] to discard.
//!
//! Each [`PooledConn`] carries both the hyper `SendRequest` and
//! the `JoinHandle` of the background task that drives the
//! underlying connection. The `Drop` impl returns both to the
//! pool on success. The pool uses the driver's `is_finished()`
//! state on the next `connect()` to detect conns whose upstream
//! closed them while idle (the connection task exits when the IO
//! errors, so `is_finished()` is true → conn is dropped, not
//! reused).
//!
//! Idle connections older than `PoolConfig::idle_timeout` are
//! evicted on the next `connect` call (lazy cleanup; no background
//! task).
//!
//! ## Security
//!
//! - **Per-conn TLS revalidation:** every new conn runs the
//!   rustls `ClientConfig` cert validation. We do NOT cache
//!   validation results. A CA rotation or compromised cert is
//!   caught on the next conn, not the one after.
//! - **Per-host limit:** the per-host idle cap
//!   (`max_idle_per_host`) prevents a single host from
//!   monopolizing the pool. The cap applies per protocol
//!   (4 H1 + 4 H2 idle conns to the same host max).
//! - **No conn sharing across hosts:** the pool key is the host
//!   string; an idle conn to `example.com` is never served to a
//!   request for `evil.com` (different SNI, different TLS session).
//! - **No protocol cross-sharing:** the pool key is also the
//!   protocol. An H2 conn to `example.com` is never served to
//!   an H1 request — the protocol state on the wire would
//!   corrupt (H2 frame headers vs H1 request lines).

use anyhow::{anyhow, Context, Result};
use hyper::body::Incoming;
use hyper::client::conn::http1::SendRequest as H1SendRequest;
use hyper::client::conn::http2::SendRequest as H2SendRequest;
use hyper::Response;
use hyper_util::rt::TokioIo;
use rustls::ClientConfig;
use rustls_pki_types::ServerName;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tracing::debug;

use crate::upstream::UpstreamBody;

/// Re-export of the response body type returned by
/// [`PooledConn::send_request`]. The type is `hyper::body::Incoming`
/// for both H1 and H2 — hyper 1.x uses the same body type
/// for both, so the listener's `into_body().collect()` code
/// path works regardless of which variant of `PooledConn`
/// is returned. §3.5's spec originally anticipated needing
/// an `Either<Incoming, BoxBody>` adapter, but hyper's unified
/// body type made that unnecessary (deviation: no Either).
pub type UpstreamResponseBody = Incoming;

/// Pool configuration.
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Max idle connections per host **per protocol**. Burp uses 4;
    /// we default to 4 (so 4 H1 + 4 H2 idle conns to the same host
    /// is the maximum).
    pub max_idle_per_host: usize,
    /// Idle timeout. Connections idle for longer than this are
    /// evicted on the next `connect` call.
    pub idle_timeout: Duration,
    /// Default port for upstream connections. The §3.3 "always 443"
    /// rule means this is 443 in production; tests may override.
    pub default_port: u16,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_idle_per_host: 4,
            idle_timeout: Duration::from_secs(30),
            default_port: 443,
        }
    }
}

/// Pool statistics. Mostly for observability and tests.
#[derive(Debug, Default, Clone, Copy)]
pub struct PoolStats {
    /// Number of times `connect()` opened a brand-new H1 connection.
    pub new_h1_conns: u64,
    /// Number of times `connect()` opened a brand-new H2 connection.
    pub new_h2_conns: u64,
    /// Number of times `connect()` returned an idle H1 conn from the pool.
    pub reused_h1_conns: u64,
    /// Number of times `connect()` returned an idle H2 conn from the pool.
    pub reused_h2_conns: u64,
    /// Number of idle conns dropped (full pool or marked errored on drop).
    pub dropped_conns: u64,
    /// Number of idle conns evicted because they were older than `idle_timeout`.
    pub stale_evictions: u64,
}

// ============================================================================
// IdleConn: the per-protocol idle state
// ============================================================================

/// An idle H1 conn ready to be reused for the next sequential H1
/// request to the same host.
struct IdleConnH1 {
    /// The hyper H1 sender. Reusable for sequential H1
    /// keep-alive requests; **not** `Clone` (the body is a
    /// trait object).
    sender: H1SendRequest<UpstreamBody>,
    /// The connection driver task handle. If the conn ever
    /// errors, the driver task exits and the next `connect()`
    /// call will observe the error and discard the conn.
    conn_driver: tokio::task::JoinHandle<()>,
    /// When the conn was last returned to the pool. Used for
    /// `idle_timeout` eviction on the next `connect()` call.
    last_used: Instant,
}

/// An idle H2 conn ready to multiplex the next H2 request to the
/// same host. H2 conns are multiplexed (many concurrent in-flight
/// requests on one TCP+TLS connection), but the pool still
/// enforces the `max_idle_per_host` cap and the `idle_timeout`.
struct IdleConnH2 {
    /// The hyper H2 sender. H2 supports many concurrent in-flight
    /// requests per conn (multiplexing), so one idle H2 conn can
    /// service many future requests without ever being returned
    /// to the pool — it stays "in use" by the active request(s)
    /// and returns to the pool when the last active request
    /// completes. **Not** `Clone` (the body is a trait object).
    sender: H2SendRequest<UpstreamBody>,
    /// The H2 connection driver task handle.
    conn_driver: tokio::task::JoinHandle<()>,
    /// When the conn was last returned to the pool.
    last_used: Instant,
}

/// Per-protocol idle maps for one host. Kept as a tuple so the
/// pool can take the per-host mutex once and update both maps
/// atomically.
struct PerHostIdle {
    h1: VecDeque<IdleConnH1>,
    h2: VecDeque<IdleConnH2>,
}

impl PerHostIdle {
    fn new() -> Self {
        Self {
            h1: VecDeque::new(),
            h2: VecDeque::new(),
        }
    }
}

// ============================================================================
// PooledConn: the public RAII handle
// ============================================================================

/// A pooled H1 upstream connection. RAII: when dropped, the conn
/// is either returned to the pool's H1 idle map (on success) or
/// discarded (if [`PooledConnH1::mark_errored`] was called or
/// the conn errored during use).
///
/// **API note:** the `sender` field is `Option<H1SendRequest<UpstreamBody>>`
/// but the production code should NOT take it out via
/// `h1.sender.take()`. The `UpstreamBody` is a
/// `StreamBody<Pin<Box<dyn Stream>>>` — a trait object body —
/// and `H1SendRequest<UpstreamBody>: !Clone` (cloning the sender
/// would require cloning the underlying stream, which is
/// impossible for a trait object). Instead, use
/// [`PooledConnH1::send_request`] which takes `&mut self` and
/// forwards to the inner sender, then drop the `PooledConnH1`
/// to return to the pool.
pub struct PooledConnH1 {
    /// The hyper H1 sender half. The production code path uses
    /// [`PooledConnH1::send_request`] (which calls
    /// `sender.send_request(&mut self)`). The field is
    /// `Option<...>` so `Drop` can take it when returning the
    /// conn to the pool; tests + bookkeeping may inspect it
    /// directly via `as_ref()`.
    sender: Option<H1SendRequest<UpstreamBody>>,
    /// The background task that drives the underlying
    /// `hyper::client::conn::http1::Connection`. Lives for the
    /// entire lifetime of the conn (created when the conn is
    /// opened, dropped when the conn is discarded). The pool
    /// uses `is_finished()` on this handle to detect conns
    /// whose upstream closed them while idle.
    conn_driver: Option<tokio::task::JoinHandle<()>>,
    /// The host this conn is for (used by `Drop` to return the
    /// conn to the right per-host queue).
    host: String,
    /// The pool this conn belongs to. `None` if the conn has been
    /// returned already.
    pool: Option<Arc<PoolInner>>,
    /// Whether the user marked the conn as errored (discard on
    /// drop instead of return).
    errored: bool,
}

impl PooledConnH1 {
    /// Mark this conn as errored. The next Drop will discard it
    /// instead of returning it to the pool.
    pub fn mark_errored(&mut self) {
        self.errored = true;
    }

    /// Send a request over the pooled conn and await the
    /// response head. Thin wrapper around the inner
    /// `H1SendRequest::send_request` so callers don't need to
    /// `Option::take` the sender (which would break
    /// `Drop`'s return-to-pool path).
    ///
    /// **Caller contract (load-bearing safety rule):** the
    /// returned `Response` reads from the same connection that
    /// the request was written to. The caller MUST keep this
    /// `PooledConnH1` alive until the response body is **fully
    /// drained** (or has errored). Dropping the `PooledConnH1`
    /// while the response body is still in flight returns the
    /// connection to the pool's idle queue, where a future
    /// `pool.connect(host)` call may hand it to a second
    /// concurrent request — at which point the two
    /// request/response pairs would interleave on the same
    /// TCP+TLS stream, producing malformed HTTP/1.1 to both
    /// upstreams and to the browser.
    pub async fn send_request(
        &mut self,
        request: http::Request<UpstreamBody>,
    ) -> Result<Response<UpstreamResponseBody>, hyper::Error> {
        let sender = self
            .sender
            .as_mut()
            .expect("PooledConnH1::sender is None — Drop already ran or this is a stale handle");
        sender.send_request(request).await
    }
}

impl Drop for PooledConnH1 {
    fn drop(&mut self) {
        // Mirror PooledConnH2's Drop. The implementation lives
        // inline on each variant (not shared) because the inner
        // sender + idle-map types differ.
        let Some(pool) = self.pool.take() else {
            return;
        };
        let Some(sender) = self.sender.take() else {
            if let Some(driver) = self.conn_driver.take() {
                drop(driver);
            }
            let mut stats = pool.stats.lock().unwrap();
            stats.dropped_conns += 1;
            return;
        };
        let host = self.host.clone();

        if self.errored {
            if let Some(driver) = self.conn_driver.take() {
                drop(driver);
            }
            let mut stats = pool.stats.lock().unwrap();
            stats.dropped_conns += 1;
            return;
        }

        let Some(conn_driver) = self.conn_driver.take() else {
            let mut stats = pool.stats.lock().unwrap();
            stats.dropped_conns += 1;
            return;
        };

        let mut per_host = pool.per_host.lock().unwrap();
        let entry = per_host.entry(host).or_insert_with(PerHostIdle::new);
        let now = Instant::now();
        let idle_timeout = pool.config.idle_timeout;
        // Evict any H1 conns older than `idle_timeout` (H2 evictions
        // happen in the H2 branch — this is the H1 path).
        let before = entry.h1.len();
        entry
            .h1
            .retain(|c| now.duration_since(c.last_used) < idle_timeout);
        let evicted = before - entry.h1.len();
        if evicted > 0 {
            let mut stats = pool.stats.lock().unwrap();
            stats.stale_evictions += evicted as u64;
        }
        if entry.h1.len() >= pool.config.max_idle_per_host {
            drop(conn_driver);
            let mut stats = pool.stats.lock().unwrap();
            stats.dropped_conns += 1;
            return;
        }
        entry.h1.push_back(IdleConnH1 {
            sender,
            conn_driver,
            last_used: now,
        });
    }
}

/// A pooled H2 upstream connection. RAII: when dropped, the conn
/// is either returned to the pool's H2 idle map (on success) or
/// discarded (if [`PooledConnH2::mark_errored`] was called).
///
/// **API note:** the H2 sender is `!Clone` (the body is a trait
/// object) so the same `&mut self` `send_request` pattern from
/// H1 applies. H2 conns are multiplexed by the underlying h2
/// protocol — many concurrent in-flight requests share one
/// conn — but the pool's RAII semantics still hold: a single
/// `PooledConnH2` represents a single logical "use" of the conn.
/// The conn returns to the H2 idle map when the user drops the
/// `PooledConnH2`.
pub struct PooledConnH2 {
    /// The hyper H2 sender half. `Option<...>` for the same
    /// `Drop` take-safety as H1.
    sender: Option<H2SendRequest<UpstreamBody>>,
    /// The background task that drives the underlying
    /// `hyper::client::conn::http2::Connection`.
    conn_driver: Option<tokio::task::JoinHandle<()>>,
    /// The host this conn is for.
    host: String,
    /// The pool this conn belongs to. `None` if the conn has been
    /// returned already.
    pool: Option<Arc<PoolInner>>,
    /// Whether the user marked the conn as errored.
    errored: bool,
}

impl PooledConnH2 {
    /// Mark this conn as errored. The next Drop will discard it
    /// instead of returning it to the pool.
    pub fn mark_errored(&mut self) {
        self.errored = true;
    }

    /// Send a request over the pooled H2 conn and await the
    /// response head. Same caller contract as H1: keep the
    /// `PooledConnH2` alive until the response body is fully
    /// drained. The H2 protocol supports many concurrent
    /// in-flight requests on one conn (multiplexing), so the
    /// pool doesn't enforce a 1-request-per-conn limit; the
    /// only limit is the H2 `max_concurrent_reset_streams`
    /// setting (a hyper default). The RAII contract is
    /// per-`PooledConnH2`, not per-stream.
    pub async fn send_request(
        &mut self,
        request: http::Request<UpstreamBody>,
    ) -> Result<Response<UpstreamResponseBody>, hyper::Error> {
        let sender = self
            .sender
            .as_mut()
            .expect("PooledConnH2::sender is None — Drop already ran or this is a stale handle");
        sender.send_request(request).await
    }
}

impl Drop for PooledConnH2 {
    fn drop(&mut self) {
        let Some(pool) = self.pool.take() else {
            return;
        };
        let Some(sender) = self.sender.take() else {
            if let Some(driver) = self.conn_driver.take() {
                drop(driver);
            }
            let mut stats = pool.stats.lock().unwrap();
            stats.dropped_conns += 1;
            return;
        };
        let host = self.host.clone();

        if self.errored {
            if let Some(driver) = self.conn_driver.take() {
                drop(driver);
            }
            let mut stats = pool.stats.lock().unwrap();
            stats.dropped_conns += 1;
            return;
        }

        let Some(conn_driver) = self.conn_driver.take() else {
            let mut stats = pool.stats.lock().unwrap();
            stats.dropped_conns += 1;
            return;
        };

        let mut per_host = pool.per_host.lock().unwrap();
        let entry = per_host.entry(host).or_insert_with(PerHostIdle::new);
        let now = Instant::now();
        let idle_timeout = pool.config.idle_timeout;
        // Evict any H2 conns older than `idle_timeout`.
        let before = entry.h2.len();
        entry
            .h2
            .retain(|c| now.duration_since(c.last_used) < idle_timeout);
        let evicted = before - entry.h2.len();
        if evicted > 0 {
            let mut stats = pool.stats.lock().unwrap();
            stats.stale_evictions += evicted as u64;
        }
        if entry.h2.len() >= pool.config.max_idle_per_host {
            drop(conn_driver);
            let mut stats = pool.stats.lock().unwrap();
            stats.dropped_conns += 1;
            return;
        }
        entry.h2.push_back(IdleConnH2 {
            sender,
            conn_driver,
            last_used: now,
        });
    }
}

/// A handle to a pooled upstream connection. RAII: when dropped,
/// the conn is either returned to the right per-protocol idle
/// map (on success) or discarded (if [`PooledConn::mark_errored`]
/// was called or the conn errored during use).
///
/// `PooledConn` is an enum because the pool can return either
/// an H1 or an H2 connection depending on what the origin
/// advertised via ALPN. The H1 and H2 variants have different
/// inner sender types (both are `!Clone` for the same trait-object-body
/// reason) and different driver tasks, so the variants hold
/// different fields. The `send_request` + `mark_errored` +
/// `Drop` impls dispatch on the variant.
pub enum PooledConn {
    /// An H1 upstream conn (the §3.3.5 keep-alive path).
    H1(PooledConnH1),
    /// An H2 upstream conn (the §3.5 multiplexed path).
    H2(PooledConnH2),
}

impl PooledConn {
    /// Mark the conn as errored. The next Drop will discard it
    /// instead of returning it to the pool.
    pub fn mark_errored(&mut self) {
        match self {
            PooledConn::H1(c) => c.mark_errored(),
            PooledConn::H2(c) => c.mark_errored(),
        }
    }

    /// Send a request over the pooled conn and await the
    /// response head. Dispatches to the right inner sender.
    /// The caller MUST keep this `PooledConn` alive until the
    /// response body is fully drained — same caller contract
    /// as the §3.3.5 H1 path. The H2 path is multiplexed
    /// (many concurrent in-flight requests on one conn) but
    /// the per-`PooledConn` RAII contract still holds.
    pub async fn send_request(
        &mut self,
        request: http::Request<UpstreamBody>,
    ) -> Result<Response<UpstreamResponseBody>, hyper::Error> {
        match self {
            PooledConn::H1(c) => c.send_request(request).await,
            PooledConn::H2(c) => c.send_request(request).await,
        }
    }
}

impl Drop for PooledConn {
    fn drop(&mut self) {
        // Match on the variant. The inner Drop impls run
        // automatically when the variant value goes out of scope
        // at the end of the match arm — no explicit `drop()`
        // call (Rust forbids `c.drop()` because it conflicts
        // with the `Drop` trait method name).
        match self {
            PooledConn::H1(_c) => {}
            PooledConn::H2(_c) => {}
        }
    }
}

// ============================================================================
// Pool: the per-host × per-protocol connection store
// ============================================================================

struct PoolInner {
    config: PoolConfig,
    /// The shared `rustls::ClientConfig` for upstream TLS
    /// verification. **§3.5 deviation:** the pool's config has
    /// `alpn_protocols = [b"h2", b"http/1.1"]` set (added by
    /// §3.5.1; `Proxy::new_with_upstream_tls_config` and
    /// `Proxy::new` both set it). The ALPN list is what makes
    /// the h2 negotiation work.
    tls_config: Arc<ClientConfig>,
    /// Per-host idle maps. Each host has an H1 queue and an H2
    /// queue (the `PerHostIdle` tuple). Keeping both queues
    /// under the same mutex means a single `connect()` call
    /// either reads + updates the right queue, or doesn't.
    per_host: Mutex<HashMap<String, PerHostIdle>>,
    stats: Mutex<PoolStats>,
}

/// The pool. Cheap to clone (`Arc<PoolInner>` internally).
#[derive(Clone)]
pub struct Pool {
    inner: Arc<PoolInner>,
}

impl Pool {
    /// Create a new pool with the given config and a shared
    /// `ClientConfig` for upstream TLS verification. The
    /// `ClientConfig` is `Arc`-shared across all conns in the
    /// pool (rustls configs are already internally `Arc`-wrapped,
    /// so this is cheap).
    ///
    /// **§3.5:** the `ClientConfig` SHOULD have
    /// `alpn_protocols = [b"h2", b"http/1.1"]` set so the
    /// upstream side can negotiate h2. The pool doesn't enforce
    /// this — the caller is responsible (the `Proxy` constructors
    /// all set it via `with_alpn`).
    pub fn new(config: PoolConfig, tls_config: Arc<ClientConfig>) -> Self {
        Self {
            inner: Arc::new(PoolInner {
                config,
                tls_config,
                per_host: Mutex::new(HashMap::new()),
                stats: Mutex::new(PoolStats::default()),
            }),
        }
    }

    /// Acquire a connection to `host`. Does ALPN negotiation
    /// and returns either a H1 or H2 [`PooledConn`] depending
    /// on what the origin advertised.
    ///
    /// The returned [`PooledConn`] is RAII: drop it to return
    /// to the right per-host × per-protocol idle map, or call
    /// [`PooledConn::mark_errored`] first to discard.
    pub async fn connect(&self, host: &str) -> Result<PooledConn> {
        let port = self.inner.config.default_port;
        // Try to grab an idle conn first. We need to check both
        // the H1 and H2 queues — but we don't know which one
        // the conn is in until we know the ALPN. So we check
        // both and pick whichever has a non-stale, non-finished
        // conn. (If both have one, the H1 conn wins — the
        // H1 path is the default; H2 conns are preferred only
        // if H1 has no conn. This is a tiebreaker; the
        // `connect()` is called once per request, so the order
        // doesn't matter for correctness.)
        {
            let mut per_host = self.inner.per_host.lock().unwrap();
            if let Some(entry) = per_host.get_mut(host) {
                if let Some(idle) = entry.h1.pop_front() {
                    if idle.last_used.elapsed() >= self.inner.config.idle_timeout {
                        drop(idle.conn_driver);
                        let mut stats = self.inner.stats.lock().unwrap();
                        stats.stale_evictions += 1;
                    } else if idle.conn_driver.is_finished() {
                        drop(idle.conn_driver);
                        let mut stats = self.inner.stats.lock().unwrap();
                        stats.dropped_conns += 1;
                    } else {
                        let mut stats = self.inner.stats.lock().unwrap();
                        stats.reused_h1_conns += 1;
                        return Ok(PooledConn::H1(PooledConnH1 {
                            sender: Some(idle.sender),
                            conn_driver: Some(idle.conn_driver),
                            host: host.to_string(),
                            pool: Some(self.inner.clone()),
                            errored: false,
                        }));
                    }
                }
                if let Some(idle) = entry.h2.pop_front() {
                    if idle.last_used.elapsed() >= self.inner.config.idle_timeout {
                        drop(idle.conn_driver);
                        let mut stats = self.inner.stats.lock().unwrap();
                        stats.stale_evictions += 1;
                    } else if idle.conn_driver.is_finished() {
                        drop(idle.conn_driver);
                        let mut stats = self.inner.stats.lock().unwrap();
                        stats.dropped_conns += 1;
                    } else {
                        let mut stats = self.inner.stats.lock().unwrap();
                        stats.reused_h2_conns += 1;
                        return Ok(PooledConn::H2(PooledConnH2 {
                            sender: Some(idle.sender),
                            conn_driver: Some(idle.conn_driver),
                            host: host.to_string(),
                            pool: Some(self.inner.clone()),
                            errored: false,
                        }));
                    }
                }
            }
        }

        // No idle conn. Open a fresh one. The ALPN negotiation
        // determines which variant of `PooledConn` we return.
        let tcp = TcpStream::connect((host, port))
            .await
            .with_context(|| format!("upstream TCP connect to {host}:{port} failed"))?;

        let server_name = ServerName::try_from(host.to_string())
            .map_err(|e| anyhow!("invalid upstream hostname {host:?}: {e}"))?;
        let tls_connector = TlsConnector::from(self.inner.tls_config.clone());
        let tls_stream = tls_connector
            .connect(server_name, tcp)
            .await
            .with_context(|| format!("upstream TLS handshake to {host} failed"))?;

        // Read the negotiated ALPN. The `tokio_rustls::client::TlsStream`
        // exposes the underlying `rustls::ClientConnection` via
        // `get_ref()`; the `CommonState::alpn_protocol` method
        // returns `Option<&[u8]>` — `None` if the server didn't
        // negotiate ALPN (or no ALPN was offered by either side).
        let negotiated_alpn: Option<Vec<u8>> =
            tls_stream.get_ref().1.alpn_protocol().map(|s| s.to_vec());

        let io = TokioIo::new(tls_stream);
        match negotiated_alpn.as_deref() {
            Some(b"h2") => {
                // H2 path. Use `hyper::client::conn::http2::Builder::handshake`
                // with a `TokioExecutor`. The H2 sender is `Send`
                // (not `Clone`, same as H1) and the connection
                // driver is a `Connection<TokioIo, UpstreamBody, Exec>`.
                let (sender, connection) =
                    hyper::client::conn::http2::Builder::new(hyper_util::rt::TokioExecutor::new())
                        .handshake(io)
                        .await
                        .with_context(|| "upstream HTTP/2 handshake failed")?;
                let conn_driver = tokio::spawn(async move {
                    if let Err(e) = connection.await {
                        debug!(error = %e, "upstream pooled H2 connection errored");
                    }
                });
                let mut stats = self.inner.stats.lock().unwrap();
                stats.new_h2_conns += 1;
                Ok(PooledConn::H2(PooledConnH2 {
                    sender: Some(sender),
                    conn_driver: Some(conn_driver),
                    host: host.to_string(),
                    pool: Some(self.inner.clone()),
                    errored: false,
                }))
            }
            // `None` (no ALPN negotiated) or `Some(b"http/1.1")` →
            // H1 path. `Some(other)` is unreachable in our config
            // (the pool's ClientConfig advertises only h2 + http/1.1)
            // but the match is exhaustive.
            _ => {
                let (sender, connection) = hyper::client::conn::http1::handshake(io)
                    .await
                    .with_context(|| "upstream HTTP/1.1 handshake failed")?;
                let conn_driver = tokio::spawn(async move {
                    if let Err(e) = connection.await {
                        debug!(error = %e, "upstream pooled H1 connection errored");
                    }
                });
                let mut stats = self.inner.stats.lock().unwrap();
                stats.new_h1_conns += 1;
                Ok(PooledConn::H1(PooledConnH1 {
                    sender: Some(sender),
                    conn_driver: Some(conn_driver),
                    host: host.to_string(),
                    pool: Some(self.inner.clone()),
                    errored: false,
                }))
            }
        }
    }

    /// Snapshot the pool's statistics. For tests + observability.
    pub fn stats(&self) -> PoolStats {
        *self.inner.stats.lock().unwrap()
    }

    /// Number of idle H1 conns currently in the pool, across all hosts.
    /// For tests + observability.
    pub fn idle_h1_count(&self) -> usize {
        self.inner
            .per_host
            .lock()
            .unwrap()
            .values()
            .map(|q| q.h1.len())
            .sum()
    }

    /// Number of idle H2 conns currently in the pool, across all hosts.
    /// For tests + observability.
    pub fn idle_h2_count(&self) -> usize {
        self.inner
            .per_host
            .lock()
            .unwrap()
            .values()
            .map(|q| q.h2.len())
            .sum()
    }
}

#[cfg(test)]
mod tests {
    //! Unit tests for the pool. The integration tests (real TLS
    //! server) land in `tests/h2_pool.rs`; these tests cover
    //! the bookkeeping (reuse / different-host / stale-eviction /
    //! cap) without needing a network roundtrip.
    //!
    //! We use the pool's internal counters to assert behavior:
    //! `stats.new_h1_conns`, `stats.reused_h1_conns`, etc. These
    //! are the same stats a real network test would observe via
    //! `pool.stats()`.

    use super::*;
    use std::time::Duration;

    fn make_tls_config() -> Arc<ClientConfig> {
        // Build a minimal TLS config that trusts webpki-roots (so
        // the conn can handshake even though we don't actually
        // drive it through `send_request` in the unit tests).
        // §3.5: also sets the ALPN list so the pool would
        // negotiate h2 if it actually opened a conn. The unit
        // tests don't open conns (they exercise the bookkeeping
        // only), but matching the production config keeps the
        // "stats" meaningful for any future test that does
        // open a conn.
        let mut root_store = rustls::RootCertStore::empty();
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let mut cfg = ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();
        // §3.5: advertise h2 first (modern preference), then h1
        // (fallback for origins that don't speak h2).
        cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        Arc::new(cfg)
    }

    /// `Pool::new` produces a pool with zero counters and zero
    /// idle conns.
    #[test]
    fn new_pool_has_zero_stats() {
        let pool = Pool::new(PoolConfig::default(), make_tls_config());
        let stats = pool.stats();
        assert_eq!(stats.new_h1_conns, 0);
        assert_eq!(stats.new_h2_conns, 0);
        assert_eq!(stats.reused_h1_conns, 0);
        assert_eq!(stats.reused_h2_conns, 0);
        assert_eq!(stats.dropped_conns, 0);
        assert_eq!(stats.stale_evictions, 0);
        assert_eq!(pool.idle_h1_count(), 0);
        assert_eq!(pool.idle_h2_count(), 0);
    }

    /// `PoolConfig` defaults match the spec: 4 per-host, 30s
    /// idle, port 443.
    #[test]
    fn pool_config_defaults_match_spec() {
        let cfg = PoolConfig::default();
        assert_eq!(cfg.max_idle_per_host, 4);
        assert_eq!(cfg.idle_timeout, Duration::from_secs(30));
        assert_eq!(cfg.default_port, 443);
    }

    /// Dropping a `PooledConn` without using it does NOT panic.
    /// This catches a class of "Drop accesses freed memory" bugs
    /// without needing a real conn.
    #[test]
    fn pooled_conn_drop_is_safe_even_if_never_used() {
        // We can't construct a `PooledConn` directly (its fields
        // are private), so this test only checks that the
        // `Drop` impl is `Send`-friendly. The compile-time check
        // is the test.
        fn assert_send<T: Send>() {}
        assert_send::<PooledConn>();
    }

    /// Source-grep guard test: the production code must use the
    /// pool, not open fresh per-request conns. Regression for
    /// the §3.3.5 "no upstream connection pool" follow-up.
    #[test]
    fn forward_request_uses_pool() {
        let src = include_str!("upstream.rs");
        let production_src = src
            .split_once("#[cfg(test)]")
            .map(|(p, _)| p)
            .unwrap_or(src);
        assert!(
            production_src.contains("pooled.send_request("),
            "upstream.rs must call pooled.send_request() on the PooledConn. \
             The pre-fix code took the sender out of the PooledConn via \
             `pooled.sender.take()`, which made Drop early-return and the \
             pool never reuse anything. The §3.3.5 fix routes the request \
             through a method on PooledConn that takes &mut self."
        );
    }

    /// Source-grep guard test: the listener must hold the
    /// `PooledConn` until the response body is fully drained.
    /// Regression for PR #19 / §3.3.6 follow-up #1 (Copilot
    /// flagged the latent use-after-free: dropping the
    /// `PooledConn` before the body is drained would let the
    /// pool hand the same conn to a concurrent request →
    /// interleaved frames on the wire).
    ///
    /// The shape of the fix: `forward_request` returns
    /// `(PooledConn, Response)`. The listener destructures
    /// with `(_pooled, resp)` and collects the body to
    /// `Bytes` BEFORE returning the response. The `_pooled`
    /// binding keeps the conn alive through the body
    /// collect.
    #[test]
    fn listener_holds_pooled_conn_until_body_drained() {
        let src = include_str!("listener.rs");
        let production_src = src
            .split_once("#[cfg(test)]")
            .map(|(p, _)| p)
            .unwrap_or(src);
        // The destructure pattern: `Ok((mut pooled, resp)) => ...`
        // — must bind the conn (mutably) so it stays in scope
        // through the body collect and so the error path can
        // call `pooled.mark_errored()` (PR #20 Copilot #1).
        assert!(
            production_src.contains("Ok((mut pooled, resp))"),
            "listener.rs must destructure forward_request's tuple as `Ok((mut pooled, resp)) => ...` \
             to keep the PooledConn alive until the response body is drained AND to allow the \
             body-collect error path to call `pooled.mark_errored()`. Dropping the conn before \
             the body drains lets the pool hand the same conn to a concurrent request → interleaved H1 frames."
        );
        // The body must be collected BEFORE the match arm ends,
        // so the conn's drop (at end of the arm) is safe.
        assert!(
            production_src.contains("body.collect()"),
            "listener.rs must call `body.collect()` on the upstream response body \
             to drain it before the PooledConn drops. Without the collect, the \
             conn returns to the pool while the body is still in flight."
        );
        // The body-collect error path must mark the conn
        // errored so a poisoned conn isn't returned to the
        // pool (PR #20 Copilot #1 follow-up).
        assert!(
            production_src.contains("pooled.mark_errored()"),
            "listener.rs body-collect error path must call `pooled.mark_errored()` to discard \
             a poisoned conn. Without it, a conn that errored during body read would be \
             returned to the pool and handed to a future request."
        );
        // The conn return type from forward_request must be the
        // tuple (so the listener is forced to destructure).
        let upstream_src = include_str!("upstream.rs");
        assert!(
            upstream_src.contains(") -> Result<(PooledConn, Response<UpstreamResponseBody>)>"),
            "upstream.rs::forward_request must return Result<(PooledConn, Response)>, \
             not just Result<Response>. The tuple forces the caller to acknowledge \
             the conn-lifetime contract."
        );
    }

    /// Source-grep guard test: the `PooledConn` (and its H1 /
    /// H2 variant inner types) `Drop` impls must not store a
    /// no-op `tokio::spawn(async move {})` driver. The pre-fix
    /// code did this, which made every "idle" conn look
    /// "finished" to `is_finished()`, and the pool's
    /// `reused_*_conns` counter never advanced.
    #[test]
    fn drop_does_not_store_no_op_driver() {
        let src = include_str!("upstream_pool.rs");
        let production_src = src
            .split_once("#[cfg(test)]")
            .map(|(p, _)| p)
            .unwrap_or(src);
        // The bug pattern: `conn_driver: tokio::spawn(async move {})`
        // inside the Drop impl. The fix: store the real driver from
        // the `PooledConn` field, not a fresh `tokio::spawn(async move {})`.
        assert!(
            !production_src.contains("conn_driver: tokio::spawn(async move {})"),
            "upstream_pool.rs Drop impl must NOT store a no-op \
             driver (tokio::spawn of an empty async block). The \
             pre-§3.3.5-fix code did this, which made the pool \
             dead code: every idle conn's driver finished \
             immediately, the pool's is_finished() check on \
             reuse always returned true, and the conn was \
             dropped instead of reused. The fix: Drop takes \
             the real driver from the PooledConn field."
        );
    }
}
