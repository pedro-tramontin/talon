//! Shared test helpers for the bk-proxy integration tests.
//!
//! §3.5-prep adds an in-process TLS test origin so we can write
//! real end-to-end MITM tests without hitting the network. The
//! pattern is reused by §3.5's HTTP/2 roundtrip test.
//!
//! **The test origin is a hyper server with a self-signed
//! `rcgen` cert that the proxy's upstream-side rustls config
//! is configured to trust.** The test client side uses the
//! proxy's dynamically-loaded `RootCa` as its trust anchor
//! (the proxy mints a leaf cert for the test origin's SNI
//! from that `RootCa`).
//!
//! The architecture is symmetric:
//! - Test client → proxy: trusts the proxy's `RootCa` (via
//!   the `Arc<rustls::ClientConfig>` the test builds)
//! - Proxy → test origin: trusts the test origin's self-signed
//!   cert (via `Proxy::new_with_upstream_tls_config`)
//!
//! Both sides need a custom TLS config because the default
//! (`webpki-roots`) doesn't trust either the test origin's
//! self-signed cert or the proxy's dynamically-minted leaf.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use http_body_util::Full;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use rcgen::{generate_simple_self_signed, CertifiedKey};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio_rustls::TlsAcceptor;
use tracing::debug;

/// Which HTTP protocol the in-process test origin speaks on
/// accepted connections. §3.5 adds [`Protocol::H2`] so the
/// same `TestOrigin` helper can serve both H1 (default —
/// matches §3.5-prep behavior, no test churn) and H2 (the
/// §3.5 roundtrip + multiplexing tests).
///
/// `#[allow(dead_code)]` because [`Protocol::H2`] is not yet
/// referenced by any committed test — it's the opt-in for the
/// next PR (§3.5 production work). The H1 variant is the
/// default and is used by `mitm_roundtrip_through_in_process_tls_origin`.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// HTTP/1.1 (`hyper::server::conn::http1::Builder`).
    H1,
    /// HTTP/2 (`hyper::server::conn::http2::Builder`).
    H2,
}
/// A self-signed cert + key for the in-process TLS test origin.
///
/// The cert is `CN=localhost`, valid for 1 year, with no
/// extensions beyond the SAN. The private key is PKCS#8 DER
/// (the format `rustls` wants).
pub struct TestOriginCert {
    /// The cert chain as a single-element vector of DER certs.
    /// `rustls` wants `Vec<CertificateDer<'static>>` for the
    /// `with_single_cert` builder.
    pub cert_chain: Vec<CertificateDer<'static>>,
    /// The private key as PKCS#8 DER.
    pub key: PrivateKeyDer<'static>,
}

impl TestOriginCert {
    /// Generate a fresh self-signed cert for `localhost` +
    /// `127.0.0.1`. New per test invocation so fingerprints
    /// don't leak between CI runs.
    ///
    /// Both SANs are listed because the test client uses
    /// `127.0.0.1` as the CONNECT host (per the proxy's
    /// "use the CONNECT target as the SNI" rule), and the
    /// test origin's rustls server uses SAN for SNI-to-cert
    /// selection. The `localhost` SAN is kept for symmetry
    /// with how the real proxy tests would exercise the
    /// cert (a real `https://localhost/...` URL).
    pub fn new() -> Self {
        let CertifiedKey { cert, signing_key } =
            generate_simple_self_signed(vec!["localhost".into(), "127.0.0.1".into()])
                .expect("generate self-signed cert");
        let cert_der = cert.der().to_vec();
        let key_der = signing_key.serialized_der().to_vec();
        Self {
            cert_chain: vec![CertificateDer::from(cert_der)],
            key: PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_der)),
        }
    }

    /// Build a `rustls::ServerConfig` with the given ALPN
    /// protocols (or no ALPN when the slice is empty — the
    /// pre-§3.5 default, where the H1 origin didn't advertise
    /// any protocol negotiation). The list is the order the
    /// server will advertise; the client picks the first one
    /// it supports. §3.5 tests pass `vec![b"h2", b"http/1.1"]`
    /// so the server can negotiate either protocol on the
    /// same listener.
    pub fn server_config_with_alpn(&self, alpn_protocols: &[Vec<u8>]) -> Arc<rustls::ServerConfig> {
        // Use the simple `ServerConfig::builder()` form (not
        // `builder_with_provider`) so the config picks up the
        // process-default crypto provider, the same way the
        // production `ca.rs::tls_server_config` does. The
        // process-default provider is whichever feature is
        // enabled in the workspace (`aws_lc_rs` by default in
        // `rustls 0.23`).
        let mut cfg = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(self.cert_chain.clone(), self.key.clone_key())
            .expect("with_single_cert");
        if !alpn_protocols.is_empty() {
            cfg.alpn_protocols = alpn_protocols.to_vec();
        }
        Arc::new(cfg)
    }
}

/// Handle to a running in-process TLS test origin. Dropping
/// the handle sends a shutdown signal to the server task; the
/// task drains any in-flight requests and exits.
pub struct TestOrigin {
    /// The bound address the test client dials.
    pub addr: SocketAddr,
    /// The cert the proxy's upstream-side rustls config trusts.
    pub cert: Arc<TestOriginCert>,
    /// Count of completed TLS sessions. Tests can read this
    /// to assert the proxy reused / didn't reuse a pooled
    /// upstream conn.
    #[allow(dead_code)]
    pub session_count: Arc<AtomicUsize>,
    // Shutdown sender. Drop reads it to break the server task
    // out of its accept loop. Marked `Option<...>` so the
    // `take()` in `Drop` doesn't need a default.
    shutdown_tx: Option<oneshot::Sender<()>>,
    // Server task `JoinHandle`. Held to keep the task alive
    // for the lifetime of the `TestOrigin` (dropping the
    // handle would still let the task run to completion, but
    // holding it makes the lifetime explicit and silences the
    // unused-field lint).
    #[allow(dead_code)]
    join: Option<tokio::task::JoinHandle<()>>,
}

impl TestOrigin {
    /// Start a new TLS test origin on a free port. Returns
    /// the bound address (which the test client dials) and a
    /// `TestOrigin` handle whose `Drop` impl shuts the server
    /// down. Defaults to HTTP/1.1; use
    /// [`TestOrigin::start_with_protocol`] to opt into H2.
    pub async fn start() -> Result<Self> {
        Self::start_with_protocol(Protocol::H1).await
    }

    /// Start a new TLS test origin speaking the given
    /// protocol. When `protocol == Protocol::H1` the
    /// `ServerConfig` advertises no ALPN (the default
    /// pre-§3.5 behavior — H1 keep-alive still works). When
    /// `protocol == Protocol::H2` the `ServerConfig` advertises
    /// `[h2, http/1.1]` so the proxy's `rustls::ClientConfig`
    /// picks `h2` via ALPN.
    pub async fn start_with_protocol(protocol: Protocol) -> Result<Self> {
        let cert = Arc::new(TestOriginCert::new());
        // The H1 origin advertises no ALPN — falls back to
        // H1 in the pool's ALPN branch. The H2 origin
        // advertises `[h2, http/1.1]` so a client that supports
        // both picks h2 (the proxy advertises the same list, in
        // the same order, so the negotiation is symmetric).
        //
        // **Copilot review (PR #22 #3):** the previous version
        // advertised `[h2, http/1.1]` for `Protocol::H2` but
        // `serve_one` always ran the H2 server builder. If a
        // client offered only `http/1.1`, ALPN would pick it and
        // the H2-only server would fail. Fix: branch the
        // `serve_one` builder on the *negotiated* ALPN value,
        // not on the configured `Protocol`. This matches real
        // production origins (Cloudflare, AWS ALB) that serve
        // both h1 + h2 on the same listener.
        let alpn: Vec<Vec<u8>> = match protocol {
            Protocol::H1 => Vec::new(),
            Protocol::H2 => vec![b"h2".to_vec(), b"http/1.1".to_vec()],
        };
        let server_config = cert.server_config_with_alpn(&alpn);
        let session_count = Arc::new(AtomicUsize::new(0));
        let session_count_clone = session_count.clone();

        let tcp = TcpListener::bind("127.0.0.1:0")
            .await
            .context("bind test origin")?;
        let addr = tcp.local_addr().context("local_addr")?;

        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let acceptor = TlsAcceptor::from(server_config);

        let join = tokio::spawn(async move {
            serve_loop(tcp, acceptor, shutdown_rx, session_count_clone).await
        });

        Ok(Self {
            addr,
            cert,
            session_count,
            shutdown_tx: Some(shutdown_tx),
            join: Some(join),
        })
    }
}

impl Drop for TestOrigin {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            // Best-effort: the receiver may already have been
            // dropped (race with the server task exiting on
            // its own). We don't care — `Drop` is fire-and-forget.
            let _ = tx.send(());
        }
    }
}

/// The server task. Accepts connections in a loop, hands each
/// off to [`serve_one`], and exits when the shutdown signal
/// fires (or the listener errors out).
async fn serve_loop(
    tcp: TcpListener,
    acceptor: TlsAcceptor,
    mut shutdown_rx: oneshot::Receiver<()>,
    session_count: Arc<AtomicUsize>,
) {
    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown_rx => {
                debug!("test origin received shutdown");
                return;
            }
            accept = tcp.accept() => {
                let (stream, _peer) = match accept {
                    Ok(pair) => pair,
                    Err(e) => {
                        debug!(error = %e, "test origin accept error");
                        return;
                    }
                };
                let acceptor = acceptor.clone();
                let count = session_count.clone();
                tokio::spawn(async move {
                    if let Err(e) = serve_one(acceptor, stream).await {
                        debug!(error = %e, "test origin serve_one error");
                    } else {
                        count.fetch_add(1, Ordering::SeqCst);
                    }
                });
            }
        }
    }
}

/// Drive a single TLS session: accept the TLS handshake, then
/// drive one hyper HTTP/1.1 (or HTTP/2) request through it.
/// The response body is the same as the request body (echo),
/// so a test can set a unique token in the request and assert
/// the same token comes back.
///
/// The protocol on the wire is whatever ALPN negotiated —
/// this function does **not** take a `Protocol` argument.
/// The `Protocol` is set at `start_with_protocol` time
/// (it controls the advertised ALPN list) but the *negotiated*
/// value is what we dispatch on here. The configured
/// `Protocol::H2` advertises `[h2, http/1.1]`, so a client
/// that supports both picks h2; a client that only offers
/// h1 falls through to the h1 branch.
async fn serve_one(acceptor: TlsAcceptor, tcp: tokio::net::TcpStream) -> Result<()> {
    let tls = acceptor.accept(tcp).await.context("TLS accept")?;
    // Read the negotiated ALPN. Per RFC 7301 §3.2 the server's
    // preference wins — the value here is what the client
    // picked from our advertised list, not what we wanted.
    //
    // **Copilot review (PR #22 #3):** the previous version
    // ignored the negotiated value and unconditionally ran
    // either the h1 or h2 builder based on the configured
    // `Protocol`. If the client offered only `http/1.1` and
    // the configured `Protocol` was `H2`, ALPN would pick
    // `http/1.1` and the H2 builder would fail. Fix: dispatch
    // on the *negotiated* ALPN, not the configured protocol.
    // An unconfigured origin (`Protocol::H1` with no ALPN)
    // negotiates to `None` and falls back to H1 — same as
    // pre-§3.5 behavior.
    let negotiated: Option<Vec<u8>> = tls.get_ref().1.alpn_protocol().map(|s| s.to_vec());
    let io = TokioIo::new(tls);

    let svc = service_fn(|req: hyper::Request<hyper::body::Incoming>| async move {
        use http_body_util::BodyExt;
        let body = req
            .collect()
            .await
            .map_err(|e| std::io::Error::other(format!("body collect: {e}")))?;
        let echoed = body.to_bytes();
        Ok::<_, std::io::Error>(
            hyper::Response::builder()
                .status(200)
                .body(Full::new(echoed))
                .expect("build echo response"),
        )
    });

    // Dispatch by the *negotiated* ALPN, not the configured
    // protocol. The `protocol` parameter is still in the
    // signature so callers can opt into ALPN via
    // `start_with_protocol(Protocol::H2)`; the actual
    // protocol on the wire is whatever the client picked.
    match negotiated.as_deref() {
        Some(b"h2") => {
            hyper::server::conn::http2::Builder::new(hyper_util::rt::TokioExecutor::new())
                .serve_connection(io, svc)
                .await
                .context("hyper h2 serve_connection")?;
        }
        // `None` (no ALPN configured) or `Some(b"http/1.1")`
        // (ALPN negotiated h1) both fall through to h1.
        // `Some(other)` is unreachable in our config (we
        // only advertise h2 + http/1.1) but the match is
        // exhaustive to keep clippy happy.
        _ => {
            http1::Builder::new()
                .serve_connection(io, svc)
                .await
                .context("hyper h1 serve_connection")?;
        }
    }
    Ok(())
}

/// A request body + the token the test expects to come back in
/// the response body. The body is `"prefix:" + token + ":suffix"`,
/// ~30 bytes total — small enough for the in-process test, large
/// enough to exercise the body pipeline (more than the 1-byte
/// payloads the §3.3 unit tests used).
pub struct EchoAssertion {
    /// The unique token that should be present in both the
    /// request and the response body.
    pub token: String,
    /// The full request body the test sends.
    pub request_body: Vec<u8>,
}

impl EchoAssertion {
    /// Build a new assertion with a unique token. The token
    /// defaults to a UUID-derived string (callers can override
    /// to use a deterministic value for grep-friendly tests).
    pub fn new(token: impl Into<String>) -> Self {
        let token = token.into();
        let request_body = format!("prefix:{token}:suffix").into_bytes();
        Self {
            token,
            request_body,
        }
    }
}
