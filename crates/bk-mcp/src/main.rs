//! `bk-mcp` — the binary entry point. Spawned by MCP clients
//! (Claude Desktop, etc.) as a subprocess; speaks JSON-RPC 2.0
//! over stdio.
//!
//! ## Usage
//!
//! ```sh
//! bk-mcp --config-dir ~/.config/talon
//! ```
//!
//! `--config-dir` is the directory the `bk-engine::Engine`
//! reads/writes projects to. One DB file per project, named
//! `<project-slug>-<YYYY-MM-DD>.db` per the engine's filename
//! convention.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use bk_engine::Engine;
use bk_mcp::McpServer;
use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "bk-mcp",
    version,
    about = "Talon MCP server (stdio JSON-RPC 2.0). Speaks the Model Context Protocol to AI agents like Claude Desktop."
)]
struct Args {
    /// Directory the engine reads/writes projects to. One
    /// `<project-slug>-<date>.db` file per opened project.
    #[arg(long, env = "TALON_CONFIG_DIR", default_value = "~/.config/talon")]
    config_dir: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing for the server-side error log
    // (sanitized McpError::Engine messages are written here).
    // The MCP client never sees this output — it goes to stderr.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();

    // Expand a leading `~` to the user's home directory. The
    // config dir is a trust boundary (the engine writes DB
    // files there) so we resolve it once at startup.
    let config_dir = args.config_dir;
    let config_dir = if config_dir.starts_with("~") {
        if let Some(home) = std::env::var_os("HOME") {
            PathBuf::from(home).join(config_dir.strip_prefix("~").unwrap())
        } else {
            config_dir
        }
    } else {
        config_dir
    };
    std::fs::create_dir_all(&config_dir)
        .with_context(|| format!("creating config dir {}", config_dir.display()))?;

    tracing::info!(config_dir = %config_dir.display(), "bk-mcp starting");

    let engine = Arc::new(
        Engine::new(&config_dir)
            .with_context(|| format!("creating engine at {}", config_dir.display()))?,
    );
    let server = McpServer::with_engine(engine);
    server
        .run_stdio()
        .await
        .context("bk-mcp server loop exited with error")?;
    Ok(())
}
