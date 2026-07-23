//! TLS configuration (the `rustls` server config builder +
//! the cert/key loading helpers).
//!
//! Only used in remote mode (`--allow-remote` + `--tls-cert`
//! + `--tls-key`). The loopback path is plain HTTP — no TLS.

use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::ServerConfig;
use rustls_pki_types::pem::PemObject;

/// The configured TLS material. The cert + key are loaded
/// from disk at startup (via [`TlsConfig::load`]) and the
/// resulting `ServerConfig` is what `tokio_rustls::TlsAcceptor`
/// uses.
#[derive(Debug, Clone)]
pub struct TlsConfig {
    /// The cert file path. Stored for diagnostics (the
    /// `validate` check).
    pub cert: PathBuf,
    /// The key file path. Stored for diagnostics.
    pub key: PathBuf,
}

impl TlsConfig {
    /// Build a new TLS config from cert + key paths. Does
    /// NOT load the files yet — call [`TlsConfig::load`] in
    /// the `run` path so a missing file surfaces as a clear
    /// error.
    pub fn new(cert: PathBuf, key: PathBuf) -> Self {
        Self { cert, key }
    }

    /// Load the cert + key from disk and build a
    /// `rustls::ServerConfig`. The cert file is PEM-encoded
    /// (a chain of one or more certs); the key file is
    /// PEM-encoded PKCS#8 (or RSA) — `rustls` accepts both.
    pub fn load(&self) -> Result<ServerConfig, crate::ServerError> {
        let cert_chain = load_certs(&self.cert)?;
        let key = load_key(&self.key)?;
        let cfg = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(cert_chain, key)?;
        Ok(cfg)
    }
}

/// Load a PEM-encoded cert chain from disk.
fn load_certs(path: &PathBuf) -> Result<Vec<CertificateDer<'static>>, crate::ServerError> {
    let file = File::open(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            crate::ServerError::CertNotFound(path.clone())
        } else {
            crate::ServerError::Io(e)
        }
    })?;
    let mut reader = BufReader::new(file);
    // Use `rustls-pki-types::pem` directly (the
    // `rustls-pemfile` crate is unmaintained as of
    // 2025-08 and the rustls team recommends
    // migrating to this in-tree API).
    let certs: Vec<CertificateDer<'static>> =
        rustls_pki_types::pem::PemObject::pem_reader_iter(&mut reader)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| {
                crate::ServerError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e))
            })?;
    if certs.is_empty() {
        return Err(crate::ServerError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "no certificates found in PEM file",
        )));
    }
    Ok(certs)
}

/// Load a PEM-encoded private key from disk.
fn load_key(path: &PathBuf) -> Result<PrivateKeyDer<'static>, crate::ServerError> {
    let file = File::open(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            crate::ServerError::KeyNotFound(path.clone())
        } else {
            crate::ServerError::Io(e)
        }
    })?;
    let mut reader = BufReader::new(file);
    // `rustls-pki-types::pem::PemObject::from_pem_reader`
    // reads the first private key it finds in the PEM
    // stream. Returns `Err(NoItemsFound)` if there is
    // no key, which we treat as an error.
    let key = PrivateKeyDer::from_pem_reader(&mut reader).map_err(|e| {
        crate::ServerError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    })?;
    Ok(key)
}
