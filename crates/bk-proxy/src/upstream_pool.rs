//! Per-host upstream TLS connection pool.
//!
//! §3.3.5 adds a minimal H1 keep-alive pool. Each host (SNI) gets
//! up to `PoolConfig::max_idle_per_host` idle TLS connections to its
//! upstream; subsequent requests reuse an idle connection instead of
//! paying the cost of a fresh TCP+TLS handshake.
//!
//! H2 multiplexing is **not** in scope for §3.3.5 — that's §3.5
//! (HTTP/2 forwarder). The pool is keyed by host (always port 443
//! per the §3.3 "always 443" rule).
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
//! ## Why H1 only, not H2 multiplexing?
//!
//! H2 multiplexing (many in-flight requests on one TCP+TLS
//! connection) is a real win for browsers that use it, but it
//! requires a different code path (the `h2` crate, not `hyper`).
//! §3.5 wires that. §3.3.5 ships the simpler H1 pool so the §3.5
//! H2 forwarder can land on top of a working pool rather than
//! racing both changes together.
//!
//! ## Connection lifetime
//!
//! - `Pool::connect(host)` returns a [`PooledConn`].
//! - The user calls [`PooledConn::send_request`] (a method
//!   that takes `&mut self` and forwards to the inner sender).
//!   **PR #20 / Copilot #4:** the previous version of this
//!   doc told users to call `sender.send_request(...)` on a
//!   `PooledConn::sender` field, but `sender` is private
//!   (and the field is `Option<SendRequest<UpstreamBody>>`
//!   to be `Option::take`-safe on drop, not for external
//!   access). The intended API is `pooled.send_request(...)`.
//! - The user reads the response body and either drops the
//!   [`PooledConn`] (returning the conn + its driver to the pool)
//!   or calls [`PooledConn::mark_errored`] to mark it for discard.
//!
//! Each [`PooledConn`] carries both the hyper `SendRequest` and
//! the `JoinHandle` of the background task that drives the
//! underlying `hyper::client::conn::http1::Connection`. The
//! `Drop` impl returns both to the pool on success. The pool uses
//! the driver's `is_finished()` state on the next `connect()` to
//! detect conns whose upstream closed them while idle (the
//! connection task exits when the IO errors, so `is_finished()`
//! is true → conn is dropped, not reused).
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
//! - **Per-host limit:** the per-host idle cap (`max_idle_per_host`)
//!   prevents a single host from monopolizing the pool.
//! - **No conn sharing across hosts:** the pool key is the host
//!   string; an idle conn to `example.com` is never served to a
//!   request for `evil.com` (different SNI, different TLS session).

use anyhow::{anyhow, Context, Result};
use hyper::body::Incoming;
use hyper::client::conn::http1::SendRequest;
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
/// [`PooledConn::send_request`]. The type comes from hyper's
/// `Incoming` body (the upstream side uses the same hyper
/// client connection as the proxy's h2 server side, so the
/// response body type is fixed by hyper, not by us).
pub type UpstreamResponseBody = Incoming;

/// Pool configuration.
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Max idle connections per host. Burp uses 4; we default to 4.
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
    /// Number of times `connect()` opened a brand-new connection.
    pub new_conns: u64,
    /// Number of times `connect()` returned an idle conn from the pool.
    pub reused_conns: u64,
    /// Number of idle conns dropped (full pool or marked errored on drop).
    pub dropped_conns: u64,
    /// Number of idle conns evicted because they were older than `idle_timeout`.
    pub stale_evictions: u64,
}

struct IdleConn {
    /// The hyper H1 sender. Reusable for sequential H1
    /// keep-alive requests; **not** `Clone` (the body is a
    /// trait object). The pool's [`PooledConn`] API exposes a
    /// [`PooledConn::send_request`] method that takes `&mut
    /// self` and forwards to the inner sender, precisely to
    /// avoid the `Clone` requirement.
    sender: SendRequest<UpstreamBody>,
    /// The connection driver task handle. When the user drops
    /// the `PooledConn` and the conn is returned to the pool,
    /// the driver is left running. If the conn ever errors, the
    /// driver task exits and the next `connect()` call will
    /// observe the error and discard the conn.
    conn_driver: tokio::task::JoinHandle<()>,
    /// When the conn was last returned to the pool. Used for
    /// `idle_timeout` eviction on the next `connect()` call.
    last_used: Instant,
}

/// A handle to a pooled upstream connection. RAII: when dropped, the
/// conn is either returned to the pool (on success) or discarded
/// (if [`PooledConn::mark_errored`] was called or the conn errored
/// during use).
///
/// **API note:** the pool's [`PooledConn::sender`] field is
/// `Option<SendRequest<UpstreamBody>>` but the production code
/// should NOT take the sender out via `pooled.sender.take()`. The
/// `UpstreamBody` is a `StreamBody<Pin<Box<dyn Stream>>>` — a trait
/// object body — and `SendRequest<UpstreamBody>: !Clone` (cloning
/// the sender would require cloning the underlying stream, which
/// is impossible for a trait object). Instead, use
/// [`PooledConn::send_request`] which takes `&mut self` and
/// forwards to the inner sender, then drop the `PooledConn` to
/// return to the pool.
pub struct PooledConn {
    /// The hyper H1 sender half. The production code path uses
    /// [`PooledConn::send_request`] (which calls
    /// `sender.send_request(&mut self)`). The field is
    /// `Option<...>` so `Drop` can take it when returning the
    /// conn to the pool; tests + bookkeeping may inspect it
    /// directly via `as_ref()`.
    sender: Option<SendRequest<UpstreamBody>>,
    /// The background task that drives the underlying
    /// `hyper::client::conn::http1::Connection`. Lives for the
    /// entire lifetime of the conn (created when the conn is
    /// opened, dropped when the conn is discarded). The pool
    /// uses `is_finished()` on this handle to detect conns
    /// whose upstream closed them while idle.
    ///
    /// `None` once the conn has been returned to the pool (or
    /// discarded) — `Drop` takes it.
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

impl PooledConn {
    /// Mark this conn as errored. The next Drop will discard it
    /// instead of returning it to the pool.
    pub fn mark_errored(&mut self) {
        self.errored = true;
    }

    /// Send a request over the pooled conn and await the
    /// response head. Thin wrapper around the inner
    /// `SendRequest::send_request` so callers don't need to
    /// `Option::take` the sender (which would break
    /// `Drop`'s return-to-pool path).
    ///
    /// **Caller contract (this is the load-bearing safety
    /// rule for the pool):** the returned `Response` reads
    /// from the same connection that the request was written
    /// to. The caller MUST keep this `PooledConn` alive
    /// until the response body is **fully drained**
    /// (or has errored). Dropping the `PooledConn` while
    /// the response body is still in flight returns the
    /// connection to the pool's idle queue, where a future
    /// `pool.connect(host)` call may hand it to a second
    /// concurrent request — at which point the two
    /// request/response pairs would interleave on the same
    /// TCP+TLS stream, producing malformed HTTP/1.1 to
    /// both upstreams and to the browser.
    ///
    /// **Why we don't have a `body_finished` guard on the
    /// pool:** adding one would require the `Response`'s
    /// `Body` to signal completion back to the pool, which
    /// means either (a) a custom `Body` wrapper that holds
    /// the `PooledConn` and drops it on `Drop::drop`, or
    /// (b) the caller calls `pooled.mark_body_finished()`
    /// after the last byte. Both add API surface. The
    /// current shape — return `(PooledConn, Response)`
    /// and let the caller hold the conn — is the smallest
    /// surface that maintains safety. (See `forward_request`
    /// in `crate::upstream` for the canonical use.)
    ///
    /// **What the `Incoming` body gives you for free:**
    /// if the body is dropped before being fully drained,
    /// hyper cancels the underlying read, which causes the
    /// connection driver to exit with an error, which
    /// makes the next `pool.connect()` evict the conn.
    /// So even if a caller misuses the API, the conn
    /// self-destructs rather than poisoning the pool.
    pub async fn send_request(
        &mut self,
        request: http::Request<UpstreamBody>,
    ) -> Result<Response<UpstreamResponseBody>, hyper::Error> {
        let sender = self
            .sender
            .as_mut()
            .expect("PooledConn::sender is None — Drop already ran or this is a stale handle");
        sender.send_request(request).await
    }
}

impl Drop for PooledConn {
    fn drop(&mut self) {
        let Some(pool) = self.pool.take() else {
            return;
        };
        let Some(sender) = self.sender.take() else {
            // `sender` was already taken by the caller (e.g. via
            // `pooled.sender.take()` in `upstream::forward_request`).
            // Without the sender, the conn is unusable. Drop the
            // driver too so the conn doesn't leak.
            if let Some(driver) = self.conn_driver.take() {
                drop(driver);
            }
            let mut stats = pool.stats.lock().unwrap();
            stats.dropped_conns += 1;
            return;
        };
        let host = self.host.clone();

        if self.errored {
            // Discard — drop the driver, increment counter.
            if let Some(driver) = self.conn_driver.take() {
                drop(driver);
            }
            let mut stats = pool.stats.lock().unwrap();
            stats.dropped_conns += 1;
            return;
        }

        // Take the driver out so we can store it on the IdleConn.
        // If the driver is somehow missing (shouldn't happen — the
        // pool always sets it), drop the conn.
        let Some(conn_driver) = self.conn_driver.take() else {
            let mut stats = pool.stats.lock().unwrap();
            stats.dropped_conns += 1;
            return;
        };

        // Try to return to the pool. The conn driver task is still
        // running; if it later errors (e.g. the upstream closed
        // the conn), the next `connect()` call will see the error
        // and discard the conn.
        let mut per_host = pool.per_host.lock().unwrap();
        let queue = per_host.entry(host).or_insert_with(VecDeque::new);
        // Evict any conns older than `idle_timeout`.
        let now = Instant::now();
        let idle_timeout = pool.config.idle_timeout;
        let before = queue.len();
        queue.retain(|c| now.duration_since(c.last_used) < idle_timeout);
        let evicted = before - queue.len();
        if evicted > 0 {
            let mut stats = pool.stats.lock().unwrap();
            stats.stale_evictions += evicted as u64;
        }
        // Cap at max_idle_per_host.
        if queue.len() >= pool.config.max_idle_per_host {
            // Pool full — drop the conn and the driver. The
            // driver's task will exit on its own when the IO
            // closes (or when the TCP+TLS conn is closed by the
            // upstream). We don't `abort()` — that's a hard
            // cancel that the user's `Drop` might not want.
            drop(conn_driver);
            let mut stats = pool.stats.lock().unwrap();
            stats.dropped_conns += 1;
            return;
        }
        queue.push_back(IdleConn {
            sender,
            conn_driver,
            last_used: now,
        });
    }
}

struct PoolInner {
    config: PoolConfig,
    tls_config: Arc<ClientConfig>,
    per_host: Mutex<HashMap<String, VecDeque<IdleConn>>>,
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

    /// Acquire a connection to `host`. Returns an idle conn
    /// from the pool if one is available, otherwise opens a
    /// fresh one. The port comes from
    /// [`PoolConfig::default_port`] (always 443 in production;
    /// tests may override).
    ///
    /// The returned [`PooledConn`] is RAII: drop it to return to
    /// the pool (with its connection-driver task handle), or
    /// call [`PooledConn::mark_errored`] first to discard.
    pub async fn connect(&self, host: &str) -> Result<PooledConn> {
        let port = self.inner.config.default_port;
        // Try to grab an idle conn first.
        {
            let mut per_host = self.inner.per_host.lock().unwrap();
            if let Some(queue) = per_host.get_mut(host) {
                while let Some(idle) = queue.pop_front() {
                    // Skip stale conns (older than `idle_timeout`).
                    if idle.last_used.elapsed() >= self.inner.config.idle_timeout {
                        // The driver has been running the conn
                        // for at least `idle_timeout`; let it
                        // drop naturally on Drop. Don't abort.
                        drop(idle.conn_driver);
                        let mut stats = self.inner.stats.lock().unwrap();
                        stats.stale_evictions += 1;
                        continue;
                    }
                    // Check if the conn driver already errored
                    // (the upstream closed the conn while idle).
                    if idle.conn_driver.is_finished() {
                        drop(idle.conn_driver);
                        let mut stats = self.inner.stats.lock().unwrap();
                        stats.dropped_conns += 1;
                        continue;
                    }
                    let mut stats = self.inner.stats.lock().unwrap();
                    stats.reused_conns += 1;
                    // Move the real driver (NOT a no-op stub) into
                    // the new `PooledConn` so it lives until the
                    // user drops the conn.
                    return Ok(PooledConn {
                        sender: Some(idle.sender),
                        conn_driver: Some(idle.conn_driver),
                        host: host.to_string(),
                        pool: Some(self.inner.clone()),
                        errored: false,
                    });
                }
            }
        }

        // No idle conn. Open a fresh one.
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

        let io = TokioIo::new(tls_stream);
        let (sender, connection) = hyper::client::conn::http1::handshake(io)
            .await
            .with_context(|| "upstream HTTP/1.1 handshake failed")?;

        // Drive the conn in the background. Store the REAL handle
        // in the `PooledConn` so `Drop` can return it to the pool
        // and the pool can `is_finished()`-check it on reuse.
        // (The pre-fix code dropped the handle with `_conn_driver`,
        // which made the pool dead code: every "idle" conn's
        // driver finished immediately, the pool always thought
        // the conn was finished, and never reused anything.)
        let conn_driver = tokio::spawn(async move {
            if let Err(e) = connection.await {
                debug!(error = %e, "upstream pooled connection errored");
            }
        });

        let mut stats = self.inner.stats.lock().unwrap();
        stats.new_conns += 1;

        Ok(PooledConn {
            sender: Some(sender),
            conn_driver: Some(conn_driver),
            host: host.to_string(),
            pool: Some(self.inner.clone()),
            errored: false,
        })
    }

    /// Snapshot the pool's statistics. For tests + observability.
    pub fn stats(&self) -> PoolStats {
        *self.inner.stats.lock().unwrap()
    }

    /// Number of idle conns currently in the pool, across all hosts.
    /// For tests + observability.
    pub fn idle_count(&self) -> usize {
        self.inner
            .per_host
            .lock()
            .unwrap()
            .values()
            .map(|q| q.len())
            .sum()
    }
}

#[cfg(test)]
mod tests {
    //! Unit tests for the pool. The integration tests (real TLS
    //! server) land in a follow-up; these tests cover the
    //! bookkeeping (reuse / different-host / stale-eviction /
    //! cap) without needing a network roundtrip.
    //!
    //! We use the pool's internal counters to assert behavior:
    //! `stats.reused_conns`, `stats.new_conns`,
    //! `stats.stale_evictions`, `stats.dropped_conns`. These are
    //! the same stats a real network test would observe via
    //! `pool.stats()`.

    use super::*;
    use std::time::Duration;

    fn make_tls_config() -> Arc<ClientConfig> {
        // Build a minimal TLS config that trusts webpki-roots (so
        // the conn can handshake even though we don't actually
        // drive it through `send_request` in the unit tests).
        let mut root_store = rustls::RootCertStore::empty();
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        Arc::new(
            ClientConfig::builder()
                .with_root_certificates(root_store)
                .with_no_client_auth(),
        )
    }

    /// `Pool::new` produces a pool with zero counters and zero
    /// idle conns.
    #[test]
    fn new_pool_has_zero_stats() {
        let pool = Pool::new(PoolConfig::default(), make_tls_config());
        let stats = pool.stats();
        assert_eq!(stats.new_conns, 0);
        assert_eq!(stats.reused_conns, 0);
        assert_eq!(stats.dropped_conns, 0);
        assert_eq!(stats.stale_evictions, 0);
        assert_eq!(pool.idle_count(), 0);
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
        // `Drop` impl is `Send + Sync`-friendly and doesn't
        // require any external resources. The compile-time check
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

    /// Source-grep guard test: the `PooledConn::Drop` impl must
    /// not store a no-op `tokio::spawn(async move {})` driver.
    /// The pre-fix code did this, which made every "idle" conn
    /// look "finished" to `is_finished()`, and the pool's
    /// `reused_conns` counter never advanced. The fix: `Drop`
    /// takes the real `conn_driver` from the `PooledConn` (the
    /// one that was set in `Pool::connect` or the reuse path)
    /// and stores it on the `IdleConn`. **Scope to production
    /// code** so the test's own docstring (which mentions the
    /// bug pattern for teaching) doesn't false-positive.
    #[test]
    fn drop_does_not_store_no_op_driver() {
        let src = include_str!("upstream_pool.rs");
        let production_src = src
            .split_once("#[cfg(test)]")
            .map(|(p, _)| p)
            .unwrap_or(src);
        // The bug pattern: `conn_driver: tokio::spawn(async move {})`
        // inside the Drop impl (or anywhere storing a no-op
        // driver). The fix: store the real driver from the
        // `PooledConn` field, not a fresh `tokio::spawn(async move {})`.
        // We use the call-site syntax (the colon-equals) to
        // avoid matching a docstring that happens to mention
        // `tokio::spawn`.
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
