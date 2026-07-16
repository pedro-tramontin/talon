//! `bk-proxy` command-line interface.

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;

/// Talon MITM proxy.
#[derive(Debug, Parser)]
#[command(
    name = "bk-proxy",
    version,
    about = "Talon MITM web-security proxy",
    long_about = None
)]
pub struct Cli {
    /// Path to the Talon config directory. Contains `proxy.toml`, the
    /// CA cert / key, and project storage. Defaults to
    /// `$XDG_CONFIG_HOME/talon` or `~/.config/talon` if unset.
    #[arg(long, env = "TALON_CONFIG_DIR", value_name = "DIR")]
    pub config_dir: Option<PathBuf>,

    /// Address the TCP listener binds to.
    #[arg(long, default_value = "127.0.0.1:8080", value_name = "ADDR")]
    pub listen: SocketAddr,

    /// Maximum number of in-flight connections. Acts as a backpressure
    /// cap: when reached, the accept loop pauses until a slot frees up.
    #[arg(long, default_value_t = 256, value_name = "N")]
    pub max_connections: usize,
}

/// Resolve the effective config dir, honouring the XDG fallback.
pub fn resolve_config_dir(cli: &Cli) -> PathBuf {
    if let Some(d) = &cli.config_dir {
        return d.clone();
    }
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("talon");
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".config").join("talon");
    }
    PathBuf::from(".config/talon")
}
