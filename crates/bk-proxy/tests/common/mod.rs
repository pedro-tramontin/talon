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

    /// Build a `rustls::ServerConfig` from this cert + key. The
    /// `ServerConfig` has no client auth (the test origin
    /// doesn't need to authenticate the proxy).
    pub fn server_config(&self) -> Arc<rustls::ServerConfig> {
        // Use the simple `ServerConfig::builder()` form (not
        // `builder_with_provider`) so the config picks up the
        // process-default crypto provider, the same way the
        // production `ca.rs::tls_server_config` does. The
        // process-default provider is whichever feature is
        // enabled in the workspace (`aws_lc_rs` by default in
        // `rustls 0.23`).
        Arc::new(
            rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(self.cert_chain.clone(), self.key.clone_key())
                .expect("with_single_cert"),
        )
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
    /// down.
    pub async fn start() -> Result<Self> {
        let cert = Arc::new(TestOriginCert::new());
        let server_config = cert.server_config();
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
/// drive one hyper HTTP/1.1 request through it. The response
/// body is the same as the request body (echo), so a test can
/// set a unique token in the request and assert the same
/// token comes back.
async fn serve_one(acceptor: TlsAcceptor, tcp: tokio::net::TcpStream) -> Result<()> {
    let tls = acceptor.accept(tcp).await.context("TLS accept")?;
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

    http1::Builder::new()
        .serve_connection(io, svc)
        .await
        .context("hyper serve_connection")?;
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
