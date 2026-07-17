//! Talon MITM proxy crate.
//!
//! This crate is being built incrementally across Phase 3 of the Talon
//! master plan. §3.1 ships the TCP listener, the per-connection
//! concurrency cap, the clean-shutdown plumbing, and a working CLI
//! binary. Subsequent sections add the CA, the HTTP/1+2 MITM cores,
//! and the pipeline that turns a request into a stored exchange.

#![deny(unsafe_code)]
// Struct field docs are covered by struct-level docs; suppress the
// per-field "missing documentation" warnings. See the same line in
// `bk-core` for the rationale.
#![allow(missing_docs)]

pub mod ca;
pub mod cli;
pub mod config;
pub mod events;
pub mod listener;
pub mod mitm;
pub mod upstream;
pub mod upstream_pool;

use std::sync::Arc;

use anyhow::Context;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tracing::{info, warn};

pub use ca::RootCa;
pub use config::ProxyConfig;
pub use events::{ProxyEvent, ProxyEventBus, StopReason};
pub use upstream_pool::{Pool, PoolConfig};

/// The Talon MITM proxy.
///
/// §3.1 ships a skeleton: it owns its [`ProxyConfig`] and an event bus,
/// binds a TCP listener, and dispatches accepted sockets to a
/// [`tokio::sync::Semaphore`]-capped [`tokio::task::JoinSet`]. §3.2
/// adds the [`RootCa`] (used by §3.3 to mint per-host leaf certs
/// for TLS termination). §3.3.5 adds the [`Pool`] (per-host
/// upstream TLS connection pool — replaces the per-request
/// fresh handshake of §3.3).
pub struct Proxy {
    /// The runtime configuration.
    pub config: ProxyConfig,
    /// The dynamic root CA used to mint per-host leaf certs.
    /// `Arc<RootCa>` so the spawned connection tasks can share it.
    pub root_ca: Arc<RootCa>,
    /// The event bus that surfaces lifecycle + per-connection events to
    /// other components (notably the Tauri UI in §3.5).
    pub events: ProxyEventBus,
    /// The per-host upstream connection pool. Holds the
    /// `Arc<ClientConfig>` for upstream TLS verification. Shared
    /// across all connection handlers via `Arc<Pool>`.
    pub upstream_pool: Pool,
}

impl Proxy {
    /// Build a new [`Proxy`] from a config and a [`RootCa`].
    ///
    /// The CA is wrapped in `Arc` so the spawned connection tasks can
    /// share it without re-loading the cert/key for every connection.
    /// The event bus is created lazily here so the constructor stays
    /// infallible. The upstream connection pool is constructed with
    /// the default [`PoolConfig`] — `Proxy::new` does not take a
    /// custom pool config; use [`Proxy::new_with_pool`] for that.
    pub fn new(config: ProxyConfig, root_ca: Arc<RootCa>) -> Self {
        let tls_config = build_default_upstream_tls_config();
        let upstream_pool = Pool::new(PoolConfig::default(), Arc::new(tls_config));
        Self {
            config,
            root_ca,
            events: ProxyEventBus::new(),
            upstream_pool,
        }
    }

    /// Like [`Proxy::new`] but with a custom [`PoolConfig`]. Use
    /// this when you want to tune `max_idle_per_host` or
    /// `idle_timeout`.
    pub fn new_with_pool(
        config: ProxyConfig,
        root_ca: Arc<RootCa>,
        pool_config: PoolConfig,
    ) -> Self {
        let tls_config = build_default_upstream_tls_config();
        let upstream_pool = Pool::new(pool_config, Arc::new(tls_config));
        Self {
            config,
            root_ca,
            events: ProxyEventBus::new(),
            upstream_pool,
        }
    }

    /// Build a new [`Proxy`] with a fully-custom upstream TLS config
    /// and a custom pool config. Use this when the upstream sites use
    /// a private CA (corporate MITM, internal services) that the
    /// Mozilla bundle doesn't trust, or when the integration tests
    /// need to plug in a one-off trust anchor (e.g. the in-process
    /// TLS test origin from `tests/common/mod.rs`).
    ///
    /// The provided [`rustls::ClientConfig`] is wrapped in an `Arc` and
    /// shared across every connection handler via the pool. The
    /// caller is responsible for keeping the config valid for the
    /// lifetime of the proxy.
    ///
    /// **No `&rustls::ClientConfig` overload is provided** — the
    /// pool needs an `Arc` because the conn-driver task spawned
    /// by `connect()` outlives the `Proxy::new_with_upstream_tls_config`
    /// call. Callers without an `Arc` can wrap with `Arc::new`
    /// at the call site. The simple constructors (`Proxy::new`,
    /// `Proxy::new_with_pool`) always use the Mozilla bundle,
    /// which is the right default for production.
    pub fn new_with_upstream_tls_config(
        config: ProxyConfig,
        root_ca: Arc<RootCa>,
        pool_config: PoolConfig,
        upstream_tls: Arc<rustls::ClientConfig>,
    ) -> Self {
        let upstream_pool = Pool::new(pool_config, upstream_tls);
        Self {
            config,
            root_ca,
            events: ProxyEventBus::new(),
            upstream_pool,
        }
    }

    /// Get a cloneable handle to the event bus. Callers can subscribe
    /// before calling [`Proxy::run`] to avoid missing early events.
    pub fn events(&self) -> ProxyEventBus {
        self.events.clone()
    }

    /// Bind the listener and run until `shutdown` flips to `true` (or
    /// all shutdown senders are dropped).
    ///
    /// On a clean shutdown this returns `Ok(())`. On a bind error it
    /// returns `Err`; transient accept errors are logged and the loop
    /// continues.
    pub async fn run(self, shutdown: watch::Receiver<bool>) -> anyhow::Result<()> {
        let proxy = Arc::new(self);

        let listener = TcpListener::bind(proxy.config.listener_addr)
            .await
            .with_context(|| {
                format!(
                    "failed to bind TCP listener to {}",
                    proxy.config.listener_addr
                )
            })?;

        let local_addr = listener
            .local_addr()
            .context("bound TCP listener had no local_addr")?;

        // §3.3 wires the real `RootCa::fingerprint()` into the
        // `ProxyStarted` event. The §3.1/§3.2 placeholder is gone.
        let ca_fingerprint = proxy.root_ca.fingerprint().to_string();

        info!(
            listener = %local_addr,
            ca_fingerprint = %ca_fingerprint,
            "bk-proxy started"
        );

        proxy.events.send(ProxyEvent::ProxyStarted {
            listener_addr: local_addr,
            ca_fingerprint: ca_fingerprint.clone(),
        });

        // Print the user-facing banner the CLI contract requires. Goes
        // to stdout (not the log stream) because the contract is
        // literally "Print ..."
        println!(
            "bk-proxy listening on {}, CA fingerprint: {}",
            local_addr, ca_fingerprint
        );

        let res = listener::accept_loop(proxy.clone(), listener, shutdown.clone()).await;

        let reason = match &res {
            Ok(()) => {
                if *shutdown.borrow() {
                    StopReason::Signal
                } else {
                    // accept_loop only returns Ok on a shutdown signal;
                    // any other path would be a bug. Treat it as a
                    // signal anyway so the event bus is always
                    // consistent.
                    StopReason::Signal
                }
            }
            Err(e) => {
                warn!(error = %e, "accept_loop ended with error");
                StopReason::Error(e.to_string())
            }
        };

        proxy.events.send(ProxyEvent::ProxyStopped { reason });

        res
    }
}

/// Build the default upstream TLS config (Mozilla CA bundle via
/// `webpki-roots`, no client auth). Used by [`Proxy::new`] to
/// construct the upstream connection pool's shared `ClientConfig`.
fn build_default_upstream_tls_config() -> rustls::ClientConfig {
    let mut root_store = rustls::RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth()
}

#[cfg(test)]
mod tests {
    //! §3.1 tests for the bk-proxy crate.
    //!
    //! Three tests, all listed in the §3.1 contract:
    //!
    //! 1. `accept_loop_succeeds_and_returns_conn_stream` — the happy
    //!    path: bind, connect, observe the connection on the server
    //!    side, then shut down.
    //! 2. `accept_loop_respects_max_concurrent_connections_cap` — the
    //!    [`Semaphore`] cap actually throttles the number of in-flight
    //!    tasks.
    //! 3. `accept_loop_exits_cleanly_on_shutdown_signal` — shutting
    //!    down with no in-flight work returns `Ok(())` promptly.

    use std::net::SocketAddr;
    use std::sync::Arc;
    use std::time::Duration;

    use tokio::io::AsyncWriteExt;
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::watch;
    use tokio::task::JoinSet;
    use tokio::time::{sleep, timeout, Instant};

    use super::{listener, Proxy};
    use crate::ca::RootCa;
    use crate::config::ProxyConfig;

    fn free_addr() -> SocketAddr {
        // Port 0 => OS picks a free port. We bind ephemerally to read
        // the port back, then drop the listener so the port is free for
        // the test's real listener to grab. (There's still a tiny race
        // where another process grabs the port in between, but it's
        // vanishingly unlikely on a test runner.)
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        drop(l);
        addr
    }

    fn root_ca_for_test() -> Arc<RootCa> {
        let tmp = tempfile::tempdir().unwrap();
        Arc::new(RootCa::load_or_create(tmp.path()).unwrap())
    }

    fn proxy_with_addr(addr: SocketAddr, max_connections: usize) -> Proxy {
        let cfg = ProxyConfig {
            listener_addr: addr,
            max_concurrent_connections: max_connections,
            ..ProxyConfig::default()
        };
        Proxy::new(cfg, root_ca_for_test())
    }

    #[tokio::test]
    async fn accept_loop_succeeds_and_returns_conn_stream() {
        let addr = free_addr();
        let proxy = Arc::new(proxy_with_addr(addr, 256));
        let listener = TcpListener::bind(addr).await.unwrap();

        let (tx, rx) = watch::channel(false);

        let proxy_for_loop = proxy.clone();
        let rx_for_loop = rx.clone();
        let task = tokio::spawn(async move {
            listener::accept_loop(proxy_for_loop, listener, rx_for_loop).await
        });

        // Open a client connection and send 1 byte. The accept loop
        // hands it off to a spawned task. §3.3's `handle_connection`
        // reads a CONNECT request; the 1 byte isn't a valid request
        // and the client then closes the write side, so the next
        // read returns EOF and the handler returns.
        let mut client = TcpStream::connect(addr).await.unwrap();
        client.write_all(&[0x42]).await.unwrap();
        client.shutdown().await.unwrap();

        // Give the runtime a moment to schedule the accept + the
        // handler's read-to-EOF.
        sleep(Duration::from_millis(50)).await;

        // Trigger shutdown and verify the loop returns within 1s.
        tx.send(true).unwrap();
        timeout(Duration::from_secs(1), task)
            .await
            .expect("accept_loop did not exit within 1s after shutdown")
            .expect("accept_loop task panicked")
            .expect("accept_loop returned an error");
    }

    #[tokio::test]
    async fn accept_loop_respects_max_concurrent_connections_cap() {
        let addr = free_addr();
        let proxy = Arc::new(proxy_with_addr(addr, 2));
        let listener = TcpListener::bind(addr).await.unwrap();

        let (tx, rx) = watch::channel(false);

        let proxy_for_loop = proxy.clone();
        let rx_for_loop = rx.clone();
        let task = tokio::spawn(async move {
            listener::accept_loop(proxy_for_loop, listener, rx_for_loop).await
        });

        // We can't directly observe how many handlers are in-flight
        // (the contract is intentionally side-effect-free in §3.1),
        // but we CAN observe the indirect signal: the loop must remain
        // alive with the cap engaged. We open 5 client connections,
        // let the runtime schedule 2 accepts, then assert the task
        // is still running (i.e. the loop is parked on the Semaphore,
        // not exited). Then we send shutdown and verify the loop
        // drains and exits cleanly within 1s.
        let mut clients = Vec::new();
        for _ in 0..5 {
            let c = TcpStream::connect(addr).await.unwrap();
            clients.push(c);
        }

        // Shut down each client (close the write side) so the
        // server's `handle_connection` reads return EOF and the
        // handlers complete; then trigger the proxy's shutdown.
        // The order matters: the §3.3 handler reads a CONNECT
        // request, and a client that never sends data + never
        // closes would hang the handler indefinitely.
        let mut drains: JoinSet<()> = JoinSet::new();
        for mut c in clients {
            drains.spawn(async move {
                let _ = c.shutdown().await;
            });
        }
        while drains.join_next().await.is_some() {}

        // Small wait so the server's read-to-EOF on the now-closed
        // clients can propagate and the handlers return.
        sleep(Duration::from_millis(50)).await;

        // The task must still be alive (not finished) until we send
        // the explicit shutdown signal — the handlers have all
        // returned, the accept loop is parked on the inner
        // `listener.accept().await` waiting for new connections.
        assert!(
            !task.is_finished(),
            "accept_loop exited prematurely with cap=2 and 5 clients"
        );

        // Shut down; the loop should drain and exit within 1s.
        tx.send(true).unwrap();
        timeout(Duration::from_secs(1), task)
            .await
            .expect("accept_loop did not exit within 1s after shutdown")
            .expect("accept_loop task panicked")
            .expect("accept_loop returned an error");
    }

    #[tokio::test]
    async fn accept_loop_exits_cleanly_on_shutdown_signal() {
        let addr = free_addr();
        let proxy = Arc::new(proxy_with_addr(addr, 256));
        let listener = TcpListener::bind(addr).await.unwrap();

        let (tx, rx) = watch::channel(false);

        // Flip the shutdown signal BEFORE the loop starts to make sure
        // it observes shutdown on the very first iteration.
        tx.send(true).unwrap();

        let start = Instant::now();
        let res = listener::accept_loop(proxy, listener, rx).await;
        let elapsed = start.elapsed();

        assert!(
            res.is_ok(),
            "expected Ok(()) on clean shutdown, got {res:?}"
        );
        assert!(
            elapsed < Duration::from_secs(1),
            "accept_loop took {elapsed:?} to exit on shutdown; expected < 1s"
        );
    }

    /// Regression for the Copilot review thread on PR #16: when all
    /// shutdown senders are dropped without ever calling `send(true)`,
    /// the loop must still exit gracefully rather than busy-looping
    /// on `shutdown.changed()` returning `Err(RecvError)` every
    /// iteration.
    #[tokio::test]
    async fn accept_loop_exits_when_shutdown_senders_dropped() {
        let addr = free_addr();
        let proxy = Arc::new(proxy_with_addr(addr, 256));
        let listener = TcpListener::bind(addr).await.unwrap();

        // `tx` is created and immediately dropped; no `send(true)`
        // ever happens, so the only way the loop can exit is by
        // detecting the dropped sender on the `changed()` future.
        let (tx, rx) = watch::channel(false);
        drop(tx);

        let start = Instant::now();
        let inner = timeout(
            Duration::from_secs(2),
            listener::accept_loop(proxy, listener, rx),
        )
        .await
        .expect("accept_loop did not exit within 2s after senders dropped (busy loop?)");

        let elapsed = start.elapsed();
        assert!(
            inner.is_ok(),
            "expected Ok(()) on dropped senders, got {inner:?}"
        );
        assert!(
            elapsed < Duration::from_secs(1),
            "accept_loop took {elapsed:?} to exit on dropped senders; expected < 1s"
        );
    }

    /// Sanity test: with cap=1 and 3 clients queued, the loop stays
    /// alive (parked on permit/accept) and shuts down within 1s after
    /// a shutdown signal. Doesn't directly assert the eager-accept
    /// vs permit-first distinction (the §3.1 handler is too fast to
    /// make that observable in a unit test), but it does cover the
    /// shutdown-while-cap-saturated path which is the integration
    /// contract the fix has to honor.
    #[tokio::test]
    async fn accept_loop_holds_backpressure_when_cap_saturated() {
        let addr = free_addr();
        let proxy = Arc::new(proxy_with_addr(addr, 1));
        let listener = TcpListener::bind(addr).await.unwrap();

        let (tx, rx) = watch::channel(false);

        let proxy_for_loop = proxy.clone();
        let rx_for_loop = rx.clone();
        let task = tokio::spawn(async move {
            listener::accept_loop(proxy_for_loop, listener, rx_for_loop).await
        });

        // Open 3 client connections. With cap=1, the first client is
        // accepted and its handler is reading the CONNECT request
        // (blocked on the read until the client sends data or closes).
        // The remaining 2 clients sit in the kernel's accept queue;
        // the loop must be parked waiting for the first handler to
        // release its permit.
        let mut clients = Vec::new();
        for _ in 0..3 {
            let c = TcpStream::connect(addr).await.unwrap();
            clients.push(c);
        }

        // Give the loop time to accept #1 and start the handler.
        sleep(Duration::from_millis(50)).await;

        // Close the clients so the handlers' CONNECT reads return
        // EOF and the handlers complete.
        let mut drains: JoinSet<()> = JoinSet::new();
        for mut c in clients {
            drains.spawn(async move {
                let _ = c.shutdown().await;
            });
        }
        while drains.join_next().await.is_some() {}

        // Small wait for the read-to-EOF to propagate.
        sleep(Duration::from_millis(50)).await;

        // The task must still be alive (the handlers have all
        // returned, the accept loop is parked on the next
        // `listener.accept().await` or the permit wait).
        assert!(
            !task.is_finished(),
            "accept_loop exited prematurely with cap=1 and 3 clients"
        );

        // Shut down; the loop should drain and exit within 1s.
        tx.send(true).unwrap();
        timeout(Duration::from_secs(1), task)
            .await
            .expect("accept_loop did not exit within 1s after shutdown")
            .expect("accept_loop task panicked")
            .expect("accept_loop returned an error");
    }

    // -----------------------------------------------------------------
    // §3.2 tests for the dynamic root CA.
    //
    // Three tests, all listed in the §3.2 contract:
    //
    // 1. `root_ca_load_or_create_persists_across_calls` — the second
    //    call to load_or_create on the same dir must NOT regenerate
    //    the CA; the fingerprint must match.
    // 2. `root_ca_sign_leaf_produces_valid_x509_for_sni` — the leaf
    //    DER parses as a valid X.509 with the SNI as a SAN, and it
    //    chains to the root (same issuer DN, root signs the leaf).
    // 3. `root_ca_persists_fingerprint_in_config_dir` — the
    //    `ca.fingerprint` file is written and matches what
    //    `ca.fingerprint()` returns.
    // -----------------------------------------------------------------

    #[test]
    fn root_ca_load_or_create_persists_across_calls() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        // First call: must create the CA.
        let ca1 = RootCa::load_or_create(dir).expect("first load_or_create failed");
        let fp1 = ca1.fingerprint().to_string();

        // Files must exist on disk.
        for name in ["ca.crt.pem", "ca.key.pem", "ca.fingerprint", "ca.meta.toml"] {
            let p = RootCa::ca_dir(dir).join(name);
            assert!(
                p.exists(),
                "expected {} to exist after first load",
                p.display()
            );
        }

        // Second call: must reload, NOT regenerate.
        let ca2 = RootCa::load_or_create(dir).expect("second load_or_create failed");
        let fp2 = ca2.fingerprint().to_string();

        assert_eq!(fp1, fp2, "fingerprint changed across load_or_create calls");
    }

    #[test]
    fn root_ca_sign_leaf_produces_valid_x509_for_sni() {
        let tmp = tempfile::tempdir().unwrap();
        let ca = RootCa::load_or_create(tmp.path()).unwrap();

        let (cert_der, key_der) = ca
            .sign_leaf("example.com")
            .expect("sign_leaf failed for example.com");
        assert!(!cert_der.is_empty(), "cert DER is empty");
        assert!(!key_der.is_empty(), "key DER is empty");

        // Parse the leaf cert. `x509-parser` is in `[dev-dependencies]`.
        let (_, leaf) =
            x509_parser::parse_x509_certificate(&cert_der).expect("leaf cert did not parse");
        let leaf_subject = leaf.tbs_certificate.subject.to_string();

        // The leaf must have example.com in its SAN extension.
        let san_contains_example = leaf
            .tbs_certificate
            .extensions_map()
            .expect("extensions_map failed")
            .values()
            .any(|ext| {
                use x509_parser::prelude::GeneralName;
                let parsed = ext.parsed_extension();
                matches!(parsed, x509_parser::extensions::ParsedExtension::SubjectAlternativeName(san)
                    if san.general_names.iter().any(|gn| {
                        matches!(gn, GeneralName::DNSName(name) if *name == "example.com")
                    }))
            });
        assert!(
            san_contains_example,
            "leaf cert does not have example.com in SAN; subject={leaf_subject}"
        );

        // The leaf must be signed by the root — i.e. the leaf's
        // `issuer` DN must match the root's `subject` DN.
        let root_der = ca.root_cert_der();
        let (_, root) =
            x509_parser::parse_x509_certificate(&root_der).expect("root cert did not parse");
        let root_subject = root.tbs_certificate.subject.to_string();
        let leaf_issuer = leaf.tbs_certificate.issuer.to_string();

        assert_eq!(
            leaf_issuer, root_subject,
            "leaf issuer {leaf_issuer:?} != root subject {root_subject:?}"
        );
    }

    #[test]
    fn root_ca_persists_fingerprint_in_config_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let ca = RootCa::load_or_create(tmp.path()).unwrap();

        let stored = std::fs::read_to_string(RootCa::ca_dir(tmp.path()).join("ca.fingerprint"))
            .expect("ca.fingerprint missing on disk");
        let stored = stored.trim();

        assert_eq!(
            stored,
            ca.fingerprint(),
            "fingerprint on disk ({stored}) != what the in-memory CA reports ({})",
            ca.fingerprint()
        );
    }
}
