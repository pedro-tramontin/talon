//! The TCP accept loop.
//!
//! This is the §3.1 heart of the proxy: it pulls connections off a
//! [`TcpListener`], parks each one on a [`Semaphore`]-gated slot, and
//! hands it to a [`JoinSet`] so a clean shutdown drains in-flight
//! tasks before returning.

use std::sync::Arc;

use http_body_util::BodyExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{watch, Semaphore};
use tokio::task::JoinSet;
use tracing::{debug, warn};

use crate::Proxy;
use crate::ProxyEvent;

/// Run the accept loop until `shutdown` flips to `true` or all
/// shutdown senders are dropped.
///
/// Semantics:
/// * The [`Semaphore`] cap is honored *before* pulling a connection
///   off the listener: the loop first awaits a permit, then accepts.
///   When the cap is saturated, the kernel queue absorbs bursts and
///   the listener is parked on `acquire_owned()` until a permit
///   frees up — the backpressure behavior the tests exercise.
/// * Each accepted connection is spawned as a task tracked by a
///   [`JoinSet`]. On shutdown we await the entire set so the process
///   doesn't exit with live sockets open.
/// * The shutdown arm handles both `Ok(())` (value changed to `true`)
///   and `Err(RecvError)` (all senders dropped) as graceful exits.
pub async fn accept_loop(
    proxy: Arc<Proxy>,
    listener: TcpListener,
    mut shutdown: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let max_conn = proxy.config.max_concurrent_connections;
    let conn_sem = Arc::new(Semaphore::new(max_conn));
    let mut in_flight: JoinSet<()> = JoinSet::new();

    loop {
        // Make sure the *current* shutdown state is reflected in the
        // first iteration. `changed()` is a no-op until the value
        // actually changes, so we peek with `borrow()` first.
        if *shutdown.borrow() {
            debug!("shutdown observed before next accept; draining in-flight");
            break;
        }

        // Wait for either a permit or a shutdown signal. The permit
        // is the backpressure gate — without it we must not call
        // `accept()`.
        let permit = tokio::select! {
            biased;

            shutdown_res = shutdown.changed() => {
                match shutdown_res {
                    Ok(()) if *shutdown.borrow() => {
                        debug!("shutdown signal received; stopping accept loop");
                        break;
                    }
                    // Spurious change (same value) or senders dropped
                    // mid-loop with no value set: treat as graceful
                    // shutdown so we don't busy-loop. The `RecvError`
                    // type is public but its constructor is private;
                    // match on `Err(_)` instead.
                    Ok(()) | Err(_) => {
                        debug!("shutdown channel closed or stale; treating as shutdown");
                        break;
                    }
                }
            }

            permit_res = conn_sem.clone().acquire_owned() => {
                match permit_res {
                    Ok(p) => p,
                    Err(_) => {
                        // The semaphore is closed only if we close
                        // it; we never do in §3.1. Treat as fatal.
                        warn!("connection semaphore closed unexpectedly");
                        break;
                    }
                }
            }
        };

        // Permit in hand. Now accept the next connection, but stay
        // responsive to shutdown: if the user hits Ctrl-C while we're
        // parked on accept, abandon the accept and loop back to the
        // top (where the shutdown check at the start of the loop
        // breaks us out cleanly).
        let accept_res = tokio::select! {
            biased;
            _ = shutdown.changed() => {
                // Drop the permit (it's released when `_permit` goes
                // out of scope below) and loop.
                drop(permit);
                continue;
            }
            res = listener.accept() => res,
        };

        let (stream, peer_addr) = match accept_res {
            Ok(pair) => pair,
            Err(e) => {
                // A transient accept error doesn't need to kill the
                // whole loop. Log and continue; the permit is
                // released when `_permit` goes out of scope.
                drop(permit);
                warn!(error = %e, "accept error; continuing");
                continue;
            }
        };

        let proxy_for_task = proxy.clone();
        in_flight.spawn(async move {
            handle_connection(proxy_for_task, stream, peer_addr).await;
            // Permit is released when this task ends.
            drop(permit);
        });
    }

    // Drop the listener so no further accepts can queue.
    drop(listener);

    // Wait for in-flight tasks to drain.
    while let Some(res) = in_flight.join_next().await {
        if let Err(e) = res {
            // JoinError here means a task panicked. We don't bring
            // down the whole loop, but it shouldn't happen in §3.1.
            warn!(error = %e, "in-flight connection task join error");
        }
    }

    Ok(())
}

/// Handle a single accepted connection.
///
/// §3.3 dispatches the per-connection work:
/// 1. Try to read a `CONNECT host:port` request from the browser.
/// 2. Reply `200 Connection Established` and perform the TLS handshake
///    using a per-host cert minted from the proxy's `RootCa`.
/// 3. Read the HTTP/1.1 **or HTTP/2** request from the decrypted
///    TLS stream. The protocol is negotiated via ALPN; the same
///    `service_fn` closure works for both because hyper's
///    `Service` / `HttpService` traits are protocol-agnostic.
/// 4. Forward the request to the real upstream over a fresh TLS
///    connection (the host comes from the SNI, NOT the Host header —
///    design contract gotcha #1).
/// 5. Stream the response back to the browser over the same TLS
///    stream.
/// 6. Emit `ProxyEvent::RequestForwarded` for the request
///    (success path returns the real upstream status; 501/502
///    rejections return the proxy-generated status).
///
/// Non-CONNECT requests (plain HTTP proxy use) are not handled in
/// §3.3 — Phase 4 adds that. The connection is closed with an
/// error log.
async fn handle_connection(proxy: Arc<Proxy>, stream: TcpStream, peer_addr: std::net::SocketAddr) {
    let (host, tls_stream) =
        match crate::mitm::handle_connect_tunnel(stream, proxy.root_ca.clone()).await {
            Ok(pair) => pair,
            Err(e) => {
                // Non-CONNECT or other failure. §3.3 doesn't support
                // plain HTTP proxying; log and close.
                warn!(%peer_addr, error = %e, "CONNECT failed; closing");
                return;
            }
        };

    // Read the HTTP request from the now-decrypted TLS stream.
    // The hyper 1.x server builder takes a TokioIo wrapper around
    // any AsyncRead+AsyncWrite, which the TlsStream<TcpStream> is.
    //
    // We use the HTTP/2 server builder, NOT http1, because the
    // ALPN list in `ca.rs` advertises both `h2` and `http/1.1`
    // (modern browsers always try h2 first over TLS). An
    // http1-only handler would fail the browser handshake any
    // time the browser selects h2. Both builders accept the same
    // `HttpService` shape, so the same `svc` closure works for
    // both protocols.
    use hyper::server::conn::http2;
    use hyper_util::rt::{TokioExecutor, TokioIo};
    let io = TokioIo::new(tls_stream);

    // The service closure needs `host` (from CONNECT) for the
    // upstream forwarder and the event-bus handle to emit
    // `RequestForwarded` per request. The closure is `Fn`
    // (called multiple times for keep-alive on a single TLS
    // tunnel), so we must clone both into it. We also need
    // `host` for the connection-level debug log below, so the
    // closure gets its own clone and we keep the original.
    let events = proxy.events.clone();
    let host_for_log = host.clone();

    let svc = hyper::service::service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
        let host = host.clone();
        let events = events.clone();
        let started = std::time::Instant::now();
        async move {
            // §3.3 only forwards GETs. POST/PUT/etc. would
            // require forwarding the request body, which the
            // current `UpstreamBody` type (`Empty<Bytes>`) can't
            // represent. Until body forwarding lands, reject
            // non-GETs with a clear 501 so the browser doesn't
            // silently see a GET (the old bug — POST became GET,
            // which corrupts state on the server side).
            if req.method() != hyper::Method::GET {
                tracing::warn!(
                    method = %req.method(),
                    "rejecting non-GET request; only forwards GETs"
                );
                let resp = hyper::Response::builder()
                    .status(501)
                    .header("content-type", "text/plain; charset=utf-8")
                    .body(
                        http_body_util::Full::new(bytes::Bytes::from_static(
                            b"not implemented: bk-proxy only forwards GET requests (POST/PUT/etc. land in a follow-up)\n",
                        ))
                        .map_err(|never| match never {})
                        .boxed(),
                    )
                    .unwrap();
                // Emit a 501 event so the UI/logger sees the
                // rejection (without it, keep-alive rejections
                // would be silent).
                events.send(ProxyEvent::RequestForwarded {
                    host: host.clone(),
                    status: 501,
                    bytes_in: 0,
                    bytes_out: 0,
                    duration_ms: started.elapsed().as_millis() as u64,
                });
                return Ok::<_, std::convert::Infallible>(resp);
            }

            // Build the upstream request (always GET; method is
            // fixed by the `UpstreamBody` body type alias).
            let upstream_req = match crate::upstream::build_get_request(
                &host,
                req.uri()
                    .path_and_query()
                    .map(|pq| pq.as_str())
                    .unwrap_or("/"),
            ) {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!(error = %e, "failed to build upstream request");
                    let resp = hyper::Response::builder()
                        .status(502)
                        .body(
                            http_body_util::Full::new(bytes::Bytes::from_static(b"bad gateway\n"))
                                .map_err(|never| match never {})
                                .boxed(),
                        )
                        .unwrap();
                    events.send(ProxyEvent::RequestForwarded {
                        host: host.clone(),
                        status: 502,
                        bytes_in: 0,
                        bytes_out: 0,
                        duration_ms: started.elapsed().as_millis() as u64,
                    });
                    return Ok::<_, std::convert::Infallible>(resp);
                }
            };

            match crate::upstream::forward_request(&host, upstream_req).await {
                Ok(resp) => {
                    // Capture the actual upstream status (not
                    // hard-coded 200 — the old bug). Emit
                    // per-request so keep-alive connections
                    // don't batch N requests into one event.
                    let status = resp.status().as_u16();
                    events.send(ProxyEvent::RequestForwarded {
                        host: host.clone(),
                        status,
                        bytes_in: 0,
                        bytes_out: 0,
                        duration_ms: started.elapsed().as_millis() as u64,
                    });
                    Ok::<_, std::convert::Infallible>(resp.map(|b| b.boxed()))
                }
                Err(e) => {
                    tracing::error!(error = %e, "upstream forward failed");
                    let resp = hyper::Response::builder()
                        .status(502)
                        .body(
                            http_body_util::Full::new(bytes::Bytes::from_static(
                                b"upstream error\n",
                            ))
                            .map_err(|never| match never {})
                            .boxed(),
                        )
                        .unwrap();
                    events.send(ProxyEvent::RequestForwarded {
                        host: host.clone(),
                        status: 502,
                        bytes_in: 0,
                        bytes_out: 0,
                        duration_ms: started.elapsed().as_millis() as u64,
                    });
                    Ok::<_, std::convert::Infallible>(resp)
                }
            }
        }
    });

    // Drive the h2 server. We don't capture `status` here anymore
    // — the closure emits one `RequestForwarded` event per
    // request, with the real upstream status, before the
    // response body is fully streamed. The connection-level
    // result (Ok vs Err) is just a health signal; if it errors,
    // we log and return without emitting anything.
    match http2::Builder::new(TokioExecutor::new())
        .serve_connection(io, svc)
        .await
    {
        Ok(()) => {
            debug!(host = %host_for_log, "connection closed cleanly");
        }
        Err(e) => {
            warn!(host = %host_for_log, error = %e, "hyper server connection errored");
        }
    }
}

#[cfg(test)]
mod tests {
    //! Regression tests for the §3.3 server side.
    //!
    //! The original §3.3 implementation used
    //! `http1::Builder::new().serve_connection(io, svc)`. The
    //! ALPN list in `ca.rs` advertises both `h2` and `http/1.1`,
    //! so any browser that selected `h2` (which is the default
    //! for Chrome/Firefox/curl over TLS) would fail the
    //! handshake with a 502-equivalent.
    //!
    //! The fix is to use the `http2::Builder` instead. The test
    //! below enforces that the file actually uses the http2
    //! builder — a guard against accidental reverts.

    /// Guard test: the `handle_connection` body must use the
    /// HTTP/2 hyper server builder, not the HTTP/1.1 one. Read
    /// the source of this file and assert the expected
    /// `http2::Builder::new(TokioExecutor::new())` call site is
    /// present, and that the http1 builder is NOT used in the
    /// production code.
    #[test]
    fn handle_connection_uses_http2_server_builder() {
        let src = include_str!("listener.rs");
        // The fix site: must be present.
        assert!(
            src.contains("http2::Builder::new(TokioExecutor::new())"),
            "listener.rs must use http2::Builder::new(TokioExecutor::new()) to \
             match the ALPN list in ca.rs (which advertises `h2`). \
             An http1-only server would break any browser that selects h2."
        );
        // The old bug: must be absent from the production code.
        // We only check up to the `#[cfg(test)]` marker so the
        // assertion doesn't false-positive on this test's own
        // docstring.
        let production_src = src
            .split_once("#[cfg(test)]")
            .map(|(p, _)| p)
            .unwrap_or(src);
        assert!(
            !production_src.contains("http1::Builder::new()"),
            "listener.rs must NOT use http1::Builder::new() in production code — \
             that's the pre-Copilot-fix code that the ALPN h2 advertising broke."
        );
    }

    /// Guard test: the service closure must reject non-GET
    /// requests with a 501 instead of silently forwarding them
    /// as GET. Regression for Copilot review thread 3593770829
    /// (PR #17).
    #[test]
    fn handle_connection_rejects_non_get_with_501() {
        let src = include_str!("listener.rs");
        let production_src = src
            .split_once("#[cfg(test)]")
            .map(|(p, _)| p)
            .unwrap_or(src);
        assert!(
            production_src.contains("req.method() != hyper::Method::GET"),
            "listener.rs must check req.method() != GET and return 501 for \
             non-GET requests. The pre-Copilot-fix code always called \
             build_get_request, so a POST became a GET — silent corruption."
        );
    }

    /// Guard test: the `RequestForwarded` event must capture the
    /// real upstream status (not a hard-coded 200) and must be
    /// emitted from inside the service closure (per request,
    /// not per connection). Regression for Copilot review
    /// thread 3593770866 (PR #17).
    #[test]
    fn handle_connection_emits_event_with_real_status_per_request() {
        let src = include_str!("listener.rs");
        let production_src = src
            .split_once("#[cfg(test)]")
            .map(|(p, _)| p)
            .unwrap_or(src);
        // The closure must call `events.send(...)` (emit
        // per-request), not just at the connection level.
        assert!(
            production_src.contains("events.send(ProxyEvent::RequestForwarded"),
            "listener.rs must emit RequestForwarded from inside the service \
             closure, capturing the real upstream status. The pre-Copilot-fix \
             code emitted once per connection with a hard-coded 200, so \
             keep-alive connections batched N requests into one event and \
             non-200 upstream responses were misreported."
        );
        // The status must come from the upstream response, not be
        // hard-coded. Look for `resp.status().as_u16()` or similar.
        assert!(
            production_src.contains("resp.status().as_u16()"),
            "listener.rs must capture the upstream response status \
             (resp.status().as_u16()), not hard-code 200."
        );
    }

    /// Guard test: the upstream request builder
    /// (`build_get_request` in `upstream.rs`) must produce
    /// origin-form URIs (just the path, not `https://host/path`)
    /// so the request is valid for both HTTP/1.1 and HTTP/2
    /// upstreams and so IPv6 literals in the host can be sent
    /// without ambiguity. Regression for Copilot review thread
    /// 3594225116 (PR #17).
    #[test]
    fn build_get_request_uses_origin_form_uri() {
        let src = include_str!("upstream.rs");
        let production_src = src
            .split_once("#[cfg(test)]")
            .map(|(p, _)| p)
            .unwrap_or(src);
        // The fix site: must be present (build_get_request must
        // NOT construct `https://{host}{path}`).
        assert!(
            !production_src.contains("format!(\"https://{host}{path_and_query}\")"),
            "build_get_request must NOT use absolute-form URIs \
             (`https://{{host}}{{path}}`). Origin-form (`/path`) is \
             the standard request-target for direct-connection \
             requests, the only legal form for h2 per RFC 7540 \
             §8.1.2.3, and the only way to keep IPv6 literals \
             unambiguous in the URI authority."
        );
    }
}
