//! `bk-mcp` — the binary entry point. Spawned by MCP clients
//! (Claude Desktop, etc.) as a subprocess; speaks JSON-RPC 2.0
//! over stdio.
//!
//! ## Usage
//!
//! ```sh
//! TALON_CONFIG_DIR=~/.config/talon bk-mcp
//! ```
//!
//! `TALON_CONFIG_DIR` is the directory the `bk-engine::Engine`
//! reads/writes projects to. One DB file per project, named
//! `<project-slug>-<YYYY-MM-DD>.db` per the engine's filename
//! convention. The env var is the single source of truth for
//! the config dir; no CLI flags (the binary is a stdio server,
//! not a CLI tool).
//!
//! Logging is intentionally minimal: server-side errors are
//! written to stderr via `eprintln!` so the MCP client (which
//! reads from stdin) is not interfered with. This keeps the
//! binary's dep set small.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use bk_engine::Engine;
use bk_mcp::McpServer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config_dir = std::env::var_os("TALON_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("~/.config/talon"));

    // Expand a leading `~` to the user's home directory. The
    // config dir is a trust boundary (the engine writes DB
    // files there) so we resolve it once at startup.
    let config_dir = if config_dir.starts_with("~") {
        if let Some(home) = std::env::var_os("HOME") {
            PathBuf::from(home).join(
                config_dir
                    .strip_prefix("~")
                    .expect("starts_with('~') implies a prefix to strip"),
            )
        } else {
            config_dir
        }
    } else {
        config_dir
    };
    std::fs::create_dir_all(&config_dir)
        .with_context(|| format!("creating config dir {}", config_dir.display()))?;

    eprintln!("bk-mcp starting (config_dir = {})", config_dir.display());

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
