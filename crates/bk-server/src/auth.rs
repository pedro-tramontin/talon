//! Per-install auth token + axum middleware.
//!
//! The token is a 32-byte secret, hex-encoded for the
//! `Authorization: Bearer <token>` header (64 hex chars).
//! On-disk it's the raw 32 bytes at the configured path
//! (mode 0600 on Unix). The hex form is what the user sees
//! from `talon token`; the raw bytes are what the server
//! compares against.
//!
//! ## Constant-time comparison
//!
//! The token comparison uses [`subtle::ConstantTimeEq`] so a
//! network attacker cannot time-side-channel the byte
//! differences. The `matches` helper below wraps the
//! constant-time check with a length-equality short-circuit
//! (the length is not secret — both the on-wire and
//! on-disk forms have a fixed length — and the `ct_eq`
//! requires equal-length slices).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use rand::RngCore;
use subtle::ConstantTimeEq;

/// Number of random bytes in the auth token. 32 bytes = 256
/// bits of entropy — overkill for local auth but matches the
/// "this is a per-install secret, not a password" model.
const TOKEN_BYTES: usize = 32;

/// A 32-byte auth token. Cheap to clone (it's a 32-byte
/// array); wrap in `Arc` for the axum state.
#[derive(Clone)]
pub struct AuthToken {
    bytes: [u8; TOKEN_BYTES],
}

impl AuthToken {
    /// Generate a new random token. Uses
    /// [`rand::thread_rng`] (CSPRNG).
    pub fn generate() -> Self {
        let mut bytes = [0u8; TOKEN_BYTES];
        rand::thread_rng().fill_bytes(&mut bytes);
        Self { bytes }
    }

    /// Load a token from a file on disk. The file's contents
    /// are the raw 32 bytes (not hex). The file must be
    /// exactly `TOKEN_BYTES` long.
    pub fn load(path: &Path) -> Result<Self, crate::ServerError> {
        let bytes = std::fs::read(path).map_err(|e| {
            crate::ServerError::AuthTokenUnreadable(path.to_path_buf(), e.to_string())
        })?;
        if bytes.len() != TOKEN_BYTES {
            return Err(crate::ServerError::AuthTokenUnreadable(
                path.to_path_buf(),
                format!(
                    "expected {} bytes, got {} (the file is corrupted or was written by a different version)",
                    TOKEN_BYTES,
                    bytes.len()
                ),
            ));
        }
        let mut arr = [0u8; TOKEN_BYTES];
        arr.copy_from_slice(&bytes);
        Ok(Self { bytes: arr })
    }

    /// Save the token to a file. Sets the file mode to
    /// 0600 on Unix (owner read/write only).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, self.bytes.as_ref())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(path, perms)?;
        }
        Ok(())
    }

    /// The hex-encoded form (64 chars). This is what the
    /// user copies into their `Authorization: Bearer` header
    /// and what the WS subprotocol carries.
    pub fn to_hex(&self) -> String {
        hex_encode(&self.bytes)
    }

    /// Constant-time comparison of an incoming token (in hex
    /// form) against this token. Returns `true` only on
    /// exact match. Used by the auth middleware + the WS
    /// upgrade handler.
    pub fn matches(&self, incoming_hex: &str) -> bool {
        // Reject obviously wrong lengths up-front (length
        // is not secret — both forms are 64 chars). The
        // actual byte comparison still uses ct_eq below.
        if incoming_hex.len() != self.to_hex().len() {
            return false;
        }
        // Decode the incoming hex to bytes. If the hex is
        // invalid, return false (we still do a constant-time
        // compare against the zero array so the timing
        // doesn't leak the "valid hex" vs "invalid hex"
        // distinction at scale — but a single 401 is fine
        // to short-circuit here; the attacker can't
        // enumerate based on this branch).
        let decoded = match hex_decode(incoming_hex) {
            Ok(b) => b,
            Err(_) => return false,
        };
        // subtle::ConstantTimeEq requires equal-length
        // slices. We've already checked the hex length
        // above, so the decoded length is also correct.
        self.bytes.ct_eq(&decoded).into()
    }
}

impl std::fmt::Debug for AuthToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print the bytes — even in debug builds.
        f.debug_struct("AuthToken")
            .field("bytes", &"<redacted>")
            .finish()
    }
}

/// Lowercase hex encoder. Avoids pulling in the `hex` crate
/// for one function.
fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

/// Lowercase hex decoder. Returns an error on invalid hex.
fn hex_decode(s: &str) -> Result<Vec<u8>, String> {
    if s.len() % 2 != 0 {
        return Err("odd length".to_string());
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    for i in (0..bytes.len()).step_by(2) {
        let hi = hex_nibble(bytes[i])?;
        let lo = hex_nibble(bytes[i + 1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn hex_nibble(c: u8) -> Result<u8, String> {
    match c {
        b'0'..=b'9' => Ok(c - b'0'),
        b'a'..=b'f' => Ok(c - b'a' + 10),
        b'A'..=b'F' => Ok(c - b'A' + 10),
        _ => Err(format!("invalid hex char: {c}")),
    }
}

/// axum middleware that enforces the `Authorization: Bearer
/// <token>` header on every request when a token is
/// configured. Returns 401 on missing or wrong tokens.
///
/// The middleware is a function (not a struct implementing
/// `tower::Layer`) because axum's `middleware::from_fn_with_state`
/// is the canonical pattern for stateful middlewares in
/// axum 0.7. The state is the `Arc<AuthToken>` (cheap to
/// clone for the extractor).
pub struct AuthLayer;

impl AuthLayer {
    /// The middleware function. Pass to
    /// `axum::middleware::from_fn_with_state(token, Self::middleware)`.
    pub async fn middleware(
        State(token): State<Arc<AuthToken>>,
        req: Request<Body>,
        next: Next,
    ) -> Response {
        // The `Authorization` header is the only accepted
        // form on plain HTTP. The WS upgrade uses the
        // `Sec-WebSocket-Protocol` subprotocol (browsers
        // can't set `Authorization` on WS upgrades).
        let header = req
            .headers()
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok());

        let valid = match header {
            Some(h) if h.starts_with("Bearer ") => {
                token.matches(h.trim_start_matches("Bearer ").trim())
            }
            _ => false,
        };

        if !valid {
            return (
                StatusCode::UNAUTHORIZED,
                [(
                    axum::http::header::WWW_AUTHENTICATE,
                    "Bearer realm=\"talon\"",
                )],
                "{\"error\":\"unauthorized\"}",
            )
                .into_response();
        }
        next.run(req).await
    }
}

/// Convenience: the path of the default auth-token file
/// (`~/.config/talon/auth-token` on Linux/macOS,
/// `%APPDATA%\talon\auth-token` on Windows). Mirrors the
/// `dirs` crate's `config_dir()` so the user finds the
/// token in the same place as the rest of the config.
pub fn default_auth_token_path() -> PathBuf {
    dirs::config_dir()
        .map(|d| d.join("talon").join("auth-token"))
        .unwrap_or_else(|| PathBuf::from("talon-auth-token"))
}
