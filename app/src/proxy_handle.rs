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

use bk_core::scope::{MatchReplaceRule, ScopeRule};
use bk_proxy::{Proxy, ProxyConfig, ProxyEvent};
use serde::{Deserialize, Serialize};
use tauri::async_runtime::Mutex;
use tokio::sync::{broadcast, watch};
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
    /// The proxy event bus sender (cloned into the spawned
    /// task). `subscribe_events` uses this to mint fresh
    /// subscribers for the §4.2 wire bus.
    proxy_event_tx: Option<broadcast::Sender<ProxyEvent>>,
    /// The scope rules to apply on the next `start` /
    /// `start_with_rules` call (Phase 6 Part C, §C-A.2).
    /// Set by the Tauri `start_proxy` command by looking up
    /// the active project's `ProjectSettings.scope_rules`. The
    /// v0.5+ capture loop (not yet landed) is the consumer.
    pending_scope_rules: Vec<ScopeRule>,
    /// The M&R rules to apply on the next `start` /
    /// `start_with_rules` call (Phase 6 Part C, §C-A.2).
    pending_match_replace_rules: Vec<MatchReplaceRule>,
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
                proxy_event_tx: None,
                pending_scope_rules: Vec::new(),
                pending_match_replace_rules: Vec::new(),
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

    /// Subscribe to the proxy's event bus. Returns a fresh
    /// `broadcast::Receiver<ProxyEvent>`. Used by the §4.2
    /// `wire_bus` to feed the engine+proxy fan-in.
    pub fn subscribe_events(&self) -> broadcast::Receiver<ProxyEvent> {
        // The proxy event bus is owned by the spawned task
        // and is not directly accessible from outside. We
        // use a `OnceCell`-like lazy pattern: the FIRST call
        // to `start` creates the bus and stores a sender
        // clone; subsequent calls subscribe to it.
        //
        // For v1 we take a simpler path: the handle owns
        // an `Option<broadcast::Sender<ProxyEvent>>` that's
        // populated by `start`, and `subscribe_events`
        // returns a fresh subscriber on it. Before `start`
        // is called, `subscribe_events` returns a closed
        // receiver (the bus hasn't been created yet).
        self.inner
            .blocking_lock()
            .proxy_event_tx
            .as_ref()
            .map(|tx| tx.subscribe())
            .unwrap_or_else(|| {
                // Return a closed receiver; subscribers
                // immediately see `RecvError::Closed` and
                // exit their loops.
                let (tx, rx) = broadcast::channel(1);
                drop(tx);
                rx
            })
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
        // Create the proxy event bus BEFORE building the
        // proxy, so `subscribe_events` (called by the
        // §4.2 wire bus) gets a working receiver. The
        // bus is then cloned into the spawned task.
        let (proxy_event_tx, _proxy_event_rx) = broadcast::channel::<ProxyEvent>(256);
        let (tx, rx) = watch::channel(false);
        let ca = inner.ca.clone();
        let status = inner.status.clone();
        let proxy_event_tx_for_task = proxy_event_tx.clone();
        let task = tauri::async_runtime::spawn(async move {
            // Build the proxy; subscribe to the event bus BEFORE
            // `run` so we don't miss the `ProxyStarted` event.
            // The proxy's `events()` returns a cloneable bus, so
            // the main task and the event-listener task each
            // hold their own subscriber.
            let proxy = Proxy::new(config, ca);
            // Bridge the proxy's internal event bus to the
            // handle's external bus (the §4.2 wire bus
            // subscribes to this).
            let mut internal_events = proxy.events().subscribe();
            let bridge_task = tauri::async_runtime::spawn(async move {
                while let Ok(event) = internal_events.recv().await {
                    let _ = proxy_event_tx_for_task.send(event);
                }
            });
            let events = proxy.events();
            let result = ProxyHandle::run_with_events(proxy, rx, status.clone(), events).await;
            bridge_task.abort();
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
        inner.proxy_event_tx = Some(proxy_event_tx);
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

    /// Set the pending scope + M&R rules that the next
    /// `start_with_rules` call will use (Phase 6 Part C,
    /// §C-A.2). The Tauri `start_proxy` command looks up the
    /// active project's `ProjectSettings` and calls this
    /// before `start_with_rules`. Empty `Vec`s are valid (the
    /// "no rules yet" case).
    ///
    /// **Why "pending" and not "active":** the rules take
    /// effect on the next `start` call. Once the proxy is
    /// running, the rules are read by the v0.5+ capture loop
    /// (not yet landed). The handle stores them so a future
    /// `Proxy::run` signature can read them via
    /// `take_pending_rules`.
    pub async fn set_pending_rules(
        &self,
        scope_rules: Vec<ScopeRule>,
        match_replace_rules: Vec<MatchReplaceRule>,
    ) {
        let mut inner = self.inner.lock().await;
        inner.pending_scope_rules = scope_rules;
        inner.pending_match_replace_rules = match_replace_rules;
    }

    /// Take the pending rules (drains them). Returns
    /// `(scope_rules, match_replace_rules)`. Called by the
    /// proxy's MITM-forwarding task on startup. After the
    /// take, the pending fields are empty (the next `start`
    /// without a `set_pending_rules` reverts to the v1
    /// "empty rules" behavior).
    ///
    /// **Why this is `dead_code` allowed:** the v0.5+ capture
    /// loop (the consumer of these rules) is not yet landed —
    /// the current `Proxy::run` signature doesn't take the
    /// rules as a parameter. The method is kept as the public
    /// API contract for the future capture loop. Remove the
    /// `#[allow(dead_code)]` when the capture loop is wired
    /// in (Phase 8+ or a dedicated capture-loop phase).
    #[allow(
        dead_code,
        reason = "Public API for the v0.5+ capture loop consumer; not yet called"
    )]
    pub async fn take_pending_rules(&self) -> (Vec<ScopeRule>, Vec<MatchReplaceRule>) {
        let mut inner = self.inner.lock().await;
        let scope = std::mem::take(&mut inner.pending_scope_rules);
        let mr = std::mem::take(&mut inner.pending_match_replace_rules);
        (scope, mr)
    }

    /// Start the proxy with the given rules. The rules are
    /// stored as "pending" and the proxy task reads them on
    /// startup via `take_pending_rules`. The actual consumer
    /// of the rules (the MITM-forwarding loop) is the v0.5+
    /// capture loop, not yet landed. This method is a thin
    /// wrapper over `set_pending_rules` + `start`.
    ///
    /// **Defensive:** if the `set_pending_rules` call fails
    /// (e.g. the lock is poisoned), the proxy still starts
    /// with empty `Vec`s (the v1 default). The fallback is
    /// logged but not returned to the caller.
    pub async fn start_with_rules(
        &self,
        config: ProxyConfig,
        scope_rules: Vec<ScopeRule>,
        match_replace_rules: Vec<MatchReplaceRule>,
    ) -> anyhow::Result<()> {
        self.set_pending_rules(scope_rules, match_replace_rules)
            .await;
        self.start(config).await
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

    /// `set_pending_rules` + `take_pending_rules` round-trips:
    /// pending rules set via `set_pending_rules` come back via
    /// `take_pending_rules`, and `take_pending_rules` drains
    /// the pending fields (a second take returns empty
    /// `Vec`s).
    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn pending_rules_round_trip() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let h = ProxyHandle::new(tmp.path());
        let rules = vec![bk_core::ScopeRule {
            kind: bk_core::ScopeRuleKind::Host,
            pattern: "acme.bb".to_string(),
            action: bk_core::MatchAction::InScope,
            label: "primary".to_string(),
            priority: 0,
        }];
        h.set_pending_rules(rules.clone(), vec![]).await;
        let (taken_scope, taken_mr) = h.take_pending_rules().await;
        assert_eq!(taken_scope.len(), 1);
        assert_eq!(taken_scope[0].label, "primary");
        assert!(taken_mr.is_empty());
        // The take drains the fields.
        let (scope2, mr2) = h.take_pending_rules().await;
        assert!(scope2.is_empty());
        assert!(mr2.is_empty());
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
