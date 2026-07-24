//! The axum router — the API route table + the SPA fallback.

use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use tower::util::ServiceExt;
use tower_http::services::ServeDir;

use crate::handlers;
use crate::state::AppState;

/// All the API routes. The auth middleware is applied in
/// [`crate::Server::build_router`] (so the test paths can
/// construct a router without a token).
///
/// Returns a `Router<AppState>` that the caller attaches
/// to the outer router via `nest("/api", api)` (so the
/// `/api` prefix is added once, by the outer router).
/// The routes below are path-relative to the
/// `nest("/api", ...)` call site.
pub fn api_routes() -> Router<AppState> {
    Router::new()
        // Health
        .route("/health", get(handlers::health))
        // Project management
        .route("/projects", get(handlers::list_projects))
        // Exchanges
        .route(
            "/exchanges",
            get(handlers::list_exchanges).post(handlers::search_exchanges),
        )
        .route(
            "/exchanges/:id",
            get(handlers::get_exchange).delete(handlers::delete_exchange),
        )
        // Proxy control
        .route("/proxy/start", post(handlers::start_proxy))
        .route("/proxy/stop", post(handlers::stop_proxy))
        .route("/proxy/status", get(handlers::proxy_status))
        // Scope rules (Phase 6)
        .route("/scope/rules", get(handlers::list_scope_rules))
        .route("/scope/rules/add", post(handlers::add_scope_rule))
        .route("/scope/rules/remove", post(handlers::remove_scope_rule))
        // Match & replace rules (Phase 6)
        .route(
            "/scope/match-replace",
            get(handlers::list_match_replace_rules),
        )
        .route(
            "/scope/match-replace/add",
            post(handlers::add_match_replace_rule),
        )
        .route(
            "/scope/match-replace/remove",
            post(handlers::remove_match_replace_rule),
        )
}

/// The SPA fallback closure. Returns `index.html` for any
/// path that doesn't match a file in `ui_dist`. Wrapped in
/// `Arc` so it can be cloned into the router's `fallback`
/// handler (which is `Fn`-based and needs the captured
/// `PathBuf` to be cheaply cloneable).
pub fn spa_fallback(
    ui_dist: PathBuf,
) -> impl Fn(Request<Body>) -> futures::future::BoxFuture<'static, Response> + Clone {
    let ui_dist: Arc<PathBuf> = Arc::new(ui_dist);
    let index: Arc<PathBuf> = Arc::new(ui_dist.join("index.html"));
    let assets_dir: Arc<PathBuf> = Arc::new(ui_dist.join("assets"));
    move |req: Request<Body>| {
        let ui_dist = ui_dist.clone();
        let index = index.clone();
        let assets_dir = assets_dir.clone();
        let path = req.uri().path().to_string();
        Box::pin(async move {
            // First, try the assets directory. The
            // `ServeDir` looks for the file at
            // `assets_dir + request_path`, so we need
            // to strip the `/assets/` prefix from the
            // request path before serving.
            if path.starts_with("/assets/") {
                let stripped = path.trim_start_matches("/assets/");
                // Build a new request with the
                // stripped path. `ServeDir` then
                // serves `assets_dir + stripped`.
                let (mut parts, body) = req.into_parts();
                parts.uri = format!("/{stripped}").parse().unwrap();
                let new_req = Request::from_parts(parts, body);
                let resp = ServeDir::new(assets_dir.as_ref().clone())
                    .oneshot(new_req)
                    .await
                    .map(|r| r.into_response());
                match resp {
                    Ok(r) => return r,
                    Err(_) => {
                        return (StatusCode::NOT_FOUND, "asset not found").into_response();
                    }
                }
            }
            // Try a direct match against ui_dist.
            let direct = ServeDir::new(ui_dist.as_ref().clone())
                .oneshot(req)
                .await
                .ok();
            if let Some(resp) = direct {
                let status = resp.status();
                if status != StatusCode::NOT_FOUND {
                    return resp.into_response();
                }
            }
            // Fall back to index.html.
            match tokio::fs::read(index.as_ref()).await {
                Ok(bytes) => Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                    .body(Body::from(bytes))
                    .unwrap_or_else(|_| {
                        (StatusCode::INTERNAL_SERVER_ERROR, "build error").into_response()
                    }),
                Err(_) => (StatusCode::NOT_FOUND, "ui bundle not built").into_response(),
            }
        })
    }
}
