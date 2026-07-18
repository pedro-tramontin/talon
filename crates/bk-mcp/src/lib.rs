//! `bk-mcp` — the Model Context Protocol (MCP) server for Talon.
//!
//! This crate exposes the `bk-engine` API as MCP tools so AI agents
//! (Claude Desktop, etc.) can interact with Talon's HTTP-proxy
//! capture and replay workflow. The server speaks stdio MCP, holding
//! an `Engine` (with its config dir) and a subscription to the
//! MCP-narrowed event bus from `bk-engine::mcp_events`.
//!
//! ## Tool surface (v0.1, 20 tools)
//!
//! Per the design contract, the v0.1 tool list is **stable** —
//! the LLM caches the tool list and changes are breaking.
//!
//! - `talon_open_project`, `talon_close_project`, `talon_list_open_projects`
//! - `talon_insert_exchange`, `talon_get_exchange`, `talon_list_recent`,
//!   `talon_search`
//! - `talon_update_notes`, `talon_set_starred`, `talon_delete_exchange`
//! - `talon_upsert_tag`, `talon_list_tags`, `talon_attach_tag`,
//!   `talon_detach_tag`, `talon_list_tags_for_exchange`
//! - `talon_proxy_start`, `talon_proxy_stop` (Phase 3 stubs)
//! - `talon_fuzz_start`, `talon_fuzz_stop` (Phase 7 stubs)
//! - `talon_get_config`
//!
//! ## Architecture
//!
//! The crate has three pieces:
//!
//! - [`server`]: the `McpServer` struct and the main loop. The
//!   transport is a **hand-rolled JSON-RPC 2.0 stdio loop** in
//!   `run_with_streams`, NOT rmcp-based. Tool handlers return a
//!   JSON payload that the transport wraps into the MCP
//!   `content` array (each entry is
//!   `{"type": "text", "text": <json>}`). The full rmcp-based
//!   transport lands in a §3.5b-followup — the design contract
//!   says "use rmcp" but rmcp 2.2's macro API requires per-tool
//!   `#[tool]` attributes that are awkward to apply to a
//!   function-pointer dispatch table. Holds an `Engine` and an
//!   event-bus subscription.
//! - [`tools`]: the 20-tool registry. Each tool is a function that
//!   takes a `&Engine` and a `serde_json::Value` of args and returns
//!   a `serde_json::Value` (the tool's own payload, not the MCP
//!   content envelope).
//! - [`error`]: the `McpError` enum and the JSON-RPC error-code
//!   mapping from `EngineError` per design-contract gotcha 2.
//!
//! ## Usage
//!
//! As a library (in tests or embedded use):
//!
//! ```no_run
//! # use bk_mcp::McpServer;
//! # use bk_engine::Engine;
//! # use std::sync::Arc;
//! # async fn example() -> anyhow::Result<()> {
//! let engine = Arc::new(Engine::new("/tmp/talon-mcp-test")?);
//! let server = McpServer::with_engine(engine);
//! server.run_stdio().await?;
//! # Ok(()) }
//! ```
//!
//! As a binary (the `bk-mcp` executable that MCP clients like
//! Claude Desktop spawn as a subprocess):
//!
//! ```sh
//! bk-mcp --config-dir ~/.config/talon
//! ```

#![deny(missing_docs)]
#![warn(rust_2018_idioms)]

pub mod error;
pub mod server;
pub mod tools;

pub use error::McpError;
pub use server::{McpServer, McpServerConfig};
pub use tools::{lookup, ToolHandler, TOOL_REGISTRY};

/// Re-exported from `bk-engine` so downstream code that uses
/// `bk-mcp` doesn't need to depend on `bk-engine` directly.
pub use bk_engine::{mcp_events::McpEvent, Engine};
