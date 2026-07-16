//! Dynamic root CA + per-host leaf signing.
//!
//! §3.2 of the design contract. The MITM proxy signs a fresh TLS
//! leaf certificate for every SNI it intercepts, with this crate's
//! `RootCa` as the signer. The root is generated the first time
//! Talon starts (the "install" event), persisted to `<config_dir>/ca/`,
//! and never rotated.
//!
//! ## On-disk layout
//!
//! ```text
//! <config_dir>/ca/
//!   ca.crt.pem        # self-signed root cert, PEM
//!   ca.key.pem        # root key, PEM
//!   ca.fingerprint    # SHA-256 fingerprint, hex with colons
//!   ca.meta.toml      # serial_number + not_before_unix + not_after_unix
//! ```
//!
//! ## Why the on-disk cert is the source of truth for the DER
//!
//! ECDSA signatures (which is what `rcgen` produces by default) are
//! non-deterministic — every call to `sign()` uses a fresh random
//! nonce. So if we re-derived the root from `(key, params)` at load
//! time, the resulting DER would differ from the persisted DER, and
//! the fingerprint would change every run.
//!
//! Solution: persist the cert PEM as bytes and use that as the
//! source of truth for the trust chain. The `Issuer` we build at
//! load time is for signing *leaves*; it doesn't need to match the
//! signature of the persisted root cert.
//!
//! ## Why we don't use `Issuer::from_ca_cert_pem`
//!
//! `rcgen 0.14` gates that method behind a non-default `x509-parser`
//! feature, so reconstructing a `Certificate` object from a saved
//! PEM at runtime requires us to bring in `x509-parser` and re-parse.
//! Cheaper path: persist the *parameters* (serial + validity window)
//! alongside the key, and parse the cert PEM as DER via
//! `rustls-pemfile`.

use std::path::{Path, PathBuf};

use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa, Issuer, KeyPair,
    KeyUsagePurpose, SerialNumber,
};
use rustls_pemfile::Item;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use time::{Duration as TimeDuration, OffsetDateTime};

/// Errors from CA load / leaf sign operations.
#[derive(Debug, Error)]
pub enum CaError {
    /// The CA directory could not be created.
    #[error("failed to create CA directory {path}: {source}", path = .path.display())]
    CreateDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// A CA file could not be read.
    #[error("failed to read CA file {path}: {source}", path = .path.display())]
    ReadFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// A CA file could not be written.
    #[error("failed to write CA file {path}: {source}", path = .path.display())]
    WriteFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// A CA file could not be parsed (bad PEM, bad TOML, bad cert).
    #[error("failed to parse CA file {path}: {message}", path = .path.display())]
    ParseFile { path: PathBuf, message: String },
    /// `rcgen` failed to generate a cert.
    #[error("rcgen error: {0}")]
    Rcgen(String),
}

impl From<rcgen::Error> for CaError {
    fn from(e: rcgen::Error) -> Self {
        CaError::Rcgen(e.to_string())
    }
}

impl From<toml::de::Error> for CaError {
    fn from(e: toml::de::Error) -> Self {
        CaError::ParseFile {
            path: PathBuf::from("<meta.toml>"),
            message: e.to_string(),
        }
    }
}

impl From<toml::ser::Error> for CaError {
    fn from(e: toml::ser::Error) -> Self {
        CaError::ParseFile {
            path: PathBuf::from("<meta.toml>"),
            message: e.to_string(),
        }
    }
}

/// On-disk metadata that lets us re-derive the *issuer* (params +
/// key) at load time. Without this, we couldn't sign new leaves
/// from the persisted root — we'd have to keep the whole `Issuer`
/// in memory across runs, which means persisting the private key
/// (which we already do, in `ca.key.pem`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct RootCaMeta {
    /// The root cert's serial number, as a u64.
    serial: u64,
    /// Validity window start, as seconds since the Unix epoch.
    not_before_unix: i64,
    /// Validity window end, as seconds since the Unix epoch.
    not_after_unix: i64,
}

/// A persistent, on-disk root CA.
#[derive(Debug)]
pub struct RootCa {
    /// The root cert DER (loaded from `ca.crt.pem`). Used by §3.3
    /// to install the trust chain in rustls. The DER on disk is
    /// canonical — we never re-sign the root after first generation,
    /// so the persisted bytes are what the proxy serves as the trust
    /// anchor.
    root_cert_der: Vec<u8>,
    /// The issuer (params + key) used to sign leaves.
    issuer: Issuer<'static, KeyPair>,
    /// SHA-256 fingerprint, hex with colons (e.g. `"ab:cd:...:ef"`).
    fingerprint: String,
}

impl RootCa {
    /// The canonical CA directory: `<config_dir>/ca`.
    pub fn ca_dir(config_dir: &Path) -> PathBuf {
        config_dir.join("ca")
    }

    /// Load the root CA from `<config_dir>/ca/`, or generate + persist
    /// a fresh one if any of the expected files is missing.
    pub fn load_or_create(config_dir: &Path) -> Result<Self, CaError> {
        let dir = Self::ca_dir(config_dir);
        let cert_path = dir.join("ca.crt.pem");
        let key_path = dir.join("ca.key.pem");
        let fp_path = dir.join("ca.fingerprint");
        let meta_path = dir.join("ca.meta.toml");

        if cert_path.exists() && key_path.exists() && fp_path.exists() && meta_path.exists() {
            return Self::load_from_disk(&cert_path, &key_path, &meta_path, &fp_path);
        }

        Self::generate_and_persist(
            config_dir, &dir, &cert_path, &key_path, &fp_path, &meta_path,
        )
    }

    /// Reload from disk: load the saved cert PEM, parse the key,
    /// reconstruct the issuer from the persisted meta, verify the
    /// fingerprint file matches the on-disk DER.
    fn load_from_disk(
        cert_path: &Path,
        key_path: &Path,
        meta_path: &Path,
        fp_path: &Path,
    ) -> Result<Self, CaError> {
        let cert_pem = std::fs::read_to_string(cert_path).map_err(|e| CaError::ReadFile {
            path: cert_path.to_path_buf(),
            source: e,
        })?;
        let key_pem = std::fs::read_to_string(key_path).map_err(|e| CaError::ReadFile {
            path: key_path.to_path_buf(),
            source: e,
        })?;
        let meta_str = std::fs::read_to_string(meta_path).map_err(|e| CaError::ReadFile {
            path: meta_path.to_path_buf(),
            source: e,
        })?;
        let meta: RootCaMeta = toml::from_str(&meta_str)?;

        let key = KeyPair::from_pem(&key_pem).map_err(|e| CaError::ParseFile {
            path: key_path.to_path_buf(),
            message: e.to_string(),
        })?;

        let params = build_root_params(
            meta.serial,
            meta.not_before_unix,
            meta.not_after_unix,
            meta_path,
        )?;

        // Parse the on-disk cert PEM to DER. This is what we hand
        // to rustls as the trust anchor.
        let root_cert_der = parse_pem_to_der(&cert_pem, cert_path)?;

        // Sanity check: the on-disk fingerprint must match what we
        // compute from the on-disk DER. If they diverge, the
        // on-disk files are inconsistent and we should bail.
        let derived_fp = fingerprint_from_der(&root_cert_der);
        let stored_fp = std::fs::read_to_string(fp_path)
            .map_err(|e| CaError::ReadFile {
                path: fp_path.to_path_buf(),
                source: e,
            })?
            .trim()
            .to_string();
        if derived_fp != stored_fp {
            return Err(CaError::ParseFile {
                path: fp_path.to_path_buf(),
                message: format!(
                    "fingerprint mismatch: on-disk says {stored_fp}, DER-derived says {derived_fp}; \
                     CA files are inconsistent; delete the entire <config_dir>/ca/ and restart"
                ),
            });
        }

        let issuer = Issuer::new(params, key);
        Ok(Self {
            root_cert_der,
            issuer,
            fingerprint: stored_fp,
        })
    }

    /// Generate a fresh root CA and persist cert, key, fingerprint,
    /// and meta (serial + validity window) atomically.
    #[allow(clippy::too_many_arguments)]
    fn generate_and_persist(
        _config_dir: &Path,
        dir: &Path,
        cert_path: &Path,
        key_path: &Path,
        fp_path: &Path,
        meta_path: &Path,
    ) -> Result<Self, CaError> {
        // Ensure the directory exists with mode 0700.
        std::fs::create_dir_all(dir).map_err(|e| CaError::CreateDir {
            path: dir.to_path_buf(),
            source: e,
        })?;
        set_dir_mode_0700(dir);

        let key = KeyPair::generate()?;

        // 10 years — root CA lifetime. Round to whole seconds so
        // the persisted meta is stable.
        let not_before_unix = OffsetDateTime::now_utc().unix_timestamp();
        let not_after_unix = not_before_unix + TimeDuration::days(3650).whole_seconds();
        let serial = 1u64;

        let params = build_root_params(serial, not_before_unix, not_after_unix, meta_path)?;
        let cert = params.self_signed(&key)?;
        let cert_pem = cert.pem();
        let root_cert_der = cert.der().to_vec();
        let fingerprint = compute_fingerprint(&cert)?;

        let meta = RootCaMeta {
            serial,
            not_before_unix,
            not_after_unix,
        };
        let meta_str = toml::to_string_pretty(&meta)?;

        write_atomic(cert_path, cert_pem.as_bytes())?;
        write_atomic(key_path, key.serialize_pem().as_bytes())?;
        write_atomic(fp_path, fingerprint.as_bytes())?;
        write_atomic(meta_path, meta_str.as_bytes())?;

        let issuer = Issuer::new(params, key);
        Ok(Self {
            root_cert_der,
            issuer,
            fingerprint,
        })
    }

    /// The SHA-256 fingerprint of the root cert, hex with colons.
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    /// The root cert's DER bytes. §3.3 uses this to install the
    /// trust chain in rustls; the §3.2 test uses it to assert that
    /// a freshly-signed leaf chains back to the root.
    pub fn root_cert_der(&self) -> Vec<u8> {
        self.root_cert_der.clone()
    }

    /// Sign a per-host leaf certificate for the given SNI.
    ///
    /// Returns `(cert_der, key_der)` — the bytes the caller should
    /// hand to rustls to terminate the upstream TLS connection as if
    /// the user had visited `https://{sni}/` through Talon.
    pub fn sign_leaf(&self, sni: &str) -> Result<(Vec<u8>, Vec<u8>), CaError> {
        let leaf_key = KeyPair::generate()?;

        let mut params = CertificateParams::new(vec![sni.to_string()])?;
        params.distinguished_name = DistinguishedName::new();
        params.distinguished_name.push(DnType::CommonName, sni);
        params.is_ca = IsCa::NoCa;
        params.key_usages = vec![
            KeyUsagePurpose::DigitalSignature,
            KeyUsagePurpose::KeyEncipherment,
        ];
        params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
        // 1 year. Browsers tolerate up to 398 days; we give ourselves
        // some headroom for clock skew and re-sign latency.
        params.not_before = OffsetDateTime::now_utc();
        params.not_after = params.not_before + TimeDuration::days(365);

        let leaf = params.signed_by(&leaf_key, &self.issuer)?;

        // DER out for both — rustls consumes the cert as
        // `CertificateDer<'static>` and the key as `PrivateKeyDer<'static>`.
        let cert_der: Vec<u8> = leaf.der().to_vec();
        let key_der: Vec<u8> = leaf_key.serialized_der().to_vec();

        Ok((cert_der, key_der))
    }
}

/// Build the canonical `CertificateParams` for a Talon root CA with
/// the given serial and validity window. Pulled out so load + generate
/// share the field set.
fn build_root_params(
    serial: u64,
    not_before_unix: i64,
    not_after_unix: i64,
    meta_path_for_err: &Path,
) -> Result<CertificateParams, CaError> {
    let mut params = CertificateParams::default();
    params.distinguished_name = DistinguishedName::new();
    params
        .distinguished_name
        .push(DnType::CommonName, "Talon Root CA");
    params
        .distinguished_name
        .push(DnType::OrganizationName, "Talon");
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    params.serial_number = Some(SerialNumber::from(serial));
    params.not_before =
        OffsetDateTime::from_unix_timestamp(not_before_unix).map_err(|e| CaError::ParseFile {
            path: meta_path_for_err.to_path_buf(),
            message: format!("bad not_before_unix: {e}"),
        })?;
    params.not_after =
        OffsetDateTime::from_unix_timestamp(not_after_unix).map_err(|e| CaError::ParseFile {
            path: meta_path_for_err.to_path_buf(),
            message: format!("bad not_after_unix: {e}"),
        })?;
    Ok(params)
}

/// Parse a PEM-encoded X.509 certificate to DER bytes.
fn parse_pem_to_der(pem: &str, src_path: &Path) -> Result<Vec<u8>, CaError> {
    // `rustls-pemfile` reads one item at a time. We only care about
    // the first X.509 certificate in the file.
    match rustls_pemfile::read_one_from_slice(pem.as_bytes()) {
        Ok(Some((Item::X509Certificate(der), _rest))) => Ok(der.to_vec()),
        Ok(Some((other, _rest))) => Err(CaError::ParseFile {
            path: src_path.to_path_buf(),
            message: format!("expected X.509 certificate, found {other:?}"),
        }),
        Ok(None) => Err(CaError::ParseFile {
            path: src_path.to_path_buf(),
            message: "no PEM block found".to_string(),
        }),
        Err(e) => Err(CaError::ParseFile {
            path: src_path.to_path_buf(),
            message: format!("{e:?}"),
        }),
    }
}

/// Compute the SHA-256 fingerprint of a DER cert, formatted as
/// `"AA:BB:CC:...:FF"` (uppercase hex, colon-separated).
fn fingerprint_from_der(der: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(der);
    let digest = hasher.finalize();
    format_hex_colon(&digest)
}

/// Compute the SHA-256 fingerprint of an `rcgen::Certificate`. Used
/// at generate time before the cert is in the on-disk format.
fn compute_fingerprint(cert: &rcgen::Certificate) -> Result<String, CaError> {
    let der = cert.der();
    let mut hasher = Sha256::new();
    hasher.update(der.as_ref());
    let digest = hasher.finalize();
    Ok(format_hex_colon(&digest))
}

/// Format a byte slice as uppercase hex with colons between every
/// byte. `"abcdef"` → `"AB:CD:EF"`.
fn format_hex_colon(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 {
            out.push(':');
        }
        out.push_str(&format!("{:02X}", b));
    }
    out
}

/// Write `data` to `path` atomically: write to `path.tmp`, then
/// rename. Sets mode 0600 on the temp before the rename.
fn write_atomic(path: &Path, data: &[u8]) -> Result<(), CaError> {
    let tmp = path.with_extension(
        path.extension()
            .map(|e| format!("{}.tmp", e.to_string_lossy()))
            .unwrap_or_else(|| "tmp".to_string()),
    );
    std::fs::write(&tmp, data).map_err(|e| CaError::WriteFile {
        path: tmp.clone(),
        source: e,
    })?;
    set_file_mode_0600(&tmp);
    std::fs::rename(&tmp, path).map_err(|e| CaError::WriteFile {
        path: path.to_path_buf(),
        source: e,
    })?;
    Ok(())
}

#[cfg(unix)]
fn set_file_mode_0600(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path) {
        let mut perms = meta.permissions();
        perms.set_mode(0o600);
        let _ = std::fs::set_permissions(path, perms);
    }
}

#[cfg(not(unix))]
fn set_file_mode_0600(_path: &Path) {
    // On non-Unix we don't try to enforce perms — Windows uses ACLs
    // and a different API surface. Best-effort only.
}

#[cfg(unix)]
fn set_dir_mode_0700(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path) {
        let mut perms = meta.permissions();
        perms.set_mode(0o700);
        let _ = std::fs::set_permissions(path, perms);
    }
}

#[cfg(not(unix))]
fn set_dir_mode_0700(_path: &Path) {
    // See `set_file_mode_0600`.
}
