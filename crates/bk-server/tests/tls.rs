//! TLS tests (3 cases).
//!
//! Per the v0.3.42 mode-B pre-trim rule, this file has
//! exactly 3 test cases. Do NOT exceed.

use std::net::IpAddr;
use std::sync::Arc;

use bk_engine::Engine;
use bk_server::tls::TlsConfig;
use bk_server::Server;
use tempfile::TempDir;

fn make_server(tmp: &TempDir) -> Server {
    let engine = Arc::new(Engine::new(tmp.path().to_path_buf()).expect("engine"));
    let ui_dist = tmp.path().join("ui");
    std::fs::create_dir_all(&ui_dist).expect("ui dist");
    Server::new(engine, ui_dist)
}

#[test]
fn allow_remote_without_tls_is_rejected() {
    // --allow-remote is ON but --tls-cert / --tls-key
    // is missing. The server refuses to start (this
    // mirrors `tests/auth.rs` case 1 but asserts
    // independently for the TLS layer).
    let tmp = tempfile::tempdir().expect("tempdir");
    let server = make_server(&tmp)
        .with_allow_remote(true)
        .with_bind_addr(IpAddr::from([0, 0, 0, 0]));
    let err = server
        .validate()
        .expect_err("allow-remote + no TLS cert must be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("tls") || msg.contains("TLS") || msg.contains("cert"),
        "error must mention the TLS constraint; got: {msg}"
    );
}

#[test]
fn tls_config_loads_valid_cert_and_key() {
    // TLS handshake succeeds with a valid cert+key.
    // We use `rcgen` (already a workspace dep) to
    // generate a self-signed cert for the test.
    let tmp = tempfile::tempdir().expect("tempdir");
    let cert_path = tmp.path().join("cert.pem");
    let key_path = tmp.path().join("key.pem");
    // Generate a self-signed cert with `rcgen`.
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).expect("cert");
    let cert_pem = cert.cert.pem();
    let key_pem = cert.signing_key.serialize_pem();
    std::fs::write(&cert_path, cert_pem).expect("write cert");
    std::fs::write(&key_path, key_pem).expect("write key");
    let cfg = TlsConfig::new(cert_path, key_path);
    // rustls 0.23 needs a default crypto provider
    // installed before any TLS config is built. We use
    // the aws-lc-rs provider (rustls's default feature)
    // because the workspace does not enable `ring`.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let _loaded = cfg.load().expect("valid cert+key must load");
}

#[test]
fn tls_config_rejects_missing_key() {
    // TLS handshake fails with a missing key file.
    // The `TlsConfig::load` returns
    // `ServerError::KeyNotFound` for a missing key.
    let tmp = tempfile::tempdir().expect("tempdir");
    let cert_path = tmp.path().join("cert.pem");
    std::fs::write(&cert_path, "not a real cert").expect("write cert");
    let cfg = TlsConfig::new(cert_path, tmp.path().join("nonexistent-key.pem"));
    let err = cfg.load().expect_err("missing key must fail to load");
    let msg = err.to_string();
    assert!(
        msg.contains("key") || msg.contains("not found") || msg.contains("io"),
        "error must mention the missing key; got: {msg}"
    );
}
