//! Security tests (4 cases).
//!
//! Per the v0.3.42 mode-B pre-trim rule, this file has
//! exactly 4 test cases. Do NOT exceed.
//!
//! These are the canary tests for the threat model:
//! loopback-only by default, no auth routes, no admin
//! routes, no CORS wildcard. The full feature (with
//! auth + TLS + remote) is exercised in `tests/auth.rs`
//! + `tests/tls.rs`.

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use bk_engine::Engine;
use bk_server::Server;
use tempfile::TempDir;
use tower::ServiceExt;

fn make_server() -> (Server, TempDir) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let engine = Arc::new(Engine::new(tmp.path().to_path_buf()).expect("engine"));
    let ui_dist = tmp.path().join("ui");
    std::fs::create_dir_all(&ui_dist).expect("ui dist");
    let server = Server::new(engine, ui_dist);
    (server, tmp)
}

#[test]
fn binding_to_non_loopback_without_remote_is_rejected() {
    // Threat-model check: the server refuses to bind to
    // a non-loopback address when --allow-remote is OFF.
    // This is the canary test for "anyone with network
    // access to the port can hit the API".
    let (mut server, _tmp) = make_server();
    server = server.with_bind_addr(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)));
    let err = server
        .validate()
        .expect_err("non-loopback + no remote must be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("loopback") || msg.contains("127.0.0.1"),
        "error must mention the loopback constraint; got: {msg}"
    );
}

#[tokio::test]
async fn no_auth_routes_exist() {
    // Threat-model check: there are no `/api/auth/*`
    // routes. The loopback mode has no auth (anyone on
    // the host can hit the API), and the routes start
    // with `/api/health`, `/api/projects`, etc.
    let (server, _tmp) = make_server();
    let app = server.build_router();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/auth/login")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status().as_u16(),
        404,
        "no /api/auth/* routes must exist"
    );
}

#[tokio::test]
async fn no_admin_routes_exist() {
    // Threat-model check: no `/api/admin/*` routes.
    let (server, _tmp) = make_server();
    let app = server.build_router();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/admin/reload")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status().as_u16(),
        404,
        "no /api/admin/* routes must exist"
    );
}

#[tokio::test]
async fn cors_does_not_allow_wildcard() {
    // Threat-model check: the CORS layer never returns
    // `Access-Control-Allow-Origin: *`. We assert this
    // by sending a request with an `Origin` header
    // that's clearly not allowed (a different scheme)
    // and confirming the response does not contain the
    // wildcard.
    let (server, _tmp) = make_server();
    let app = server.build_router();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .header("Origin", "http://evil.example.com")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // CORS is permissive for same-origin (no Access-
    // Control-Allow-Origin header is the default), but
    // for cross-origin without an explicit allow, the
    // header must NOT be `*`.
    let allow_origin = resp
        .headers()
        .get("access-control-allow-origin")
        .map(|v| v.to_str().unwrap_or(""));
    assert_ne!(
        allow_origin,
        Some("*"),
        "CORS must not return `Access-Control-Allow-Origin: *`"
    );
}
