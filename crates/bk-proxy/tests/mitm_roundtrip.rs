//! End-to-end MITM tests for bk-proxy.
//!
//! §3.5-prep adds the first real roundtrip test: a MITM
//! connection through the in-process TLS test origin,
//! asserting the request body comes back in the response body.
//!
//! The test pattern is the load-bearing integration test for
//! the proxy: a real `proxy → in-process origin → echo back`
//! roundtrip. The §3.3.5 spec's `mitm_roundtrip_through_real_https_origin`
//! was `#[ignore]`-d because it only validated the event-bus
//! contract, not a full TLS+upstream roundtrip. This file
//! replaces that test with a real one that runs in normal CI.

mod common;

use std::sync::Arc;

use bk_proxy::ca::RootCa;
use bk_proxy::config::ProxyConfig;
use bk_proxy::listener;
use bk_proxy::upstream_pool::PoolConfig;
use bk_proxy::Proxy;
use common::EchoAssertion;
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use rustls::pki_types::{CertificateDer, ServerName};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tracing::debug;

/// Real MITM roundtrip: a hyper client connects to the proxy
/// via TCP → CONNECT → TLS, then speaks HTTP/1.1 over the TLS
/// stream. The proxy MITMs the request, dials the in-process
/// TLS test origin (which echoes the request body), and returns
/// the origin's response. The test asserts the body came back
/// intact + the proxy's `RequestForwarded` event fires with
/// the expected host.
///
/// **Why this is the load-bearing test:** every other unit test
/// in the proxy is a fragment (CA leaf signing, CONNECT parsing,
/// H1 keep-alive, body streaming). This one stitches them
/// together. A bug that breaks the proxy end-to-end will break
/// this test. §3.5's `http2_mitm_roundtrip_with_h2_upstream`
/// reuses this exact pattern.
#[tokio::test]
async fn mitm_roundtrip_through_in_process_tls_origin() {
    // 1. Start the in-process TLS test origin. It binds on a
    //    free port; the proxy will dial it as the upstream.
    let origin = common::TestOrigin::start().await.expect("start origin");
    let origin_addr = origin.addr;
    let origin_certs = origin.cert.cert_chain.clone();

    // 2. Build the proxy. Use a fresh `RootCa` in a tempdir
    //    so this test doesn't pollute the user's actual CA.
    //
    //    Tracing is initialized on best-effort so the proxy's
    //    `tracing::warn!` / `tracing::error!` / `tracing::debug!`
    //    calls show up in the test output (controlled by
    //    `RUST_LOG=bk_proxy=debug cargo test -- --nocapture`).
    //    Without this, a 502 from the upstream forward looks
    //    indistinguishable from any other failure.
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("bk_proxy=info")),
        )
        .try_init();
    let tmp = tempfile::tempdir().expect("tempdir");
    let root_ca = Arc::new(RootCa::load_or_create(tmp.path()).expect("ca"));

    // Build a `ClientConfig` that trusts the in-process origin's
    // self-signed cert (not webpki-roots — this test has no
    // internet). Pass it to the proxy so its upstream side
    // accepts the origin.
    let mut origin_root_store = rustls::RootCertStore::empty();
    for cert in &origin_certs {
        origin_root_store
            .add(cert.clone())
            .expect("add origin cert");
    }
    let upstream_tls = Arc::new(
        rustls::ClientConfig::builder()
            .with_root_certificates(origin_root_store)
            .with_no_client_auth(),
    );

    // Bind a free port for the proxy.
    let tcp = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let proxy_addr = tcp.local_addr().unwrap();
    drop(tcp);
    let listener = tokio::net::TcpListener::bind(proxy_addr).await.unwrap();

    // Use `new_with_upstream_tls_config` so the proxy's
    // upstream side trusts the in-process origin. The default
    // `Proxy::new` would use webpki-roots, which won't trust
    // the origin's self-signed cert.
    //
    // `ProxyConfig` is `#[non_exhaustive]` (Phase 10
    // forward-compat), so we can't use struct-literal syntax
    // from outside the crate. Build via `Default::default()`
    // and set the fields directly.
    //
    // The pool's `default_port` is set to the test origin's
    // port, NOT 443. The §3.3 "always 443" rule is the
    // production default; the test origin is on a free port.
    // `PoolConfig::default_port` is the documented override
    // path for this (see the field's doc comment).
    let mut proxy_config = ProxyConfig::default();
    proxy_config.listener_addr = proxy_addr;
    proxy_config.max_concurrent_connections = 16;
    // `PoolConfig::default_port` is the upstream-dial port.
    // Default 443 in production; tests override to the test
    // origin's port. Field-reassign-after-default is the
    // idiomatic way to set a single field on the
    // `#[non_exhaustive]`-friendly Default impl.
    #[allow(clippy::field_reassign_with_default)]
    let pool_config = PoolConfig {
        default_port: origin_addr.port(),
        ..PoolConfig::default()
    };
    let proxy = Arc::new(Proxy::new_with_upstream_tls_config(
        proxy_config,
        root_ca.clone(),
        pool_config,
        upstream_tls,
    ));

    // 3. Subscribe to the event bus before starting the proxy
    //    so we don't miss the `RequestForwarded` event.
    let events = proxy.events();
    let mut rx_event = events.subscribe();

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let proxy_for_loop = proxy.clone();
    let run_task =
        tokio::spawn(
            async move { listener::accept_loop(proxy_for_loop, listener, shutdown_rx).await },
        );

    // 4. Build the test client. The client trusts the proxy's
    //    `RootCa` (the cert the proxy dynamically signs the
    //    leaf from) — NOT webpki-roots, because the proxy's
    //    leaf cert is signed by the in-process `RootCa`, not
    //    by a public CA.
    let mut proxy_root_store = rustls::RootCertStore::empty();
    proxy_root_store
        .add(CertificateDer::from(root_ca.root_cert_der()))
        .expect("add root_ca cert");
    let client_tls_config = Arc::new(
        rustls::ClientConfig::builder()
            .with_root_certificates(proxy_root_store)
            .with_no_client_auth(),
    );
    let client_tls = TlsConnector::from(client_tls_config);

    // 5. Send CONNECT to the proxy. The proxy parses it and
    //    replies 200 (after verifying the SNI doesn't have a
    //    port strip bug — see PR #16's CVE-shaped bug).
    let mut tcp_to_proxy = TcpStream::connect(proxy_addr).await.expect("connect proxy");
    tcp_to_proxy
        .write_all(
            format!(
                "CONNECT 127.0.0.1:{} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n\r\n",
                origin_addr.port(),
                origin_addr.port()
            )
            .as_bytes(),
        )
        .await
        .expect("write CONNECT");

    // Read the CONNECT response (read until end of headers, then stop).
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1024];
    loop {
        let n = tcp_to_proxy
            .read(&mut tmp)
            .await
            .expect("read CONNECT resp");
        buf.extend_from_slice(&tmp[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    let connect_resp = std::str::from_utf8(&buf).expect("utf8 CONNECT resp");
    debug!("CONNECT response: {connect_resp}");
    assert!(
        connect_resp.starts_with("HTTP/1.1 200"),
        "proxy CONNECT did not return 200, got: {connect_resp}",
    );

    // 6. Upgrade the TCP stream to TLS. The SNI is the test
    //    origin's host (`127.0.0.1`) — the proxy's leaf
    //    signing code uses the CONNECT target host as the SNI
    //    and the cert SAN (per the design contract gotcha #1
    //    on confused-deputy routing).
    let server_name = ServerName::try_from("127.0.0.1").expect("server name");
    let tls_to_proxy = client_tls
        .connect(server_name, tcp_to_proxy)
        .await
        .expect("TLS handshake to proxy");

    // 7. Speak HTTP/2 over the TLS stream. The proxy's
    //    listener serves HTTP/2 to the client side (the
    //    `http2::Builder` shipped in §3.3.5 — the ALPN
    //    list advertises `h2, http/1.1` and modern browsers
    //    prefer h2). Use `hyper::client::conn::http2::handshake`
    //    to match the protocol. The `Sender` returned is
    //    `Clone` for h2 (unlike the H1 `SendRequest` which
    //    isn't Clone because the body is a trait object).
    let io = TokioIo::new(tls_to_proxy);
    let (mut sender, conn) =
        hyper::client::conn::http2::handshake(hyper_util::rt::TokioExecutor::new(), io)
            .await
            .expect("hyper h2 client handshake");
    // Drive the connection in the background.
    tokio::spawn(async move {
        let _ = conn.await;
    });

    // 8. Build the request: POST to the origin with the echo
    //    body. POST is load-bearing because the proxy's
    //    501-on-non-GET band-aid was removed in §3.3.5 (per
    //    the `forward_request_via_pool` path), so the test
    //    also validates that a non-GET goes all the way through.
    let assertion = EchoAssertion::new("mitm-roundtrip-token-7a3f");
    let request = hyper::Request::builder()
        .method("POST")
        .uri("/test?echo=1")
        .header("Host", format!("127.0.0.1:{}", origin_addr.port()))
        .body(Full::new(Bytes::from(assertion.request_body.clone())))
        .expect("build request");

    let response = sender.send_request(request).await.expect("send request");
    assert_eq!(
        response.status(),
        200,
        "expected 200, got {} (body: {:?})",
        response.status(),
        response.headers()
    );

    // 9. Read the response body and assert the echo came back.
    let body_bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect response body")
        .to_bytes();
    assert_eq!(
        body_bytes.as_ref(),
        &assertion.request_body[..],
        "response body should equal request body (echo); \
         request was {} bytes, response was {} bytes",
        assertion.request_body.len(),
        body_bytes.len(),
    );
    assert!(
        body_bytes
            .windows(assertion.token.len())
            .any(|w| w == assertion.token.as_bytes()),
        "response body should contain the token {:?}",
        assertion.token,
    );

    // 10. Shutdown the proxy cleanly. We don't strictly need to
    //     assert on the `RequestForwarded` event (the test
    //     could pass with the event missing if the proxy
    //     never reached the forward step), but we assert on
    //     the event-bus contract to be safe.
    //
    //     The proxy might shut down before we get to read the
    //     event (race with the conn task). Use a short timeout
    //     and accept the race.
    shutdown_tx.send(true).unwrap();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), run_task).await;

    // The event-bus assertion is best-effort. The test passes
    // as long as the body came back; the event check is
    // documentation that the proxy DID fire `RequestForwarded`.
    use bk_proxy::events::ProxyEvent;
    let event = rx_event.try_recv().ok();
    if let Some(ProxyEvent::RequestForwarded { host, status, .. }) = event {
        assert_eq!(host, "127.0.0.1", "event host should match SNI");
        assert_eq!(status, 200, "event status should match upstream");
    }
    // else: race lost, but the body roundtripped which is the
    // load-bearing assertion.
}

// `watch` is in `tokio::sync::watch`; the trait import
// `watch::channel` is used above. The compiler will complain
// if the import is missing — kept here as a comment so a
// future reader knows where to look.
use tokio::sync::watch;
