//! Proxy configuration.
//!
//! §3.1 reads `<config_dir>/proxy.toml` if it exists and falls back to
//! defaults otherwise. Full configuration support (TLS, upstream
//! overrides, scope rules) lands in later sections.

use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// On-disk representation of `<config_dir>/proxy.toml`.
///
/// Unknown fields are ignored so future sections can add fields
/// without breaking older binaries. Missing fields fall back to the
/// `Default` impl.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProxyConfigFile {
    /// Override the default `listener_addr`.
    pub listener_addr: Option<String>,
    /// Override the default `max_concurrent_connections`.
    pub max_concurrent_connections: Option<usize>,
    /// Override the default `upstream_timeout` (seconds).
    pub upstream_timeout_secs: Option<u64>,
    /// Override the default `log_level`.
    pub log_level: Option<String>,
}

/// Runtime proxy configuration.
///
/// Marked `#[non_exhaustive]` per the Phase 10 plugin-system
/// design contract (§5.1 item 1): v2 may add fields (e.g. a
/// `plugin_dir: Option<PathBuf>` for the v2 plugin loader, or
/// `wasm_fuel_limit: Option<u64>` for plugin sandbox tuning)
/// without breaking v1's `ProxyConfig { ... }` struct literals.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ProxyConfig {
    /// The address the TCP listener binds to.
    pub listener_addr: SocketAddr,
    /// Maximum number of in-flight proxied connections.
    pub max_concurrent_connections: usize,
    /// How long to wait for an upstream to respond.
    pub upstream_timeout: Duration,
    /// The tracing log level (`"trace"`, `"debug"`, `"info"`, ...).
    pub log_level: String,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            listener_addr: "127.0.0.1:8080"
                .parse()
                .expect("default listener addr is valid"),
            max_concurrent_connections: 256,
            upstream_timeout: Duration::from_secs(30),
            log_level: "info".to_string(),
        }
    }
}

impl ProxyConfig {
    /// Load a [`ProxyConfig`] from `<config_dir>/proxy.toml`.
    ///
    /// If the file does not exist, returns the defaults. If the file
    /// exists but cannot be read or parsed, returns an error.
    pub fn load_from_dir(config_dir: &Path) -> anyhow::Result<Self> {
        let path = config_dir.join("proxy.toml");
        if !path.exists() {
            return Ok(Self::default());
        }

        let body = std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?;
        let parsed: ProxyConfigFile = toml::from_str(&body)
            .map_err(|e| anyhow::anyhow!("parsing {}: {e}", path.display()))?;

        let mut out = Self::default();

        if let Some(addr) = parsed.listener_addr {
            out.listener_addr = addr
                .parse()
                .map_err(|e| anyhow::anyhow!("invalid listener_addr {addr:?}: {e}"))?;
        }
        if let Some(n) = parsed.max_concurrent_connections {
            if n == 0 {
                anyhow::bail!("max_concurrent_connections must be > 0, got {n}");
            }
            out.max_concurrent_connections = n;
        }
        if let Some(s) = parsed.upstream_timeout_secs {
            out.upstream_timeout = Duration::from_secs(s);
        }
        if let Some(lvl) = parsed.log_level {
            out.log_level = lvl;
        }

        Ok(out)
    }
}
