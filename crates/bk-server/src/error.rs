//! Error types for `bk-server`.

use std::path::PathBuf;

use thiserror::Error;

/// Errors that can occur when constructing, configuring, or
/// running the `bk-server`.
#[derive(Debug, Error)]
pub enum ServerError {
    /// I/O error (binding the socket, reading cert/key files,
    /// reading the auth token file, etc.).
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// axum / hyper error (rare — most of the axum errors are
    /// caught and converted to HTTP responses).
    #[error("hyper error: {0}")]
    Hyper(#[from] hyper::Error),

    /// TLS configuration error (cert/key missing, mismatched,
    /// or invalid).
    #[error("tls error: {0}")]
    Tls(#[from] rustls::Error),

    /// The `--allow-remote` flag was set but `--tls-cert` was
    /// not. Refusing to start a "remote" server without TLS
    /// would send auth tokens in cleartext — the threat model
    /// forbids it.
    #[error("--allow-remote requires --tls-cert and --tls-key (HTTPS-only)")]
    MissingTlsCert,

    /// The `--allow-remote` flag was set but `--tls-key` was
    /// not (paired with [`ServerError::MissingTlsCert`]).
    #[error("--allow-remote requires --tls-cert and --tls-key (HTTPS-only)")]
    MissingTlsKey,

    /// The server is in remote mode but no auth token has been
    /// configured. The token is the gate; without it anyone
    /// with network access to the port can hit the API.
    #[error("--allow-remote requires an auth token (use --auth-token-path)")]
    AuthTokenRequired,

    /// The server was configured to bind to a non-loopback
    /// address without `--allow-remote`. This is a defensive
    /// refusal — even if the user explicitly tried to set the
    /// bind address, the threat model says "loopback unless
    /// you've turned on remote mode + auth + TLS."
    #[error(
        "browser mode binds to 127.0.0.1 only; use --allow-remote + --tls-cert + --tls-key + --auth-token-path for remote access, \
         or use an SSH tunnel for non-TLS remote access"
    )]
    NonLoopbackWithoutRemote,

    /// The auth token file exists but could not be read or
    /// parsed.
    #[error("auth token at {0} is unreadable: {1}")]
    AuthTokenUnreadable(PathBuf, String),

    /// mDNS registration failed. Non-fatal: the server logs a
    /// warning and continues without discovery. Surfaced as an
    /// error variant so tests can assert the failure mode.
    #[error("mDNS registration failed: {0}")]
    Mdns(String),

    /// The configured TLS cert file was not found.
    #[error("tls cert file not found: {0}")]
    CertNotFound(PathBuf),

    /// The configured TLS key file was not found.
    #[error("tls key file not found: {0}")]
    KeyNotFound(PathBuf),
}
