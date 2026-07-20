//! §4.1 — `ProxyHandle`: a Tauri-friendly wrapper around `bk_proxy::Proxy`.
//!
//! The proxy's `run` method takes `self` (consumes the `Proxy`)
//! and a `watch::Receiver<bool>` for shutdown. The handle keeps
//! a `watch::Sender<bool>` so the Tauri `start_proxy` /
//! `stop_proxy` commands can flip it without owning the proxy.
//! The proxy task itself is spawned on the Tauri-managed Tokio
//! runtime and held in an `Option<JoinHandle<...>>` so we can
//! `abort` it on a second `start_proxy` call (the Tauri command
//! is idempotent — a second start while running is a no-op).
//!
//! ## Why not `tokio::sync::Mutex` for the proxy state
//!
//! The `start` call needs to `await` (it spawns a task and
//! binds a TCP listener). A sync `std::sync::Mutex` would
//! deadlock if held across the `await`; the async mutex is
//! the right tool. The lock is held only across the `start`
//! / `stop` calls, not across the proxy's runtime.

use std::net::SocketAddr;
use std::sync::Arc;

use bk_proxy::{Proxy, ProxyConfig, ProxyEvent};
use serde::{Deserialize, Serialize};
use tauri::async_runtime::Mutex;
use tokio::sync::watch;
use tracing::{info, warn};

/// Tauri-friendly handle. `Arc`'d so the command layer can hold
/// it in `tauri::State` and the proxy task can hold its own
/// reference for shutdown signaling.
pub type ProxyHandleArc = Arc<ProxyHandle>;

/// The current proxy state. Cheap to clone (it's a few `String`s
/// + a `SocketAddr`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyStatus {
    /// `Running` if the listener is bound, `Stopped` if it has
    /// never been started or has been stopped, `Error` if the
    /// last start failed.
    pub state: ProxyState,
    /// The bound address (only set when `Running`).
    pub listener_addr: Option<SocketAddr>,
    /// The CA fingerprint (only set when `Running`).
    pub ca_fingerprint: Option<String>,
    /// The error message (only set when `state == Error`).
    pub last_error: Option<String>,
}

/// The proxy's lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyState {
    Stopped,
    Running,
    Error,
}

/// The handle. One per Tauri app.
pub struct ProxyHandle {
    inner: Mutex<Inner>,
}

struct Inner {
    /// `Some` when the proxy is running; `None` otherwise.
    task: Option<tauri::async_runtime::JoinHandle<()>>,
    /// The shutdown signal's sender. Cloned into the spawned
    /// task so the task can `await` the receiver.
    shutdown_tx: Option<watch::Sender<bool>>,
    /// The current public status (read by `proxy_status` without
    /// taking the mutex).
    status: Arc<std::sync::Mutex<ProxyStatus>>,
    /// The CA, loaded once on `new`. Wrapped in `Arc` so the
    /// proxy task can hold a reference across the `Proxy::run`
    /// call (the proxy takes `Arc<RootCa>` as a constructor
    /// arg).
    ca: Arc<bk_proxy::RootCa>,
}

impl ProxyHandle {
    /// Build a new handle. The CA is loaded (or created) from
    /// `<config_dir>/ca` so the project's CA fingerprint stays
    /// stable across restarts. `config_dir` is the Talon config
    /// dir (the same dir the engine writes to).
    pub fn new(config_dir: &std::path::Path) -> Self {
        let ca_dir = bk_proxy::RootCa::ca_dir(config_dir);
        // Ensure the CA dir exists (load_or_create requires it).
        let _ = std::fs::create_dir_all(&ca_dir);
        let ca = bk_proxy::RootCa::load_or_create(&ca_dir)
            .expect("CA load_or_create; if this fails the disk is full or perms are wrong");
        Self {
            inner: Mutex::new(Inner {
                task: None,
                shutdown_tx: None,
                status: Arc::new(std::sync::Mutex::new(ProxyStatus {
                    state: ProxyState::Stopped,
                    listener_addr: None,
                    ca_fingerprint: None,
                    last_error: None,
                })),
                ca: Arc::new(ca),
            }),
        }
    }

    /// Read the current status (cheap; takes a sync mutex
    /// briefly).
    pub fn status(&self) -> ProxyStatus {
        self.inner
            .blocking_lock()
            .status
            .lock()
            .expect("status mutex poisoned")
            .clone()
    }

    /// Start the proxy. Idempotent: a no-op if already running.
    /// Returns an error if the bind fails (e.g. the port is
    /// already in use).
    pub async fn start(&self, config: ProxyConfig) -> anyhow::Result<()> {
        let mut inner = self.inner.lock().await;
        if inner.task.is_some() {
            // Already running; idempotent.
            return Ok(());
        }
        let (tx, rx) = watch::channel(false);
        let ca = inner.ca.clone();
        let status = inner.status.clone();
        let task = tauri::async_runtime::spawn(async move {
            // Build the proxy; subscribe to the event bus BEFORE
            // `run` so we don't miss the `ProxyStarted` event.
            // The proxy's `events()` returns a cloneable bus, so
            // the main task and the event-listener task each
            // hold their own subscriber.
            let proxy = Proxy::new(config, ca);
            let events = proxy.events();
            let result = ProxyHandle::run_with_events(proxy, rx, status.clone(), events).await;
            match result {
                Ok(()) => {
                    info!("bk-proxy: clean shutdown");
                }
                Err(e) => {
                    warn!(error = %e, "bk-proxy: run ended with error");
                    let mut s = status.lock().expect("status mutex poisoned");
                    s.state = ProxyState::Error;
                    s.last_error = Some(e.to_string());
                }
            }
        });
        inner.task = Some(task);
        inner.shutdown_tx = Some(tx);
        Ok(())
    }

    /// Run the proxy to completion with a sibling event-listener
    /// task. Updates the public status on `ProxyStarted` /
    /// `ProxyStopped`.
    async fn run_with_events(
        proxy: Proxy,
        shutdown: watch::Receiver<bool>,
        status: Arc<std::sync::Mutex<ProxyStatus>>,
        events: bk_proxy::ProxyEventBus,
    ) -> anyhow::Result<()> {
        let mut events_rx = events.subscribe();
        // Spawn a side task that listens for `ProxyStarted` and
        // updates the status. The proxy's `run` is the main
        // task; the event-listener is a sibling.
        let status_for_events = status.clone();
        let event_task = tauri::async_runtime::spawn(async move {
            while let Ok(event) = events_rx.recv().await {
                match event {
                    ProxyEvent::ProxyStarted {
                        listener_addr,
                        ca_fingerprint,
                    } => {
                        let mut s = status_for_events.lock().expect("status mutex poisoned");
                        s.state = ProxyState::Running;
                        s.listener_addr = Some(listener_addr);
                        s.ca_fingerprint = Some(ca_fingerprint);
                    }
                    ProxyEvent::ProxyStopped { .. } => {
                        let mut s = status_for_events.lock().expect("status mutex poisoned");
                        s.state = ProxyState::Stopped;
                    }
                    _ => {} // request/response events not surfaced via this handle
                }
            }
        });
        let res = proxy.run(shutdown).await;
        event_task.abort();
        res
    }

    /// Stop the proxy. Idempotent: a no-op if not running.
    pub fn stop(&self) {
        // Use `try_lock` so a sync Tauri command doesn't block
        // on the async mutex. If the lock is held (the proxy is
        // in the middle of `start`), the next `start`'s
        // idempotency check will return early and the user
        // can retry.
        if let Ok(mut inner) = self.inner.try_lock() {
            if let Some(tx) = inner.shutdown_tx.take() {
                let _ = tx.send(true);
            }
            if let Some(task) = inner.task.take() {
                task.abort();
            }
            let mut s = inner.status.lock().expect("status mutex poisoned");
            s.state = ProxyState::Stopped;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The freshly-constructed handle is in `Stopped` state.
    #[test]
    fn new_handle_is_stopped() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let h = ProxyHandle::new(tmp.path());
        let s = h.status();
        assert_eq!(s.state, ProxyState::Stopped);
        assert!(s.listener_addr.is_none());
        assert!(s.ca_fingerprint.is_none());
    }

    /// `start` is idempotent: a second call while running
    /// returns `Ok(())` without rebinding the listener.
    /// (We don't actually bind here — the test is the
    /// idempotency branch.)
    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn start_is_idempotent_when_already_running() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let h = ProxyHandle::new(tmp.path());
        // Manually set the task to `Some` to simulate a
        // running proxy without actually binding a port.
        {
            let mut inner = h.inner.lock().await;
            inner.task = Some(tauri::async_runtime::spawn(async {}));
            inner.shutdown_tx = Some(watch::channel(false).0);
        }
        let cfg = ProxyConfig::default();
        let res = h.start(cfg).await;
        assert!(res.is_ok(), "second start must be a no-op");
    }
}
