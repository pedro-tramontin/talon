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
