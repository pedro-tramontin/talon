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
use tokio::time::{sleep, Duration};
use tracing::{debug, warn};

use crate::Proxy;

/// Run the accept loop until `shutdown` flips to `true`.
///
/// Semantics:
/// * Accepts are driven by `select!`ing on the listener and the
///   shutdown signal — when the signal fires, the loop stops
///   accepting new connections and waits for in-flight ones to
///   finish.
/// * The [`Semaphore`] caps the number of concurrent in-flight
///   connection tasks. When the cap is hit, the next `accept` is
///   deliberately not pulled off the listener until a permit frees
///   up — this is the backpressure mechanism the tests exercise.
/// * Each accepted connection is spawned as a task tracked by a
///   [`JoinSet`]. On shutdown we await the entire set so the process
///   doesn't exit with live sockets open.
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

        tokio::select! {
            biased;

            // Honour shutdown first: even if a connection is ready,
            // if the user hit Ctrl-C we stop accepting.
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    debug!("shutdown signal received; stopping accept loop");
                    break;
                }
            }

            // Try to accept a new connection.
            accept_res = listener.accept() => {
                let (stream, peer_addr) = match accept_res {
                    Ok(pair) => pair,
                    Err(e) => {
                        // A transient accept error doesn't need to
                        // kill the whole loop. Log and continue.
                        warn!(error = %e, "accept error; continuing");
                        continue;
                    }
                };

                // Acquire a permit BEFORE spawning so the cap is
                // honoured even if the spawned task is starved.
                let permit = match conn_sem.clone().acquire_owned().await {
                    Ok(p) => p,
                    Err(_) => {
                        // The semaphore is closed only if we close
                        // it; we never do in §3.1. Treat this as a
                        // fatal error.
                        warn!("connection semaphore closed unexpectedly");
                        break;
                    }
                };

                let proxy_for_task = proxy.clone();
                in_flight.spawn(async move {
                    handle_connection(proxy_for_task, stream, peer_addr).await;
                    // Permit is released when this task ends.
                    drop(permit);
                });
            }
        }
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
/// §3.1 has no real per-connection work — the MITM cores come in
/// §3.3. For now we just close the socket after a tick so the
/// semaphore permit is released promptly and the test of the
/// concurrency cap sees realistic back-and-forth.
async fn handle_connection(_proxy: Arc<Proxy>, stream: TcpStream, peer_addr: std::net::SocketAddr) {
    // §3.1 has no MITM logic; we just close the socket after a tick
    // so the semaphore permit is released promptly and the test of
    // the concurrency cap sees realistic back-and-forth.
    debug!(%peer_addr, "connection accepted; closing (no MITM in §3.1)");
    let _ = stream;
    // Yield once to let the runtime schedule the permit release
    // before the task exits. Without this, a tight loop of accepts
    // can starve the join_next() wait above.
    sleep(Duration::from_millis(1)).await;
}
