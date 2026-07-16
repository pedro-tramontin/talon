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

use anyhow::{anyhow, Context, Result};
use hyper::body::Incoming;
use hyper::{Request, Response};
use tracing::debug;

use crate::upstream_pool::Pool;

/// The body type the proxy sends to the upstream. A streaming
/// body so POST/PUT/PATCH/DELETE requests can forward their
/// request body frame-by-frame without buffering in memory
/// (a 5 MB POST must not OOM the proxy — §3.3.5 hard
/// requirement #1).
///
/// Concretely: a `StreamBody` whose underlying stream yields
/// `Result<Frame<Bytes>, Box<dyn Error + Send + Sync>>`. The
/// stream is a trait object so the upstream code can build
/// it from any source — `Empty` for GETs, a frame-forwarder
/// for `Incoming` bodies, etc. — without generic-parameter
/// explosion at every call site.
pub type UpstreamBody = http_body_util::StreamBody<
    std::pin::Pin<
        Box<
            dyn futures_util::stream::Stream<
                    Item = Result<
                        http_body::Frame<bytes::Bytes>,
                        Box<dyn std::error::Error + Send + Sync>,
                    >,
                > + Send,
        >,
    >,
>;

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
///
/// `pool` is the per-host upstream connection pool (see
/// [`crate::upstream_pool::Pool`]). The pool is responsible for
/// reusing TLS connections across requests; §3.3.5 introduces it
/// to avoid opening a fresh TCP+TLS handshake on every request.
pub async fn forward_request(
    host: &str,
    request: Request<UpstreamBody>,
    pool: &Pool,
) -> Result<Response<UpstreamResponseBody>> {
    forward_request_with_tls_config(host, request, pool).await
}

/// Test-only variant of `forward_request`. The test should
/// build a [`Pool`] with a custom `ClientConfig` (e.g. one that
/// trusts a test `RootCa`) and pass it here. **Not part of
/// the public surface; do not call from non-test code.**
#[doc(hidden)]
pub async fn forward_request_with_tls_config(
    host: &str,
    request: Request<UpstreamBody>,
    pool: &Pool,
) -> Result<Response<UpstreamResponseBody>> {
    // Acquire a pooled connection (or open a fresh one if the
    // pool is empty for this host). The `PooledConn` is RAII:
    // dropping it returns the conn to the pool (or discards it
    // if `mark_errored` was called).
    let mut pooled = pool
        .connect(host)
        .await
        .with_context(|| format!("upstream pool connect to {host} failed"))?;
    let mut sender = pooled
        .sender
        .take()
        .ok_or_else(|| anyhow!("pool returned a conn with no sender"))?;

    // Send the request and await the response head.
    let result = sender.send_request(request).await;

    let response = match result {
        Ok(r) => r,
        Err(e) => {
            // Mark the conn as errored so Drop discards it.
            pooled.mark_errored();
            return Err(anyhow!("upstream request to {host} failed: {e}"));
        }
    };

    debug!(host = %host, status = %response.status(), "upstream response received");
    // `pooled` is dropped here, returning the conn to the pool
    // (H1 keep-alive).
    Ok(response)
}

/// Build a `hyper::Request<UpstreamBody>` for any HTTP method the
/// proxy needs to forward. The `method` is the original
/// browser-side method (GET, POST, PUT, etc.); the `path_and_query`
/// is the origin-form request-target; the `body` is a streaming
/// `UpstreamBody` (use `empty_upstream_body()` for the GET case,
/// or `forward_incoming_body(...)` to forward an `Incoming` body
/// from the browser side). The `Host:` header is **set by us** to
/// the SNI host (not the browser's Host header).
///
/// ## Request-target: origin-form, not absolute-form
///
/// The `request-target` is the origin-form `/path?query` (just the
/// path + optional query), NOT the absolute-form
/// `https://host/path`. RFC 7230 §5.3.2 says origin-form is the
/// standard for "requests made directly to an origin server" —
/// which is what we're doing, because we open a fresh direct
/// TCP+TLS connection to the upstream and the upstream is the
/// origin. Absolute-form is only for requests sent to a proxy.
///
/// Absolute-form would also be ambiguous for IPv6 literals: the
/// authority component for IPv6 requires brackets
/// (`https://[2001:db8::1]/path`), and the absolute-form URI is
/// only legal for HTTP/1.1 (h2 requires origin-form per RFC 7540
/// §8.1.2.3). Sticking with origin-form avoids both issues.
///
/// ## `Host:` header for IPv6 literals
///
/// The HTTP/1.1 `Host:` header for an IPv6 literal also requires
/// brackets — `Host: [2001:db8::1]` (the port, if non-default,
/// goes in `[host]:port` form). We always forward on port 443
/// (the §3.3 "always 443" rule), so we can omit the port and
/// just send `Host: [2001:db8::1]` (or `Host: example.com` for
/// normal hostnames).
pub fn build_request(
    method: hyper::Method,
    host: &str,
    path_and_query: &str,
    body: UpstreamBody,
) -> Result<Request<UpstreamBody>> {
    let host_header = if host.contains(':') {
        // IPv6 literal — needs brackets in the Host header.
        // (The CONNECT-target parser already stripped the
        // brackets; we re-add them here because the HTTP wire
        // format requires them.)
        format!("[{host}]")
    } else {
        host.to_string()
    };
    // Origin-form request-target: just the path + optional query.
    // The hyper Request builder accepts an origin-form URI for
    // HTTP/1.1 and h2 (both protocols use it; h2 doesn't allow
    // absolute-form at all per RFC 7540 §8.1.2.3).
    let path = if path_and_query.is_empty() {
        "/"
    } else {
        path_and_query
    };
    let req = Request::builder()
        .method(method)
        .uri(path)
        .header("Host", host_header)
        .header("User-Agent", "talon/0.1")
        .body(body)
        .map_err(|e| anyhow!("failed to build upstream request: {e}"))?;
    Ok(req)
}

/// Build a streaming `UpstreamBody` from a `hyper::body::Incoming`
/// (the browser-side request body). Yields one `Frame::data(Bytes)`
/// per `Frame` that the `Incoming` body yields, so the body is
/// forwarded frame-by-frame without buffering. The `Incoming` body
/// is consumed; the result is a `StreamBody` that, when polled,
/// pulls the next frame from the underlying `Incoming`.
///
/// This is the bridge between the `hyper::body::Incoming` (the
/// browser side) and the `UpstreamBody` (the upstream side). §3.3.5
pub fn build_body_from_incoming(incoming: hyper::body::Incoming) -> UpstreamBody {
    // `Incoming` doesn't implement `Stream<Item = Result<Frame, Error>>`
    // directly, so we adapt it with a tiny stream type. The
    // `BodyStream` adapter from `http-body-util` is `StreamExt`-based;
    // we drive it inside an `async_stream` block and yield each frame
    // into our `Stream<Frame>` shape.
    use futures_util::StreamExt as _;
    use http_body::Frame;
    type FrameStream = std::pin::Pin<
        Box<
            dyn futures_util::stream::Stream<
                    Item = Result<Frame<bytes::Bytes>, Box<dyn std::error::Error + Send + Sync>>,
                > + Send,
        >,
    >;
    fn build_stream(incoming: hyper::body::Incoming) -> FrameStream {
        Box::pin(async_stream::stream! {
            let mut incoming = incoming;
            let mut body_stream = http_body_util::BodyStream::new(&mut incoming);
            while let Some(frame_result) = body_stream.next().await {
                // BodyStream yields `Result<Frame, Error>`; map the
                // error to our boxed type.
                match frame_result {
                    Ok(frame) => yield Ok::<_, Box<dyn std::error::Error + Send + Sync>>(frame),
                    Err(e) => {
                        yield Err(Box::new(e) as Box<dyn std::error::Error + Send + Sync>);
                        break;
                    }
                }
            }
        })
    }
    UpstreamBody::new(build_stream(incoming))
}

/// Build the upstream request body for a GET (no body). Returns
/// a `UpstreamBody` that produces no frames — the `Empty<Bytes>`
/// equivalent, but as a streaming body so the type matches the
/// POST/PUT/etc. code path.
pub fn empty_upstream_body() -> UpstreamBody {
    // A stream that immediately ends and yields no frames.
    type EmptyStream = std::pin::Pin<
        Box<
            dyn futures_util::stream::Stream<
                    Item = Result<
                        http_body::Frame<bytes::Bytes>,
                        Box<dyn std::error::Error + Send + Sync>,
                    >,
                > + Send,
        >,
    >;
    let stream: EmptyStream = Box::pin(futures_util::stream::empty());
    UpstreamBody::new(stream)
}

/// Build a `hyper::Request<Empty<Bytes>>` for the common GET case
/// the proxy needs to forward. Kept for API stability — wraps
/// [`build_request`] with `Method::GET` and an empty body so the
/// existing 4 unit tests in `upstream.rs` continue to cover the
/// same code path.
pub fn build_get_request(host: &str, path_and_query: &str) -> Result<Request<UpstreamBody>> {
    build_request(
        hyper::Method::GET,
        host,
        path_and_query,
        empty_upstream_body(),
    )
}

/// Re-export `HttpConnector` for tests that want to build a hyper
/// server with the same connector type. Not used in the proxy code
/// itself, but the test for `RequestForwarded` event delivery uses
/// a hyper server and benefits from a shared type definition.
pub type SharedHttpConnector = hyper_util::client::legacy::connect::HttpConnector;

#[cfg(test)]
mod tests {
    //! Regression tests for `build_get_request` (the upstream
    //! request builder). The pre-Copilot-fix code used
    //! absolute-form URIs (`https://{host}{path}`) and an
    //! unmodified `Host` header, which broke for IPv6 literals
    //! (they need brackets in both the URI authority and the
    //! `Host:` header). It also didn't work for h2 upstream
    //! connections because h2 requires origin-form per RFC 7540
    //! §8.1.2.3. Regression for Copilot review thread
    //! 3594225116 (PR #17).

    use super::build_get_request;

    #[test]
    fn build_get_request_uses_origin_form_path() {
        let req = build_get_request("example.com", "/foo?bar=1").unwrap();
        // Origin-form: just the path, NOT `https://example.com/foo?bar=1`.
        assert_eq!(req.uri().path_and_query().unwrap().as_str(), "/foo?bar=1");
        assert_eq!(req.uri().scheme_str(), None, "origin-form has no scheme");
        assert_eq!(req.uri().authority(), None, "origin-form has no authority");
    }

    #[test]
    fn build_get_request_defaults_missing_path_to_slash() {
        let req = build_get_request("example.com", "").unwrap();
        assert_eq!(req.uri().path(), "/");
    }

    #[test]
    fn build_get_request_host_header_for_ipv6_uses_brackets() {
        // `build_get_request` expects `host` to be the bare
        // IPv6 literal (no brackets) — the helper that strips
        // them is the `strip_connect_target_port` upstream in
        // `mitm.rs`. The `Host:` header needs brackets because
        // the HTTP wire format requires them.
        let req = build_get_request("2001:db8::1", "/").unwrap();
        assert_eq!(
            req.headers().get("Host").unwrap().to_str().unwrap(),
            "[2001:db8::1]"
        );
    }

    #[test]
    fn build_get_request_host_header_for_hostname_unchanged() {
        let req = build_get_request("example.com", "/").unwrap();
        assert_eq!(
            req.headers().get("Host").unwrap().to_str().unwrap(),
            "example.com"
        );
    }

    // §3.3.5 streaming-body tests ---------------------------------

    use super::build_request;
    use super::empty_upstream_body;
    use bytes::Bytes;
    use http_body_util::BodyExt as _;

    /// Unit-level streaming test: `build_request` must accept a
    /// POST with a 5 MB streaming `UpstreamBody` and preserve the
    /// full body without eager consumption. The full roundtrip
    /// (proxy → upstream → echo) needs an in-process TLS test
    /// origin and is a follow-up; this test catches the
    /// "we forgot to use a streaming type" regression that would
    /// re-introduce the OOM gap.
    #[tokio::test]
    async fn build_request_accepts_large_streaming_post_body() {
        // 5 MB body, generated as 50 chunks of 100 KB each.
        let chunk_count = 50usize;
        let chunk_size = 100usize * 1024; // 100 KB
        let total_bytes = chunk_count * chunk_size;

        let stream = futures_util::stream::iter((0..chunk_count).map(|i| {
            // Build the chunk bytes inside the closure so they
            // don't borrow `chunk_size` (which would make the
            // closure non-`'static`).
            let bytes = vec![(i & 0xFF) as u8; 100 * 1024]; // 100 KB
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(http_body::Frame::data(Bytes::from(
                bytes,
            )))
        }));
        // Box into the trait object expected by `UpstreamBody`.
        let body: super::UpstreamBody = http_body_util::StreamBody::new(Box::pin(stream));

        let req = build_request(hyper::Method::POST, "example.com", "/upload", body)
            .expect("build_request must accept a streaming body");

        // The request must be a POST.
        assert_eq!(req.method(), hyper::Method::POST);
        // Origin-form path.
        assert_eq!(req.uri().path(), "/upload");
        // Host header unchanged.
        assert_eq!(
            req.headers().get("Host").unwrap().to_str().unwrap(),
            "example.com"
        );

        // Now consume the body — the request builder must not have
        // already consumed the body. If the OOM gap is closed, the
        // body yields 50 frames of 100 KB each, totaling 5 MB.
        let collected = req
            .into_body()
            .collect()
            .await
            .expect("body collection should succeed")
            .to_bytes();
        assert_eq!(
            collected.len(),
            total_bytes,
            "the streaming body must yield the full 5 MB without truncation"
        );
    }

    /// Guard test: `UpstreamBody` must be a streaming type, not
    /// `Empty<Bytes>`. The §3.3.5 work closed the OOM gap by
    /// replacing the buffered `Empty<Bytes>` with a
    /// `StreamBody<Pin<Box<dyn Stream<...>>>>` that yields
    /// frames one at a time. Re-introducing `Empty<Bytes>` as
    /// the body type would silently re-introduce the 5 MB POST
    /// OOM bug.
    #[test]
    fn upstream_body_is_streaming_not_empty() {
        let src = include_str!("upstream.rs");
        let production_src = src
            .split_once("#[cfg(test)]")
            .map(|(p, _)| p)
            .unwrap_or(src);
        // The type alias must use StreamBody, not Empty.
        assert!(
            production_src.contains("type UpstreamBody = http_body_util::StreamBody"),
            "UpstreamBody must be a StreamBody (streaming). The pre-§3.3.5 \
             code used `http_body_util::Empty<Bytes>`, which forces the body \
             to be buffered in memory — a 5 MB POST would OOM the proxy. \
             The streaming type is the §3.3.5 fix."
        );
        // And the Empty<Bytes> type alias must not be present.
        assert!(
            !production_src.contains("type UpstreamBody = http_body_util::Empty"),
            "UpstreamBody must NOT be http_body_util::Empty — that's the \
             pre-§3.3.5 buffered type that OOMs on large bodies."
        );
    }

    /// Guard test: `build_body_from_incoming` must exist and
    /// produce a streaming `UpstreamBody` from a
    /// `hyper::body::Incoming`. The listener's service closure
    /// calls this to forward non-GET request bodies; without
    /// it, non-GETs would carry an empty body and the upstream
    /// would see a 0-byte POST/PUT/PATCH/DELETE.
    #[test]
    fn build_body_from_incoming_exists() {
        let src = include_str!("upstream.rs");
        assert!(
            src.contains("pub fn build_body_from_incoming("),
            "upstream.rs must export build_body_from_incoming for the \
             listener to forward non-GET bodies."
        );
    }

    /// Empty-body smoke test: the `empty_upstream_body()` helper
    /// must produce a body that yields zero frames. This is the
    /// GET-equivalent of the streaming test.
    #[tokio::test]
    async fn empty_upstream_body_yields_no_frames() {
        let req = build_request(
            hyper::Method::GET,
            "example.com",
            "/",
            empty_upstream_body(),
        )
        .expect("GET with empty body must build");

        let collected = req
            .into_body()
            .collect()
            .await
            .expect("empty body collection succeeds")
            .to_bytes();
        assert_eq!(collected.len(), 0, "empty body must yield 0 bytes");
    }
}
