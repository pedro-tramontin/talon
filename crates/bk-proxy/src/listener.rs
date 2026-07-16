//! The TCP accept loop.
//!
//! This is the §3.1 heart of the proxy: it pulls connections off a
//! [`TcpListener`], parks each one on a [`Semaphore`]-gated slot, and
//! hands it to a [`JoinSet`] so a clean shutdown drains in-flight
//! tasks before returning.

use std::sync::Arc;

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
/// 3. Read the HTTP/1.1 request from the decrypted TLS stream.
/// 4. Forward the request to the real upstream over a fresh TLS
///    connection (the host comes from the SNI, NOT the Host header —
///    design contract gotcha #1).
/// 5. Stream the response back to the browser over the same TLS
///    stream.
/// 6. Emit `ProxyEvent::RequestForwarded` on success.
///
/// Non-CONNECT requests (plain HTTP proxy use) are not handled in
/// §3.3 — Phase 4 adds that. The connection is closed with an
/// error log.
async fn handle_connection(proxy: Arc<Proxy>, stream: TcpStream, peer_addr: std::net::SocketAddr) {
    let started = std::time::Instant::now();
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

    // We need the host (from CONNECT) in scope for the upstream
    // forwarder, AND we need to extract the request from the
    // hyper service call. Use a `Rc<RefCell<Option<String>>>` or
    // just clone the host into the service closure. Host is
    // cheap to clone (small String).
    let host_for_service = host.clone();
    let host_for_event = host.clone();

    let svc = hyper::service::service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
        let host = host_for_service.clone();
        async move {
            // Convert the Incoming body to our UpstreamBody type.
            // §3.3 only handles GETs (no request body), so we use Empty.
            use http_body_util::BodyExt;
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
                    // Return a 502 with a uniform body type so the
                    // service closure's two arms agree on the body.
                    return Ok::<_, std::convert::Infallible>(
                        hyper::Response::builder()
                            .status(502)
                            .body(
                                http_body_util::Full::new(bytes::Bytes::from_static(
                                    b"bad gateway\n",
                                ))
                                .map_err(|never| match never {})
                                .boxed(),
                            )
                            .unwrap(),
                    );
                }
            };
            match crate::upstream::forward_request(&host, upstream_req).await {
                Ok(resp) => Ok::<_, std::convert::Infallible>(resp.map(|b| b.boxed())),
                Err(e) => {
                    tracing::error!(error = %e, "upstream forward failed");
                    Ok::<_, std::convert::Infallible>(
                        hyper::Response::builder()
                            .status(502)
                            .body(
                                http_body_util::Full::new(bytes::Bytes::from_static(
                                    b"upstream error\n",
                                ))
                                .map_err(|never| match never {})
                                .boxed(),
                            )
                            .unwrap(),
                    )
                }
            }
        }
    });

    // Track bytes in/out + final status. We capture status by
    // wrapping the body, but for §3.3 we approximate with
    // duration only — a future §3.5 will use a tap body wrapper.
    //
    // HTTP/2 server: `Builder::new()` takes an executor
    // (`TokioExecutor` is the standard choice for the tokio
    // runtime). The same `service_fn` closure used by the
    // http1 builder is accepted here because hyper's `Service`
    // / `HttpService` traits are protocol-agnostic.
    let status = match http2::Builder::new(TokioExecutor::new())
        .serve_connection(io, svc)
        .await
    {
        Ok(()) => 200, // connection completed; status was 200-ish from upstream
        Err(e) => {
            warn!(host = %host, error = %e, "hyper server connection errored");
            return;
        }
    };
    let duration_ms = started.elapsed().as_millis() as u64;

    // Emit the event. Bytes-in/out are best-effort 0 for §3.3;
    // §3.6 adds a body-tap wrapper that measures both.
    proxy.events.send(ProxyEvent::RequestForwarded {
        host: host_for_event,
        status,
        bytes_in: 0,
        bytes_out: 0,
        duration_ms,
    });

    debug!(host = %host, status, duration_ms, "request forwarded");
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
}
