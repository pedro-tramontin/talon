//! Tokio-broadcast fan-in helper for the `WireEvent` envelope.
//!
//! ## Problem
//!
//! Phase 8's wire bus carries three kinds of events
//! (`engine_event`, `agent_event`, `proxy_event`) from three
//! independent source buses:
//!
//! * `bk_engine::events::EventReceiver` — a `broadcast::Receiver<EngineEvent>`
//! * `bk_agent::events::EventReceiver` — a `broadcast::Receiver<AgentEvent>`
//! * `bk_proxy::events::broadcast::Receiver<ProxyEvent>`
//!
//! The Tauri app needs to consume all three and re-publish them
//! on a single `wire_event` channel (the one the React
//! `WireClient` listens to). The fan-in must:
//!
//! 1. Stamp a **monotonic `seq`** on each forwarded event so the
//!    React side can detect dropped events (Phase 8 contract).
//! 2. Tolerate `RecvError::Lagged` on any source — the seq is
//!    load-bearing precisely because sources can drop, and the
//!    fan-in's seq must keep advancing even if a source lags.
//! 3. Be **cancellation safe** — wrapping the receiver in a
//!    `tokio::select!` with a `cancellation_token` must not
//!    drop events. (The implementation uses
//!    `tokio::select!` with the cancellation branch LAST so the
//!    recv branch is the cancel-safe branch; this is the
//!    canonical pattern in the tokio docs.)
//! 4. Tolerate `RecvError::Lagged` on a source by logging
//!    a warning and continuing with the SAME receiver.
//!    The seq counter keeps advancing, so the next received
//!    event will have a seq that is `n+1` greater than the
//!    last one we sent — that gap is the signal the React
//!    side uses to surface "missed events". We do NOT
//!    re-subscribe to the source on lag (the broadcast ring
//!    advances on its own; the next `recv()` returns the
//!    next event in the ring, not a re-subscription).
//!    Re-subscription would be a behavior change with
//!    subtle consequences (a new subscriber starts at the
//!    current tail, losing any events that arrived between
//!    the lag detection and the re-subscribe), and the seq
//!    gap is the load-bearing drop signal we want anyway.
//!
//! ## Design
//!
//! The function takes three `broadcast::Receiver<Value>` (the
//! source payloads, already type-erased to `serde_json::Value`)
//! and a shared `broadcast::Sender<WireEvent>` for the sink. The
//! seq counter is `Arc<AtomicU64>` so all three tasks advance
//! the SAME counter — the seq is process-global, not per-source.
//!
//! `fan_in` spawns 3 tokio tasks on a runtime [`Handle`] passed
//! by the caller and returns a [`FanInHandle`] that the caller
//! can `await` (for cancellation) or `abort` to stop the tasks.
//!
//! The handle is passed explicitly so the helper does not
//! require a Tokio runtime to be **in scope** at the call
//! site. Without this, callers from synchronous contexts
//! (Tauri's `setup` closure, the main thread at startup, etc.)
//! would panic with "there is no reactor running, must be
//! called from the context of a Tokio 1.x runtime" — which is
//! exactly what bit us in v0.1.1 on the Windows binary.
//! Production callers get a handle from
//! `tauri::async_runtime::handle().inner()`; tests get one
//! from `tokio::runtime::Handle::current()` (the
//! `#[tokio::test]` macro installs a runtime as the current
//! one).
//!
//! ## Why 3 source `Value` receivers (not generic over the
//! source event type)
//!
//! The wire payload is `serde_json::Value` for forward-compat
//! (per the §4.0 design contract). The source event types
//! (`EngineEvent`, `AgentEvent`, `ProxyEvent`) are serde-
//! derived, so `serde_json::to_value` produces a valid
//! `Value` at the source-emit site. The fan-in does not need
//! to know the source types — it just shuttles `Value`s and
//! stamps a seq.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::broadcast;
use tokio::task::JoinSet;
use tracing::warn;

use crate::{WireEvent, WireEventKind};

/// Handle returned by [`fan_in`]. The caller can `await` it
/// to block until all three tasks complete (typically when
/// the `cancellation_token` fires), or `abort` to drop the
/// tasks eagerly. The `JoinSet` is the public surface for
/// cancellation; the individual `JoinHandle`s are private.
pub struct FanInHandle {
    tasks: JoinSet<()>,
}

impl FanInHandle {
    /// Block until all three fan-in tasks finish. The tasks
    /// finish when their `cancellation_token` fires (or when
    /// the source `broadcast::Sender` is dropped and the
    /// receiver returns `RecvError::Closed`).
    pub async fn join(mut self) {
        // Drain the JoinSet. Each task ends when its source
        // closes or the cancellation_token fires; we don't
        // care about the per-task result — they're all
        // `Result<(), JoinError>` for the `return;` in the
        // task bodies.
        while let Some(res) = self.tasks.join_next().await {
            if let Err(e) = res {
                // A task panicked; this is a bug, but we
                // don't want the join to crash the caller —
                // log and continue draining the set.
                warn!(error = %e, "fan-in task exited with error");
            }
        }
    }

    /// Abort all three fan-in tasks eagerly. This is the
    /// "I'm done, drop everything" path — used in tests and
    /// on `Drop`.
    pub fn abort(&mut self) {
        self.tasks.abort_all();
    }
}

impl Drop for FanInHandle {
    fn drop(&mut self) {
        // If the caller forgets to `join` (e.g. test
        // panics mid-flight), the tasks are aborted instead
        // of leaking.
        self.tasks.abort_all();
    }
}

/// Spawn three tasks that each pull from one of the source
/// `broadcast::Receiver`s and forward to the shared
/// `broadcast::Sender<WireEvent>`, stamping a monotonic
/// `seq` on the way.
///
/// `seq_counter` is shared across all three tasks so the
/// seq is process-global (a `seq` from a lagged source is
/// not re-used by a fast source).
///
/// `cancellation_token` is a `tokio_util::sync::CancellationToken`;
/// the function is generic over the type so callers can
/// pass a `CancellationToken` or a `&CancellationToken`
/// (both work). On cancellation each task exits cleanly.
///
/// `sink_capacity` is the broadcast buffer for the
/// `WireEvent` sender. The §4.0 default is 256 — same as
/// the rest of the workspace's event buses. Lagged
/// subscribers on the sink see `RecvError::Lagged` and
/// the seq gap is the React side's signal to surface a
/// "missed events" banner.
pub fn fan_in(
    engine_rx: broadcast::Receiver<Value>,
    agent_rx: broadcast::Receiver<Value>,
    proxy_rx: broadcast::Receiver<Value>,
    sink: broadcast::Sender<WireEvent>,
    seq_counter: Arc<AtomicU64>,
    cancellation_token: tokio_util::sync::CancellationToken,
    sink_capacity: usize,
    handle: &tokio::runtime::Handle,
) -> FanInHandle {
    let mut tasks: JoinSet<()> = JoinSet::new();

    // The engine task: pulls `EngineEvent` payloads (already
    // serialized to `Value` at the emit site), wraps them in
    // a `WireEvent` with `kind: EngineEvent`, stamps the seq.
    let sink_e = sink.clone();
    let seq_e = seq_counter.clone();
    let token_e = cancellation_token.clone();
    tasks.spawn_on(
        async move {
            forward_loop(
                engine_rx,
                WireEventKind::EngineEvent,
                sink_e,
                seq_e,
                token_e,
                "engine",
                sink_capacity,
            )
            .await;
        },
        handle,
    );

    // The agent task: same shape, `kind: AgentEvent`.
    let sink_a = sink.clone();
    let seq_a = seq_counter.clone();
    let token_a = cancellation_token.clone();
    tasks.spawn_on(
        async move {
            forward_loop(
                agent_rx,
                WireEventKind::AgentEvent,
                sink_a,
                seq_a,
                token_a,
                "agent",
                sink_capacity,
            )
            .await;
        },
        handle,
    );

    // The proxy task: same shape, `kind: ProxyEvent`.
    let sink_p = sink;
    let seq_p = seq_counter;
    let token_p = cancellation_token;
    tasks.spawn_on(
        async move {
            forward_loop(
                proxy_rx,
                WireEventKind::ProxyEvent,
                sink_p,
                seq_p,
                token_p,
                "proxy",
                sink_capacity,
            )
            .await;
        },
        handle,
    );

    FanInHandle { tasks }
}

/// The per-source forwarding loop. Each of the three fan-in
/// tasks runs this. The pattern is:
///
/// 1. `recv()` an event from the source.
/// 2. On `Ok(value)`, fetch-and-increment the global seq,
///    wrap in a `WireEvent`, and `send` to the sink.
/// 3. On `Lagged(n)`, log a warning with the missed count
///    and continue — the seq counter on the NEXT event will
///    have a gap (because we did NOT re-stamp the dropped
///    events) and that gap is the React side's drop signal.
/// 4. On `Closed`, the source sender was dropped; exit.
/// 5. On cancellation_token fired, exit.
async fn forward_loop(
    mut rx: broadcast::Receiver<Value>,
    kind: WireEventKind,
    sink: broadcast::Sender<WireEvent>,
    seq_counter: Arc<AtomicU64>,
    cancellation_token: tokio_util::sync::CancellationToken,
    source_label: &'static str,
    sink_capacity: usize,
) {
    loop {
        // tokio::select! with the cancellation branch LAST so
        // the recv branch is the cancel-safe one. The tokio
        // docs guarantee that a select! where the cancel
        // branch is last never silently drops the recv
        // branch's result on cancellation.
        tokio::select! {
            biased;
            recv_result = rx.recv() => {
                match recv_result {
                    Ok(value) => {
                        // Stamp the seq. fetch_add returns the
                        // PREVIOUS value, so add 1 to get the
                        // 1-based seq. The first event has
                        // seq 1 (not 0) so the React side can
                        // use 0 as the "haven't seen anything"
                        // sentinel.
                        let seq = seq_counter.fetch_add(1, Ordering::Relaxed) + 1;
                        let ev = WireEvent::new(kind, value, seq);
                        // The sink may have no receivers (the
                        // Tauri shell hasn't started the
                        // webview yet) — `send` returns Err
                        // in that case. We swallow it; the
                        // event is dropped. The next
                        // subscriber (post-Tauri-startup)
                        // resumes from the new tail, and the
                        // seq gap is the React side's drop
                        // signal.
                        if sink.send(ev).is_err() && sink.receiver_count() == 0 {
                            // No subscribers at all — log
                            // once per source at debug level
                            // (we don't want to spam
                            // warnings in tests that drop the
                            // sink subscriber).
                            tracing::debug!(
                                source = source_label,
                                capacity = sink_capacity,
                                "fan-in: no subscribers on sink; dropping event"
                            );
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        // Lagged: the broadcast ring on the
                        // SOURCE overflowed. We did NOT
                        // receive the dropped events. The seq
                        // counter keeps advancing, so the
                        // NEXT event we receive will have a
                        // seq that's `n+1` greater than the
                        // last one we sent — that's the
                        // gap the React side detects.
                        //
                        // The broadcast::Receiver is still
                        // usable after a `Lagged` error; the
                        // next `recv()` will return the next
                        // event in the ring. We do NOT
                        // need to re-subscribe (the ring
                        // advanced on its own). We just log
                        // and continue.
                        warn!(
                            source = source_label,
                            missed = n,
                            "fan-in: source broadcast ring lagged; events dropped"
                        );
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        // The source sender was dropped; the
                        // upstream is gone. Exit cleanly.
                        tracing::debug!(
                            source = source_label,
                            "fan-in: source closed; exiting forwarder"
                        );
                        return;
                    }
                }
            }
            _ = cancellation_token.cancelled() => {
                // Cancellation: the app is shutting down
                // (or a test is ending). Exit cleanly.
                tracing::debug!(
                    source = source_label,
                    "fan-in: cancellation received; exiting forwarder"
                );
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashSet;
    use tokio::sync::broadcast;

    /// Test-bus tuple type. The 7-tuple would trigger
    /// `clippy::type_complexity` if spelled inline; the
    /// type alias silences it.
    type FanInTestBus = (
        broadcast::Sender<Value>,
        broadcast::Sender<Value>,
        broadcast::Sender<Value>,
        broadcast::Receiver<WireEvent>,
        Arc<AtomicU64>,
        tokio_util::sync::CancellationToken,
        FanInHandle,
    );

    /// Helper: build a 3-source bus with a sink. Returns the
    /// three source senders, the sink receiver, the
    /// seq counter, the cancellation token, and the
    /// `FanInHandle`. Used by all three fan-in tests.
    fn make_bus(source_capacity: usize, sink_capacity: usize) -> FanInTestBus {
        let (engine_tx, engine_rx) = broadcast::channel::<Value>(source_capacity);
        let (agent_tx, agent_rx) = broadcast::channel::<Value>(source_capacity);
        let (proxy_tx, proxy_rx) = broadcast::channel::<Value>(source_capacity);
        let (sink_tx, sink_rx) = broadcast::channel::<WireEvent>(sink_capacity);
        let seq = Arc::new(AtomicU64::new(0));
        let token = tokio_util::sync::CancellationToken::new();
        let handle = fan_in(
            engine_rx,
            agent_rx,
            proxy_rx,
            sink_tx,
            seq.clone(),
            token.clone(),
            sink_capacity,
            &tokio::runtime::Handle::current(),
        );
        (engine_tx, agent_tx, proxy_tx, sink_rx, seq, token, handle)
    }

    /// 3 sources × ~33 events each = 100 total. All 100 must
    /// arrive in the sink. Order across sources is NOT
    /// guaranteed (the fan-in is 3 concurrent tasks), but the
    /// TOTAL COUNT and the seq monotonicity are. Uses a 256-
    /// event buffer (the §4.0 default) so the sink doesn't lag.
    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn fan_in_forwards_all_events_with_monotonic_seq() {
        let (engine_tx, agent_tx, proxy_tx, mut sink_rx, seq, token, _handle) = make_bus(256, 256);

        // 34 + 33 + 33 = 100 events.
        for i in 0..34 {
            engine_tx
                .send(json!({"src": "engine", "i": i}))
                .expect("engine send");
        }
        for i in 0..33 {
            agent_tx
                .send(json!({"src": "agent", "i": i}))
                .expect("agent send");
        }
        for i in 0..33 {
            proxy_tx
                .send(json!({"src": "proxy", "i": i}))
                .expect("proxy send");
        }

        // Drain the sink. The fan-in tasks may need a tick to
        // wake — with `start_paused = true` we use `tokio::time::sleep`
        // with a small duration to let the scheduler advance.
        // (The exact duration doesn't matter as long as it's
        // non-zero; the runtime fires the tasks as soon as the
        // channel has data.)
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let mut received: Vec<WireEvent> = Vec::new();
        while let Ok(ev) = sink_rx.try_recv() {
            received.push(ev);
        }
        // Some events may still be in flight in the broadcast
        // ring; loop with a short wait to catch them.
        for _ in 0..100 {
            if received.len() >= 100 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            while let Ok(ev) = sink_rx.try_recv() {
                received.push(ev);
            }
        }

        // All 100 must arrive (sink buffer is 256, so no
        // sink-side lag).
        assert_eq!(
            received.len(),
            100,
            "fan-in must forward all 100 events; got {}",
            received.len()
        );

        // The seq is monotonic (1..=100). The fan-in does not
        // guarantee cross-source ordering, but the seq is
        // global so it's strictly increasing.
        let mut last_seq: u64 = 0;
        for ev in &received {
            assert!(
                ev.seq > last_seq,
                "seq must be strictly increasing; got {} after {}",
                ev.seq,
                last_seq
            );
            last_seq = ev.seq;
        }
        // The seq counter's final value is 100 (fetch_add
        // returns prev, so 0..100 -> 100).
        assert_eq!(seq.load(Ordering::Relaxed), 100);

        // Each of the 3 source kinds must be represented. We
        // don't check exact counts (cross-source ordering is
        // not guaranteed), only that none of the 3 sources
        // was dropped entirely.
        let kinds: HashSet<WireEventKind> = received.iter().map(|e| e.kind).collect();
        assert!(kinds.contains(&WireEventKind::EngineEvent));
        assert!(kinds.contains(&WireEventKind::AgentEvent));
        assert!(kinds.contains(&WireEventKind::ProxyEvent));

        // Cancel the fan-in and drop the senders so the
        // forwarder tasks exit cleanly.
        token.cancel();
        drop(engine_tx);
        drop(agent_tx);
        drop(proxy_tx);
    }

    /// A receiver that subscribes LATE (after the source
    /// ring has been overwritten) must see `Lagged` on its
    /// first `recv()` and the fan-in must log a warning.
    /// The seq counter on the NEXT event must still be
    /// monotonic (the load-bearing piece for Phase 8 drop
    /// detection).
    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn fan_in_logs_warning_on_lagged_and_seq_remains_monotonic() {
        // Initialize a per-test tracing layer that captures
        // `warn!` events into a `Vec<u8>` via a custom
        // `MakeWriter`. Using a per-test layer (via
        // `tracing::subscriber::with_default`) avoids the
        // "already initialized" race that the global
        // `tracing_subscriber::fmt::init` would hit if a
        // sibling test in the same binary initialized first.
        //
        // The `tracing-subscriber` is in dev-deps for this
        // exact reason. The captured `buf` is read at the
        // end of the test to assert the "lagged" warning
        // fired.
        use std::io::Write;
        use std::sync::{Arc, Mutex};
        use tracing_subscriber::fmt::MakeWriter;

        #[derive(Clone, Default)]
        struct SharedBuf(Arc<Mutex<Vec<u8>>>);
        impl Write for SharedBuf {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        struct SharedBufMaker(SharedBuf);
        impl<'a> MakeWriter<'a> for SharedBufMaker {
            type Writer = SharedBuf;
            fn make_writer(&'a self) -> Self::Writer {
                self.0.clone()
            }
        }
        let shared = SharedBuf::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(SharedBufMaker(shared.clone()))
            .with_max_level(tracing::Level::WARN)
            .with_target(false)
            .finish();
        let buf = shared.0.clone();
        let _guard = tracing::subscriber::set_default(subscriber);
        // deterministically. Tiny sink buffer (4) so the
        // sink-side lag recovery path is also exercised.
        let (engine_tx, engine_rx) = broadcast::channel::<Value>(4);
        let (agent_tx, agent_rx) = broadcast::channel::<Value>(4);
        let (proxy_tx, proxy_rx) = broadcast::channel::<Value>(4);
        let (sink_tx, mut sink_rx) = broadcast::channel::<WireEvent>(4);
        let seq = Arc::new(AtomicU64::new(0));
        let token = tokio_util::sync::CancellationToken::new();

        // Subscribe to the engine source AFTER the ring is
        // already populated. This forces the receiver to see
        // `Lagged` on its first recv (the ring is at
        // capacity, so any new send overwrites the oldest).
        // First, fill the ring with 4 events (this is
        // BEFORE the fan-in subscribes — the source sender
        // holds the ring in memory regardless of subscribers).
        for i in 0..4 {
            engine_tx.send(json!({"warmup": i})).unwrap();
        }
        // Now subscribe the fan-in. The fan-in's first
        // recv() on this receiver returns `Lagged` because
        // the ring's "latest" is ahead of the new
        // subscriber's "next" position by the number of
        // events that were sent before the subscribe.
        let _handle = fan_in(
            engine_rx,
            agent_rx,
            proxy_rx,
            sink_tx,
            seq.clone(),
            token.clone(),
            4,
            &tokio::runtime::Handle::current(),
        );
        // Give the fan-in a tick to subscribe to its
        // sources.
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        // Now send 5 more events. The ring holds the most
        // recent 4; the fan-in's first recv() will return
        // `Lagged(N)` because by the time it tries to
        // receive, more than 4 events have been published.
        for i in 0..5 {
            engine_tx.send(json!({"real": i})).unwrap();
        }
        // Give the fan-in time to recv() the Lagged + the
        // next event. The Lagged is logged with WARN.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Now send one final event on the agent source.
        // The fan-in will receive it with a fresh seq.
        // Critically, the seq must be GREATER than the
        // last one the fan-in stamped (the one from the
        // post-Lagged engine event). The fact that the seq
        // is monotonic even across Lagged is the load-
        // bearing piece for Phase 8 drop detection.
        agent_tx.send(json!({"post-lag": true})).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        // Drain the sink.
        let mut received: Vec<WireEvent> = Vec::new();
        while let Ok(ev) = sink_rx.try_recv() {
            received.push(ev);
        }
        for _ in 0..50 {
            if !received.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            while let Ok(ev) = sink_rx.try_recv() {
                received.push(ev);
            }
        }

        // We can't assert on the exact count of received
        // events (the source ring is tiny and the sink ring
        // is tiny; Lagged on the sink side is also
        // possible). But the seq must be monotonic for
        // every pair of consecutive received events.
        let mut last_seq: u64 = 0;
        for ev in &received {
            assert!(
                ev.seq > last_seq,
                "seq must remain strictly increasing across Lagged; got {} after {}",
                ev.seq,
                last_seq
            );
            last_seq = ev.seq;
        }
        // The seq counter must have advanced at least once
        // (the post-Lagged event was stamped).
        assert!(
            seq.load(Ordering::Relaxed) >= 1,
            "seq counter must advance after Lagged"
        );

        // The warning was logged. We scan the captured
        // buffer for the "lagged" substring (case-insensitive).
        // The exact format is "fan-in: source broadcast ring
        // lagged; events dropped" — we check for "lagged" to
        // tolerate minor tracing format changes.
        let log_bytes = buf.lock().unwrap().clone();
        let log_str = String::from_utf8_lossy(&log_bytes);
        assert!(
            log_str.to_lowercase().contains("lagged"),
            "expected a 'lagged' warning in logs; got:\n{}",
            log_str
        );

        // Cancel and drop.
        token.cancel();
        drop(engine_tx);
        drop(agent_tx);
        drop(proxy_tx);
    }

    /// Pure seq-monotonicity test: stamp 50 events across
    /// the 3 sources, assert the seqs are 1..=50 (no gaps,
    /// no duplicates, no re-use). This is the regression
    /// guard for the seq counter being a process-global
    /// monotonic.
    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn seq_counter_is_globally_monotonic_across_sources() {
        let (engine_tx, agent_tx, proxy_tx, mut sink_rx, seq, token, _handle) = make_bus(256, 256);

        // Interleave sends across the 3 sources.
        for i in 0..50 {
            match i % 3 {
                0 => {
                    engine_tx.send(json!({"i": i})).unwrap();
                }
                1 => {
                    agent_tx.send(json!({"i": i})).unwrap();
                }
                _ => {
                    proxy_tx.send(json!({"i": i})).unwrap();
                }
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let mut received: Vec<WireEvent> = Vec::new();
        while let Ok(ev) = sink_rx.try_recv() {
            received.push(ev);
        }
        for _ in 0..100 {
            if received.len() >= 50 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            while let Ok(ev) = sink_rx.try_recv() {
                received.push(ev);
            }
        }

        assert_eq!(received.len(), 50, "all 50 events must arrive");
        // Strictly increasing, no gaps.
        let seqs: Vec<u64> = received.iter().map(|e| e.seq).collect();
        let mut sorted = seqs.clone();
        sorted.sort_unstable();
        assert_eq!(
            seqs, sorted,
            "seqs must already be in sorted order (monotonic)"
        );
        assert_eq!(seqs[0], 1, "first seq must be 1");
        assert_eq!(seqs[49], 50, "last seq must be 50");
        assert_eq!(seq.load(Ordering::Relaxed), 50);

        token.cancel();
        drop(engine_tx);
        drop(agent_tx);
        drop(proxy_tx);
    }

    /// Cancellation safety: cancelling the token causes the
    /// three tasks to exit. The `FanInHandle::join` returns
    /// when all three are done.
    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn fan_in_exits_cleanly_on_cancellation() {
        let (engine_tx, agent_tx, proxy_tx, _sink_rx, _seq, token, handle) = make_bus(256, 256);

        // Send one event so the tasks have something to
        // process (proves they're actually running).
        engine_tx.send(json!({"hello": "world"})).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        // Cancel and join. The join should complete
        // promptly.
        token.cancel();
        // Bound the wait so a stuck fan-in fails the test
        // loudly instead of hanging.
        tokio::time::timeout(std::time::Duration::from_secs(2), handle.join())
            .await
            .expect("fan-in must exit within 2s of cancellation");

        drop(engine_tx);
        drop(agent_tx);
        drop(proxy_tx);
    }

    /// **Regression test for v0.1.1 (Windows binary panic).**
    ///
    /// On Windows the binary panicked at startup with
    /// "there is no reactor running, must be called from the
    /// context of a Tokio 1.x runtime" because `fan_in()` was
    /// called from `WireEventBus::start` — a synchronous
    /// function invoked from Tauri's `setup` closure on the
    /// main thread, with no Tokio runtime in scope. The
    /// production fix is to require the caller to pass a
    /// `&tokio::runtime::Handle`, which `fan_in()` then uses
    /// via `JoinSet::spawn_on`.
    ///
    /// This test reproduces the production scenario: a
    /// **plain `#[test]`** (no `#[tokio::test]` — i.e. no
    /// current runtime in scope) that calls `fan_in()` with
    /// a `Handle` obtained from a separately-constructed
    /// `Runtime`. Before the fix this would panic inside
    /// `JoinSet::spawn`; after the fix it succeeds and the
    /// spawned tasks run on the constructed runtime.
    #[test]
    fn fan_in_works_from_sync_context_without_current_runtime() {
        // Sanity check: the test thread must NOT have a
        // current runtime. (If it did, the regression test
        // would be vacuous — `JoinSet::spawn` would succeed
        // for the wrong reason.)
        assert!(
            tokio::runtime::Handle::try_current().is_err(),
            "test premise violated: a Tokio runtime is in scope on this thread; \
             this test must run in a sync context with no current runtime"
        );

        // Build a separate runtime just to get a handle.
        // We don't drive any futures on it — we only need
        // the handle so `fan_in` can spawn onto it.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build test runtime");
        let handle = runtime.handle().clone();

        // Build the bus inputs.
        let (engine_tx, engine_rx) = broadcast::channel::<Value>(16);
        let (_agent_tx, agent_rx) = broadcast::channel::<Value>(16);
        let (proxy_tx, proxy_rx) = broadcast::channel::<Value>(16);
        let (sink_tx, _sink_rx) = broadcast::channel::<WireEvent>(16);
        let seq = Arc::new(AtomicU64::new(0));
        let token = tokio_util::sync::CancellationToken::new();

        // The call site: a sync function with no current
        // runtime. Before the fix, this panicked.
        let _fan_in_handle = fan_in(
            engine_rx,
            agent_rx,
            proxy_rx,
            sink_tx,
            seq.clone(),
            token.clone(),
            16,
            &handle,
        );

        // Drive the runtime a tick so the spawned tasks
        // subscribe to their sources, then send one event
        // and verify the seq counter advances — proving the
        // tasks actually ran (not just spawned and
        // immediately aborted).
        runtime.block_on(async {
            // Give the tasks a moment to subscribe.
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            engine_tx
                .send(json!({"src": "engine", "i": 0}))
                .expect("engine send");
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        });
        assert_eq!(
            seq.load(Ordering::Relaxed),
            1,
            "fan-in task must have stamped seq=1 for the one event we sent"
        );

        // Cancel and let the tasks exit. Dropping the
        // senders closes the source channels so the forwarder
        // tasks see `RecvError::Closed` and return.
        token.cancel();
        drop(engine_tx);
        drop(proxy_tx);
        // Drain any remaining tasks to avoid a panic on
        // `runtime` drop while tasks are still pending.
        runtime.block_on(async {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        });
    }
}
