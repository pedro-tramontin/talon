//! Auth tests (4 cases).
//!
//! Per the v0.3.42 mode-B pre-trim rule, this file has
//! exactly 4 test cases. Do NOT exceed.
//!
//! The auth layer:
//! - rejects `--allow-remote` + missing `--tls-cert` at
//!   startup (case 1),
//! - generates a token on first launch and stores it
//!   (case 2),
//! - rejects requests with the wrong token (case 3),
//! - accepts requests with the correct token (case 4)
//!   using `subtle::ConstantTimeEq` for the comparison.

use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use bk_engine::Engine;
use bk_server::{AuthToken, Server};
use tempfile::TempDir;
use tower::ServiceExt;

fn make_server(tmp: &TempDir) -> Server {
    let engine = Arc::new(Engine::new(tmp.path().to_path_buf()).expect("engine"));
    let ui_dist = tmp.path().join("ui");
    std::fs::create_dir_all(&ui_dist).expect("ui dist");
    Server::new(engine, ui_dist)
}

#[test]
fn allow_remote_without_tls_is_rejected() {
    // --allow-remote is ON but --tls-cert is missing.
    // The server must refuse to start with a clear
    // error.
    let tmp = tempfile::tempdir().expect("tempdir");
    let server = make_server(&tmp)
        .with_allow_remote(true)
        .with_bind_addr(IpAddr::from([0, 0, 0, 0]));
    let err = server
        .validate()
        .expect_err("allow-remote + no TLS must be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("tls") || msg.contains("TLS") || msg.contains("cert"),
        "error must mention the TLS constraint; got: {msg}"
    );
}

#[test]
fn token_generated_on_first_launch_is_stored() {
    // The token is generated on first launch and
    // stored at the configured path with mode 0600.
    let tmp = tempfile::tempdir().expect("tempdir");
    let token_path: PathBuf = tmp.path().join("auth-token");
    let token = AuthToken::generate();
    token.save(&token_path).expect("save token");
    assert!(token_path.exists(), "token file must exist after save");
    let reloaded = AuthToken::load(&token_path).expect("reload token");
    assert_eq!(
        token.to_hex(),
        reloaded.to_hex(),
        "reloaded token must match generated"
    );
}

#[tokio::test]
async fn auth_layer_rejects_wrong_token() {
    // The auth layer returns 401 on a wrong token.
    let tmp = tempfile::tempdir().expect("tempdir");
    let server = make_server(&tmp)
        .with_allow_remote(true)
        .with_bind_addr(IpAddr::from([0, 0, 0, 0]))
        .with_tls(tmp.path().join("cert.pem"), tmp.path().join("key.pem"))
        .with_auth_token(Arc::new(AuthToken::generate()));
    let app = server.build_router();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .header("Authorization", "Bearer wrong-token-aaaa")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn auth_layer_accepts_correct_token() {
    // The auth layer returns 200 on the correct token.
    // The comparison uses `subtle::ConstantTimeEq`
    // (the structural check is in the `AuthToken::matches`
    // implementation; the test pins the behavior).
    let tmp = tempfile::tempdir().expect("tempdir");
    let token = AuthToken::generate();
    let server = make_server(&tmp)
        .with_allow_remote(true)
        .with_bind_addr(IpAddr::from([0, 0, 0, 0]))
        .with_tls(tmp.path().join("cert.pem"), tmp.path().join("key.pem"))
        .with_auth_token(Arc::new(token.clone()));
    let app = server.build_router();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .header("Authorization", format!("Bearer {}", token.to_hex()))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}
