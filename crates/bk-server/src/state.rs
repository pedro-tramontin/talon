//! The shared `AppState` for axum handlers.
//!
//! Mirrors the `app::AppState` that the Tauri shell uses, but
//! the surface is narrowed to what the HTTP/WS layer needs.
//! The Engine is the source of truth; the WS hub carries
//! the wire-event bus subscription; the proxy closures are
//! the indirection that lets the `app` crate's
//! `ProxyHandle` be reached without leaking the Tauri
//! runtime types into `bk-server`.

use std::sync::Arc;

use bk_engine::Engine;

use crate::ws::WsHub;
use crate::{ProxyStatusFn, StartProxyFn, StopProxyFn};

/// Shared state for the axum router. Cheap to clone (each
/// field is either `Arc`, a function pointer, or a small
/// `Vec`).
#[derive(Clone)]
pub struct AppState {
    /// The Talon engine. All project / exchange / settings
    /// reads + writes go through this.
    pub store: Arc<Engine>,
    /// The WebSocket hub. The handlers + the WS upgrade both
    /// use this to broadcast events to connected clients.
    pub ws: WsHub,
    /// Optional proxy-control closure (start). The `app`
    /// crate wraps its `ProxyHandle::start_with_rules` in a
    /// closure that captures the Engine + the
    /// `ProxyHandle` itself.
    pub start_proxy: Option<StartProxyFn>,
    /// Optional proxy-control closure (stop).
    pub stop_proxy: Option<StopProxyFn>,
    /// Optional proxy-status closure.
    pub proxy_status: Option<ProxyStatusFn>,
}

impl AppState {
    /// Build a new `AppState`. `start_proxy` / `stop_proxy` /
    /// `proxy_status` may be `None` (the proxy routes return
    /// 503 in that case — see the handlers).
    pub fn new(
        store: Arc<Engine>,
        ws: WsHub,
        start_proxy: Option<StartProxyFn>,
        stop_proxy: Option<StopProxyFn>,
        proxy_status: Option<ProxyStatusFn>,
    ) -> Self {
        Self {
            store,
            ws,
            start_proxy,
            stop_proxy,
            proxy_status,
        }
    }
}
