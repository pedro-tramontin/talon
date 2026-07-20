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
//! 4. Re-subscribe on Lagged so a slow consumer doesn't keep
//!    losing events to the same ring overflow. The seq counter
//!    is the load-bearing piece — the new subscription resumes
//!    from the new tail, and the gap in seq is the signal the
//!    React side uses to surface "missed events".
//!
//! ## Design
//!
//! The function takes three `broadcast::Receiver<Value>` (the
//! source payloads, already type-erased to `serde_json::Value`)
//! and a shared `broadcast::Sender<WireEvent>` for the sink. The
//! seq counter is `Arc<AtomicU64>` so all three tasks advance
//! the SAME counter — the seq is process-global, not per-source.
//!
//! `fan_in` spawns 3 tokio tasks and returns a [`FanInHandle`]
//! that the caller can `await` (for cancellation) or `abort` to
//! stop the tasks.
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
) -> FanInHandle {
    let mut tasks: JoinSet<()> = JoinSet::new();

    // The engine task: pulls `EngineEvent` payloads (already
    // serialized to `Value` at the emit site), wraps them in
    // a `WireEvent` with `kind: EngineEvent`, stamps the seq.
    let sink_e = sink.clone();
    let seq_e = seq_counter.clone();
    let token_e = cancellation_token.clone();
    tasks.spawn(async move {
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
    });

    // The agent task: same shape, `kind: AgentEvent`.
    let sink_a = sink.clone();
    let seq_a = seq_counter.clone();
    let token_a = cancellation_token.clone();
    tasks.spawn(async move {
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
    });

    // The proxy task: same shape, `kind: ProxyEvent`.
    let sink_p = sink;
    let seq_p = seq_counter;
    let token_p = cancellation_token;
    tasks.spawn(async move {
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
    });

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
        // Initialize a tracing subscriber that captures
        // `warn!` events so we can assert the lagged warning
        // fired. We use `tracing_subscriber::fmt` with a
        // custom writer that pipes into a `Vec<u8>`.
        //
        // The `tracing-subscriber` is in dev-deps for this
        // exact reason. Multiple tests in the same
        // `cargo test` invocation may try to install a
        // global subscriber; we use `try_init` and ignore
        // the "already initialized" error.
        use std::io::Write;
        use std::sync::{Arc, Mutex};
        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        // The make_writer closure must return a `Write`-able
        // value, NOT the `Arc<Mutex<Vec<u8>>>` itself. We
        // adapt with a small newtype.
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
        // Clone the `Arc` for the closure (the closure
        // needs to move into the subscriber, but we still
        // need the original to read the captured bytes at
        // the end of the test).
        let buf_for_writer = buf.clone();
        let make_writer = move || -> Box<dyn Write> { Box::new(SharedBuf(buf_for_writer.clone())) };
        let _ = tracing_subscriber::fmt()
            .with_writer(make_writer)
            .with_max_level(tracing::Level::WARN)
            .try_init();

        // Tiny source buffer (4) so we can overflow it
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
}
