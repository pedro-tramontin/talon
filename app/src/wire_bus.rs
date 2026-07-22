//! §4.2 — WireEvent bus: engine + proxy events fanned into a
//! single `WireEvent` source for the React UI.
//!
//! ## Why this module exists
//!
//! The §4.0 PR added an additive `wire_event` emit alongside
//! every existing typed agent emit (per the §3.5d design — the
//! React agent store subscribes to typed `agent_event`s today,
//! the wire envelope is a future-proofing for Phase 8). §4.2
//! closes the loop for the engine + proxy paths: instead of
//! per-emit `wire_event` calls scattered through the code,
//! the engine's and proxy's event buses feed a single
//! `bk_events::fan_in` task, whose output is re-emitted to
//! the Tauri webview as `wire_event` events.
//!
//! ## Why the agent is NOT in the fan-in
//!
//! The agent's event bus is per-run (a fresh `agent_channel()`
//! is created in `agent_start` and torn down when the run
//! ends). The fan-in's contract is "3 long-lived sources" —
//! the agent doesn't fit. The §4.0 additive per-emit
//! `wire_event` is the right shape for the agent (it carries
//! the same seq counter via `WireEventSeq`, so the React
//! side sees a single monotonic seq across engine, agent, and
//! proxy events). The §4.3-4.4 React migration will switch
//! the agent store to the `wire_event` channel and drop the
//! typed `agent_event` channel.
//!
//! ## What this module does
//!
//! 1. On `setup`, subscribes to the engine's `EventReceiver`
//!    and the proxy's `ProxyEventBus` (via the `ProxyHandle`).
//! 2. Spawns a `bk_events::fan_in` task that fans the two
//!    sources into a single `broadcast::Sender<WireEvent>`.
//!    A third "dummy" channel (closed immediately) is passed
//!    as the agent slot to satisfy the fan-in's 3-source
//!    signature; the agent uses the §4.0 additive path
//!    instead.
//! 3. Spawns an emit task that `recv()`s from the fan-in
//!    output and calls `app.emit_to(WEBVIEW_LABEL,
//!    WIRE_EVENT_LABEL, wire)`.

use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use bk_engine::Engine;
use bk_events::fan_in::{fan_in as bk_fan_in, FanInHandle};
use bk_events::WireEvent;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::proxy_handle::ProxyHandleArc;
use crate::wire::{WireEventSeq, WIRE_EVENT_LABEL};

/// The Tauri webview label, matches the existing
/// `WEBVIEW_LABEL` constant in `agent.rs` (single window,
/// pinned by the Tauri config). Re-exported as `pub(crate)`
/// so other modules (e.g. `commands::replay`) can emit to
/// the same webview without duplicating the string.
pub(crate) const WEBVIEW_LABEL: &str = "main";

/// Handle to the running fan-in. Held in `tauri::State` so
/// the setup hook (or a future shutdown hook) can cancel it
/// cleanly.
#[derive(Default)]
pub struct WireEventBus {
    /// `Some` when the fan-in is running; `None` otherwise.
    pub handle: Option<WireBusHandle>,
}

/// The bus's runtime: cancellation token + fan-in handle.
pub struct WireBusHandle {
    /// Cancels the fan-in's 3 forwarder tasks.
    pub cancel: CancellationToken,
    /// The fan-in's JoinSet (drained on `stop`).
    pub fan_in: FanInHandle,
    /// The emit-task JoinHandles (drained on `stop`).
    pub emit_tasks: Vec<tauri::async_runtime::JoinHandle<()>>,
}

impl WireEventBus {
    /// Build an empty bus. The setup hook calls `start` to
    /// spin up the fan-in.
    pub fn new() -> Self {
        Self::default()
    }

    /// Start the bus. Idempotent: a no-op if already running.
    ///
    /// `engine` provides the engine event bus. `proxy`
    /// provides the proxy event bus. `seq_counter` is the
    /// process-global seq counter (the same one the §4.0
    /// additive `wire_event` emits use, so the React side
    /// sees a single monotonic seq across all 3 sources).
    /// `app` is the Tauri app handle used for the
    /// `emit_to` calls in the emit tasks.
    pub fn start(
        &mut self,
        app: AppHandle,
        engine: Arc<Engine>,
        proxy: ProxyHandleArc,
        seq_counter: Arc<AtomicU64>,
    ) {
        if self.handle.is_some() {
            return; // already running; idempotent
        }
        // Subscribe to the engine + proxy buses BEFORE
        // spawning the fan-in so we don't miss the first
        // events.
        let engine_rx = engine.subscribe_events();
        let proxy_rx = proxy.subscribe_events();
        // We need Value-typed receivers for the fan-in
        // helper. The engine and proxy buses carry typed
        // events; we map them to Value via per-source
        // forwarder tasks that serialize to JSON and send
        // to a `broadcast::Sender<Value>`. This indirection
        // keeps the fan-in helper source-type-agnostic.
        let (engine_value_tx, engine_value_rx) = broadcast::channel::<Value>(256);
        let (proxy_value_tx, proxy_value_rx) = broadcast::channel::<Value>(256);
        // Forwarder task for the engine: pull typed events,
        // serialize to JSON, send to the Value channel.
        let engine_value_tx_clone = engine_value_tx.clone();
        let engine_forwarder = tauri::async_runtime::spawn(async move {
            let mut rx = engine_rx;
            loop {
                match rx.recv().await {
                    Ok(ev) => {
                        let v = serde_json::to_value(&ev).unwrap_or(Value::Null);
                        if engine_value_tx_clone.send(v).is_err() {
                            // No receivers; the fan-in
                            // task is gone. Exit.
                            return;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!(missed = n, "wire_bus: engine bus lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        });
        // Forwarder task for the proxy: same shape.
        let proxy_value_tx_clone = proxy_value_tx.clone();
        let proxy_forwarder = tauri::async_runtime::spawn(async move {
            let mut rx = proxy_rx;
            loop {
                match rx.recv().await {
                    Ok(ev) => {
                        let v = serde_json::to_value(&ev).unwrap_or(Value::Null);
                        if proxy_value_tx_clone.send(v).is_err() {
                            return;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!(missed = n, "wire_bus: proxy bus lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        });
        // The third "agent" source for the fan-in is a
        // dummy: the agent uses the §4.0 additive per-emit
        // path, so we hand the fan-in a closed receiver.
        // The forwarder task for this slot exits
        // immediately on `RecvError::Closed`.
        let (_dummy_tx, dummy_rx) = broadcast::channel::<Value>(1);
        drop(_dummy_tx);
        // Spawn the fan-in helper against the 3 Value
        // channels (engine, agent-stub, proxy).
        let (sink_tx, _sink_rx_unused) = broadcast::channel::<WireEvent>(256);
        let cancel = CancellationToken::new();
        let fan_in = bk_fan_in(
            engine_value_rx,
            dummy_rx,
            proxy_value_rx,
            sink_tx.clone(),
            seq_counter,
            cancel.clone(),
            256,
            // Pass Tauri's internal runtime handle so the
            // fan-in's 3 forwarder tasks can be spawned
            // without requiring a Tokio runtime to be in
            // scope at this call site. `start()` is a sync
            // function invoked from Tauri's `setup` closure
            // on the main thread; before this fix,
            // `JoinSet::spawn` panicked with "there is no
            // reactor running". `tauri::async_runtime::handle()`
            // returns the global Tauri runtime's handle, and
            // `.inner()` exposes it as a `&tokio::runtime::Handle`
            // that `JoinSet::spawn_on` accepts.
            tauri::async_runtime::handle().inner(),
        );
        // Emit task: recv from sink, re-emit to the Webview.
        let app_for_emit = app.clone();
        let emit_task = tauri::async_runtime::spawn(async move {
            let mut rx = sink_tx.subscribe();
            while let Ok(wire) = rx.recv().await {
                if let Err(e) = app_for_emit.emit_to(WEBVIEW_LABEL, WIRE_EVENT_LABEL, &wire) {
                    error!(
                        kind = %wire.kind,
                        seq = wire.seq,
                        error = %e,
                        "wire_bus: emit wire_event failed"
                    );
                }
            }
        });
        self.handle = Some(WireBusHandle {
            cancel,
            fan_in,
            emit_tasks: vec![emit_task, engine_forwarder, proxy_forwarder],
        });
        info!("wire_bus: started (engine + proxy; agent uses §4.0 additive path)");
    }

    /// Stop the bus. Idempotent: a no-op if not running.
    pub fn stop(&mut self) {
        if let Some(mut h) = self.handle.take() {
            h.cancel.cancel();
            h.fan_in.abort();
            for t in h.emit_tasks.drain(..) {
                t.abort();
            }
        }
    }
}

impl Drop for WireEventBus {
    fn drop(&mut self) {
        self.stop();
    }
}

/// §4.2 — `setup_wire_bus` is the setup hook for the bus.
/// Called from `app::run`'s `setup` closure; spins up the
/// fan-in. Re-acquires the `WireEventSeq` state's inner
/// `Arc<AtomicU64>` and passes it to the bus.
pub fn setup_wire_bus(app: &AppHandle) {
    let engine = app.state::<crate::commands::EngineArc>().inner().clone();
    let proxy = app.state::<ProxyHandleArc>().inner().clone();
    let seq_state = app.state::<WireEventSeq>().inner().clone();
    let seq_counter: Arc<AtomicU64> = seq_state.0.clone();
    // Build a fresh bus and re-store it in `tauri::State`.
    // (We can't mutate the existing `WireEventBus` state
    // through `State<>` because it derefs to the inner
    // value; re-`manage` replaces it.)
    let mut bus = WireEventBus::new();
    bus.start(app.clone(), engine, proxy, seq_counter);
    app.manage(bus);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bus starts in the "not running" state.
    #[test]
    fn bus_starts_in_stopped_state() {
        let bus = WireEventBus::new();
        assert!(bus.handle.is_none());
    }

    /// `stop` is idempotent: a no-op on a stopped bus.
    #[test]
    fn stop_is_idempotent() {
        let mut bus = WireEventBus::new();
        bus.stop();
        // Calling stop twice is fine.
        bus.stop();
        assert!(bus.handle.is_none());
    }
}
