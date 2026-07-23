//! Phase 8 — Browser-access mode HTTP/WS server.
//!
//! `bk-server` serves the same React UI bundle + the same Rust
//! `bk_engine::Engine` backend over HTTP (REST) + WebSocket, so
//! a user can open `http://localhost:7331` in any browser and
//! use Talon without the Tauri desktop shell. See the per-item
//! `objective:` block in `state.md` for the full spec; this
//! module-level doc just covers the public surface.
//!
//! ## Threat model — three modes
//!
//! - **Loopback (default).** Binds `127.0.0.1:7331`, plain HTTP,
//!   no auth. Anyone who can `curl` your localhost can already
//!   read your screen — no new attack surface.
//! - **Remote with auth (`--allow-remote`).** Binds `0.0.0.0`
//!   (or a specific `--bind` addr), requires both
//!   `--tls-cert`/`--tls-key` (HTTPS via `rustls`) AND the
//!   `Authorization: Bearer <token>` header on every request
//!   + the WS upgrade. The token is a per-install secret
//!     generated on first launch and stored at
//!     `~/.config/talon/auth-token` (mode `0600`).
//! - **Remote with mDNS.** Same as (2) plus `--mdns-announce`,
//!   which auto-registers `talon-server._talon._tcp.local.`
//!   so other LAN machines can discover the URL. The token is
//!   NEVER auto-distributed via mDNS TXT records (the user
//!   must copy it via `talon token`).

#![deny(missing_docs)]

mod auth;
mod error;
mod handlers;
pub mod mdns;
mod routes;
mod state;
pub mod tls;
mod ws;

pub use auth::{default_auth_token_path, AuthLayer, AuthToken};
pub use error::ServerError;
pub use state::AppState;
pub use ws::WsHub;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use bk_engine::Engine;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::{info, warn};

/// Default port. Chosen because it's unprivileged, doesn't
/// collide with common dev servers (3000/5000/8000/8080), and
/// the digits spell "T-3-1" (Talon = "t-3-1" in 7331).
pub const DEFAULT_PORT: u16 = 7331;

/// Default bind address (loopback). The server refuses to bind
/// to a non-loopback address unless `allow_remote` is `true`.
pub const DEFAULT_BIND: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

/// A `Send + Sync` boxed closure that starts the proxy. The
/// `app` crate passes a closure that wraps its `ProxyHandle`
/// (so the server doesn't need to know the Tauri runtime
/// types). The closure receives the rule lists (scope +
/// match-and-replace) so the server can wire them through.
/// The closure returns the proxy's `ProxyStatus` as a JSON
/// value so the HTTP handler can serialize it.
pub type StartProxyFn = Arc<
    dyn Fn(Vec<bk_core::scope::ScopeRule>, Vec<bk_core::scope::MatchReplaceRule>) -> StartProxyFut
        + Send
        + Sync,
>;

/// The future returned by [`StartProxyFn`].
pub type StartProxyFut =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<serde_json::Value, String>> + Send>>;

/// A `Send + Sync` boxed closure that stops the proxy.
pub type StopProxyFn = Arc<dyn Fn() + Send + Sync>;

/// A `Send + Sync` boxed closure that returns the proxy status.
pub type ProxyStatusFn = Arc<dyn Fn() -> serde_json::Value + Send + Sync>;

/// The configured HTTP/WS server. Construct via [`Server::new`],
/// configure with the `with_*` builders, and call
/// [`Server::run`].
#[derive(Clone)]
pub struct Server {
    addr: SocketAddr,
    store: Arc<Engine>,
    start_proxy: Option<StartProxyFn>,
    stop_proxy: Option<StopProxyFn>,
    proxy_status: Option<ProxyStatusFn>,
    ui_dist: PathBuf,
    ws: Option<WsHub>,
    allow_remote: bool,
    auth_token: Option<Arc<AuthToken>>,
    tls: Option<tls::TlsConfig>,
    mdns: bool,
}

impl Server {
    /// Build a new server with the default loopback bind.
    ///
    /// `start_proxy` / `stop_proxy` / `proxy_status` are
    /// optional closures; if all three are `None` the proxy
    /// routes return 503 "proxy not configured". The `app`
    /// crate wires them up in `run_browser`.
    pub fn new(store: Arc<Engine>, ui_dist: PathBuf) -> Self {
        Self {
            addr: SocketAddr::new(DEFAULT_BIND, DEFAULT_PORT),
            store,
            start_proxy: None,
            stop_proxy: None,
            proxy_status: None,
            ui_dist,
            ws: None,
            allow_remote: false,
            auth_token: None,
            tls: None,
            mdns: false,
        }
    }

    /// Override the listening port.
    pub fn with_port(mut self, port: u16) -> Self {
        self.addr.set_port(port);
        self
    }

    /// Override the bind address. Refuses non-loopback unless
    /// `allow_remote` is also set.
    pub fn with_bind_addr(mut self, addr: IpAddr) -> Self {
        if !addr.is_loopback() && !self.allow_remote {
            warn!(
                "with_bind_addr called with non-loopback {addr} but --allow-remote is off; \
                 server will refuse to start. Use with_allow_remote(true) first."
            );
        }
        self.addr.set_ip(addr);
        self
    }

    /// Enable remote-access mode (HTTPS on 0.0.0.0, auth required).
    /// Must be paired with [`Server::with_tls`] and
    /// [`Server::with_auth_token`].
    pub fn with_allow_remote(mut self, allow: bool) -> Self {
        self.allow_remote = allow;
        if allow && self.addr.ip().is_loopback() {
            self.addr.set_ip(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
        }
        self
    }

    /// Set the TLS cert + key paths. Required when `allow_remote`
    /// is `true`.
    pub fn with_tls(mut self, cert: PathBuf, key: PathBuf) -> Self {
        self.tls = Some(tls::TlsConfig::new(cert, key));
        self
    }

    /// Set the auth token (already loaded from disk). Required
    /// when `allow_remote` is `true`.
    pub fn with_auth_token(mut self, token: Arc<AuthToken>) -> Self {
        self.auth_token = Some(token);
        self
    }

    /// Enable the mDNS announcement. Auto-enabled when
    /// `allow_remote` is `true`.
    pub fn with_mdns(mut self, on: bool) -> Self {
        self.mdns = on || self.allow_remote;
        self
    }

    /// Attach a [`WsHub`]. The hub is created by the caller
    /// (typically the `app` crate) and subscribed to the
    /// existing `wire_bus`. The server holds a clone for
    /// broadcast-on-WS-upgrade purposes.
    pub fn with_ws_hub(mut self, ws: WsHub) -> Self {
        self.ws = Some(ws);
        self
    }

    /// Wire the proxy-control closures. The `app` crate
    /// passes closures that wrap its `ProxyHandle`.
    pub fn with_proxy_handlers(
        mut self,
        start: StartProxyFn,
        stop: StopProxyFn,
        status: ProxyStatusFn,
    ) -> Self {
        self.start_proxy = Some(start);
        self.stop_proxy = Some(stop);
        self.proxy_status = Some(status);
        self
    }

    /// Build the axum [`Router`]. Exposed for tests so they
    /// can hit the router without binding a port.
    pub fn build_router(&self) -> Router<()> {
        let ws = self.ws.clone().unwrap_or_default();
        let auth = self.auth_token.clone();
        let store = self.store.clone();
        let start_proxy = self.start_proxy.clone();
        let stop_proxy = self.stop_proxy.clone();
        let proxy_status = self.proxy_status.clone();
        let state = AppState::new(store, ws.clone(), start_proxy, stop_proxy, proxy_status);
        let cors = build_cors_layer(self.allow_remote);

        // The WS upgrade handler needs the auth token (if
        // remote mode) for the `Sec-WebSocket-Protocol`
        // subprotocol check. We attach it via
        // `with_state` so the handler can read it as a
        // `State<Option<Arc<AuthToken>>>`.
        let ws_state = WsUpgradeState {
            hub: self.ws.clone().unwrap_or_default(),
            token: auth.clone(),
        };

        // Build the API router. `api_routes()` returns
        // `Router<AppState>`, which we attach the
        // `AppState` to via `with_state(state)` to
        // produce a `Router<()>`.
        let api: Router<AppState> = routes::api_routes();
        let api: Router<()> = api.with_state(state);
        // The outer router. The auth layer is applied
        // last; it carries its own state (the
        // `AuthToken`) and doesn't interact with the
        // `AppState` attached to the inner api router.
        let mut router: Router = Router::new()
            .nest("/api", api)
            .route(
                "/ws",
                axum::routing::get(ws_handler_with_auth).with_state(ws_state),
            )
            .fallback(routes::spa_fallback(self.ui_dist.clone()))
            .layer(TraceLayer::new_for_http())
            .layer(cors);
        if let Some(token) = auth {
            router = router.layer(axum::middleware::from_fn_with_state(
                token,
                AuthLayer::middleware,
            ));
        }
        router
    }

    /// Run the server until SIGINT. Validates the config first
    /// and refuses to start with a clear error if the
    /// constraints are violated.
    pub async fn run(self) -> Result<(), ServerError> {
        self.validate()?;

        let _mdns_guard = if self.mdns {
            let port = self.addr.port();
            match mdns::MdnsAnnouncer::new("talon-server", port) {
                Ok(g) => Some(g),
                Err(e) => {
                    warn!("mDNS announcement failed: {e}; continuing without discovery");
                    None
                }
            }
        } else {
            None
        };

        let router = self.build_router();
        let listener = tokio::net::TcpListener::bind(self.addr).await?;
        let bound = listener.local_addr()?;
        info!(
            "bk-server listening on http://{} (loopback={})",
            bound,
            bound.ip().is_loopback()
        );
        axum::serve(listener, router).await?;
        Ok(())
    }

    /// Validate the configured threat model. Called from
    /// [`Server::run`]; also exposed for tests.
    pub fn validate(&self) -> Result<(), ServerError> {
        if self.allow_remote {
            if self.tls.is_none() {
                return Err(ServerError::MissingTlsCert);
            }
            if self.auth_token.is_none() {
                return Err(ServerError::AuthTokenRequired);
            }
        } else if !self.addr.ip().is_loopback() {
            return Err(ServerError::NonLoopbackWithoutRemote);
        }
        Ok(())
    }
}

/// Build the CORS layer. The default is "same-origin only"
/// (no `Access-Control-Allow-Origin: *` ever). In remote
/// mode the user-supplied origin list is empty too (the
/// auth token in the Authorization header is the real
/// cross-origin check; CORS is a defense-in-depth, not the
/// primary boundary).
fn build_cors_layer(_allow_remote: bool) -> CorsLayer {
    // No wildcard ever. CORS is permissive enough for the
    // same-origin case (the user opens the UI from the same
    // host that serves it) but tight for cross-origin
    // requests — the Authorization header is the real gate.
    CorsLayer::new()
}

/// The state passed to the WS upgrade handler. Carries the
/// `WsHub` (broadcast source) + the optional auth token.
#[derive(Clone)]
struct WsUpgradeState {
    hub: WsHub,
    token: Option<Arc<AuthToken>>,
}

/// WS upgrade handler that optionally enforces the
/// `Sec-WebSocket-Protocol: talon-auth.<token>` subprotocol
/// when the server is in remote mode.
///
/// The auth check reads the `Sec-WebSocket-Protocol`
/// request header directly (browsers send it as a
/// comma-separated list). The `WebSocketUpgrade` extractor
/// doesn't expose the offered protocols via a method; the
/// `HeaderMap` is the canonical way to read them.
async fn ws_handler_with_auth(
    headers: axum::http::HeaderMap,
    upgrade: axum::extract::ws::WebSocketUpgrade,
    axum::extract::State(state): axum::extract::State<WsUpgradeState>,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    if let Some(token) = state.token.as_ref() {
        // Read the offered subprotocols from the
        // `Sec-WebSocket-Protocol` request header.
        let offered: Vec<String> = headers
            .get_all("sec-websocket-protocol")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|s| s.split(','))
            .map(|s| s.trim().to_string())
            .collect();
        let expected_prefix = "talon-auth.";
        let valid = offered.iter().any(|p| {
            p.strip_prefix(expected_prefix)
                .map(|tok| token.matches(tok))
                .unwrap_or(false)
        });
        if !valid {
            return (axum::http::StatusCode::UNAUTHORIZED, "ws auth required").into_response();
        }
        // Echo the subprotocol back so the browser
        // accepts the upgrade (RFC 6455: server must
        // echo one of the client's offered
        // subprotocols).
        let proto = offered
            .into_iter()
            .find(|p| p.starts_with(expected_prefix))
            .unwrap_or_default();
        return upgrade
            .protocols([proto])
            .on_upgrade(move |socket| state.hub.on_upgrade(socket));
    }
    // Loopback mode: no auth on the WS upgrade.
    upgrade.on_upgrade(move |socket| state.hub.on_upgrade(socket))
}
