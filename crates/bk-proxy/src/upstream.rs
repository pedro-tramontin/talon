//! HTTP/1.1 upstream client for the MITM proxy.
//!
//! The proxy terminates the browser's TLS, reads the HTTP/1.1 request,
//! then forwards it to the real upstream over a fresh TLS connection.
//! The host comes from the CONNECT request (SNI) — NEVER from the
//! `Host:` header (design contract gotcha #1: confused-deputy).
//!
//! Body streaming is mandatory (open question 7.1): a 500 MB upload
//! must not be buffered in memory. The hyper client uses
//! `hyper::client::conn::http1::handshake` for per-request connections
//! (no pool, per §3.3's default), wrapped around a tokio-rustls
//! `TlsStream` for upstream TLS.
//!
//! Upstream TLS verifies with `webpki-roots` (the Mozilla CA bundle).
//! No `verify_none`, no `dangerous_configuration` — the whole point
//! of the proxy is that the user trusts it because *it* trusts the
//! upstream.

use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use hyper::body::Incoming;
use hyper::{Request, Response};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioIo;
use rustls::ClientConfig;
use rustls_pki_types::ServerName;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tracing::{debug, warn};

/// The body type the proxy sends to the upstream. We use
/// `http_body_util::Empty<Bytes>` for GETs (no request body).
/// POST/PUT with bodies would need a different body type
/// (`http_body_util::Full<Bytes>` for buffered uploads,
/// `http_body_util::StreamBody` for streaming uploads), but
/// §3.3 only forwards GETs — non-GET requests get a 501 from
/// the listener. This `UpstreamBody` alias will be widened
/// to an enum (or trait object) when body forwarding lands.
pub type UpstreamBody = http_body_util::Empty<bytes::Bytes>;

/// Streamed body type returned by `forward_request`. This is the
/// hyper `Incoming` body (the upstream side uses the same hyper
/// client connection as the proxy's h2 server side, so the
/// response body type is fixed by hyper, not by us).
pub type UpstreamResponseBody = Incoming;

/// Send a single HTTP/1.1 request to the upstream and return its
/// response. The body is streamed, not buffered.
///
/// `host` is the SNI from the CONNECT request — used as the upstream
/// hostname for DNS resolution, the SNI for upstream TLS, and the
/// `Host:` header in the forwarded request. **Never** derive any of
/// these from the request itself.
pub async fn forward_request(
    host: &str,
    request: Request<UpstreamBody>,
) -> Result<Response<UpstreamResponseBody>> {
    let tls_config = build_upstream_tls_config()?;
    forward_request_with_tls_config(host, request, Arc::new(tls_config)).await
}

/// Test-only variant of `forward_request` that takes a custom
/// `ClientConfig`. Used by test #4 to inject a trust store that
/// also trusts the test's `RootCa`. **Not part of the public
/// surface; do not call from non-test code.**
#[doc(hidden)]
pub async fn forward_request_with_tls_config(
    host: &str,
    request: Request<UpstreamBody>,
    tls_config: Arc<ClientConfig>,
) -> Result<Response<UpstreamResponseBody>> {
    // Connect TCP to the real upstream. The host has no port (we
    // always forward on 443 — the standard HTTPS port — even if the
    // browser asked for a different port in CONNECT).
    let tcp = TcpStream::connect((host, 443))
        .await
        .with_context(|| format!("upstream TCP connect to {host}:443 failed"))?;

    // Wrap the TCP stream in TLS. The server name uses the SNI
    // (host from CONNECT), NOT the Host header.
    let server_name = ServerName::try_from(host.to_string())
        .map_err(|e| anyhow!("invalid upstream hostname {host:?}: {e}"))?;
    let tls_connector = TlsConnector::from(tls_config);
    let tls_stream = tls_connector
        .connect(server_name, tcp)
        .await
        .with_context(|| format!("upstream TLS handshake to {host} failed"))?;

    // Wrap the TLS stream in TokioIo so hyper 1.x can use it.
    let io = TokioIo::new(tls_stream);

    // Handshake with the hyper HTTP/1.1 client. Per-request, no pool.
    let (mut sender, connection) = hyper::client::conn::http1::handshake(io)
        .await
        .with_context(|| "upstream HTTP/1.1 handshake failed")?;

    // Drive the connection in the background. If it errors, we log
    // and the response body will be empty / error.
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            warn!(error = %e, "upstream connection errored");
        }
    });

    // Send the request and await the response head.
    let response = sender
        .send_request(request)
        .await
        .with_context(|| format!("upstream request to {host} failed"))?;

    debug!(host = %host, status = %response.status(), "upstream response received");
    Ok(response)
}

/// Build the rustls `ClientConfig` for upstream TLS verification,
/// using the Mozilla CA bundle from `webpki-roots`. The returned
/// `ClientConfig` is cheap to clone via `Arc` (rustls configs are
/// internally `Arc`-wrapped).
fn build_upstream_tls_config() -> Result<ClientConfig> {
    let mut root_store = rustls::RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    Ok(ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth())
}

/// Test-only: build a `ClientConfig` that also trusts a custom root
/// cert. Used by the in-process upstream test (test #4) to point
/// the proxy at a TLS server whose cert is signed by the test's
/// `RootCa`. **Not part of the public surface; do not call from
/// non-test code.**
#[doc(hidden)]
pub fn build_upstream_tls_config_with_extra_root(extra_root_der: &[u8]) -> Result<ClientConfig> {
    use rustls_pki_types::CertificateDer;
    let mut root_store = rustls::RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let extra = CertificateDer::from(extra_root_der.to_vec());
    root_store
        .add(extra)
        .map_err(|e| anyhow!("adding test root: {e}"))?;
    Ok(ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth())
}

/// Build a `hyper::Request<Empty<Bytes>>` for the common GET case
/// the proxy needs to forward. The path and method are taken from
/// the browser's request; the `Host:` header is **set by us** to the
/// SNI host (not the browser's Host header).
pub fn build_get_request(host: &str, path_and_query: &str) -> Result<Request<UpstreamBody>> {
    let req = Request::builder()
        .method("GET")
        .uri(format!("https://{host}{path_and_query}"))
        .header("Host", host)
        .header("User-Agent", "talon/0.1")
        .body(UpstreamBody::new())
        .map_err(|e| anyhow!("failed to build upstream request: {e}"))?;
    Ok(req)
}

/// Re-export `HttpConnector` for tests that want to build a hyper
/// server with the same connector type. Not used in the proxy code
/// itself, but the test for `RequestForwarded` event delivery uses
/// a hyper server and benefits from a shared type definition.
pub type SharedHttpConnector = HttpConnector;

// `Empty` and `Full` body types are used by callers that build
// upstream requests. The current code only uses `Empty`; `Full`
// is left in scope for the future POST/PUT branch.
#[allow(dead_code)]
type _UpstreamBodyFuture = http_body_util::Empty<bytes::Bytes>;
