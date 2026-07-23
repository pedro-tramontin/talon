//! SPA fallback tests (4 cases).
//!
//! Per the v0.3.42 mode-B pre-trim rule, this file has
//! exactly 4 test cases. Do NOT exceed.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use bk_engine::Engine;
use bk_server::Server;
use http_body_util::BodyExt;
use tempfile::TempDir;
use tower::ServiceExt;

/// Build a Server with a `ui_dist` containing a
/// `index.html` (with `<div id="root">`) + an `assets/`
/// subdir. The fixtures exercise the SPA fallback.
fn make_server() -> (Server, TempDir) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let ui_dist = tmp.path().join("ui");
    std::fs::create_dir_all(ui_dist.join("assets")).expect("assets");
    std::fs::write(
        ui_dist.join("index.html"),
        r#"<!DOCTYPE html><html><body><div id="root"></div></body></html>"#,
    )
    .expect("index.html");
    std::fs::write(
        ui_dist.join("assets").join("index-abc123.js"),
        "console.log('hello');",
    )
    .expect("asset");
    let engine = Arc::new(Engine::new(tmp.path().to_path_buf()).expect("engine"));
    let server = Server::new(engine, ui_dist);
    (server, tmp)
}

#[tokio::test]
async fn get_root_serves_index_html() {
    let (server, _tmp) = make_server();
    let app = server.build_router();
    let resp = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap_or(""))
        .unwrap_or("");
    assert!(
        ct.contains("text/html"),
        "GET / must serve text/html; got {ct}"
    );
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let s = std::str::from_utf8(&body).unwrap();
    assert!(
        s.contains(r#"<div id="root">"#),
        "body must contain <div id=\"root\">"
    );
}

#[tokio::test]
async fn get_capture_falls_back_to_index_html() {
    // The React router takes over from any unmatched
    // path; the fallback must serve `index.html`.
    let (server, _tmp) = make_server();
    let app = server.build_router();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/capture")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let s = std::str::from_utf8(&body).unwrap();
    assert!(s.contains(r#"<div id="root">"#));
}

#[tokio::test]
async fn get_assets_serves_javascript() {
    let (server, _tmp) = make_server();
    let app = server.build_router();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/assets/index-abc123.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let s = std::str::from_utf8(&body).unwrap();
    assert!(s.contains("console.log"), "asset body must contain the JS");
}

#[tokio::test]
async fn get_nonexistent_returns_404() {
    // A request for a missing asset (not in the SPA
    // fallback path) returns 404, NOT index.html. The
    // fallback only applies to non-asset paths.
    let (server, _tmp) = make_server();
    let app = server.build_router();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/assets/nonexistent.png")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
