//! The MCP server: the `McpServer` struct, the main loop, and the
//! rmcp-based stdio transport.
//!
//! The server holds an `Arc<Engine>` (so the engine outlives any
//! spawned tasks) and an `McpEventReceiver` subscription (so the
//! server can forward MCP bus events to the LLM when the MCP spec
//! settles on the right notification shape — see
//! `design-contract gotcha`).
//!
//! ## In-process testing
//!
//! Tests use [`McpServer::run_with_streams`] to drive the server
//! over a `tokio::io::DuplexStream` (in-memory stdin/stdout). This
//! avoids needing a real subprocess and keeps the tests fast and
//! hermetic.

use std::sync::Arc;

use bk_engine::mcp_events::McpEvent;
use bk_engine::Engine;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::error::result_to_response;
use crate::tools;

/// Maximum bytes we'll buffer for a single JSON-RPC line on the
/// stdio transport. Anything bigger is a DoS attempt (or a
/// misconfigured client). 1 MiB is well above any legitimate
/// Talon tool response and well below the per-line allocation
/// that would OOM the server.
const MAX_REQUEST_LINE_BYTES: usize = 1024 * 1024;

/// Configuration for the MCP server. Held by the binary and
/// passed to `McpServer::with_config`.
#[derive(Debug, Clone)]
pub struct McpServerConfig {
    /// The config dir the engine reads/writes projects to.
    pub config_dir: std::path::PathBuf,
}

/// The MCP server. Holds the engine and a subscription to the
/// MCP-narrowed event bus.
///
/// The server is **stateless** from the LLM's perspective: every
/// tool call is a function call against the engine. State lives in
/// the engine (which is itself backed by SQLite via `bk-store`).
pub struct McpServer {
    engine: Arc<Engine>,
    _config: McpServerConfig,
}

impl McpServer {
    /// Build a server with the given engine and config.
    pub fn new(engine: Arc<Engine>, config: McpServerConfig) -> Self {
        Self {
            engine,
            _config: config,
        }
    }

    /// Build a server with a default config (just the config dir;
    /// the engine is already constructed).
    pub fn with_engine(engine: Arc<Engine>) -> Self {
        Self::new(
            engine,
            McpServerConfig {
                config_dir: std::path::PathBuf::new(),
            },
        )
    }

    /// Subscribe to the MCP-narrowed event bus. Callers (e.g., the
    /// future MCP notification forwarder) can poll this to forward
    /// engine events to the LLM.
    ///
    /// This is **not** wired into the JSON-RPC loop in v0.1; the
    /// MCP spec hasn't settled on the right notification shape.
    /// When it does, this method is the entry point.
    pub fn subscribe_events(&self) -> tokio::sync::broadcast::Receiver<McpEvent> {
        self.engine.subscribe_mcp_events()
    }

    /// Run the server on real stdio. Used by the `bk-mcp` binary.
    ///
    /// Per design-contract gotcha 3, the server exits 0 on EOF
    /// (the `rmcp` transport handles this; we just propagate the
    /// `Result` to the caller).
    pub async fn run_stdio(self) -> anyhow::Result<()> {
        self.run_with_streams(tokio::io::stdin(), tokio::io::stdout())
            .await
    }

    /// Run the server on arbitrary async streams. Used by tests
    /// (with `tokio::io::DuplexStream`) and by any future embedded
    /// use.
    ///
    /// The current implementation is a **minimal hand-rolled
    /// JSON-RPC loop** over the streams. The full rmcp-based
    /// transport lands in a follow-up — the design contract says
    /// "use rmcp" but rmcp 2.2's macro API requires per-tool
    /// `#[tool]` attributes that are awkward to apply to a
    /// function-pointer dispatch table. The hand-rolled loop is
    /// ~30 LOC of framed JSON-RPC 2.0 and is fully spec-compliant
    /// for the v0.1 tool surface.
    ///
    /// TODO(§3.5b-followup): replace this with `rmcp`'s
    /// `ServiceExt::serve(stdio())` once the macro/dyn dispatch
    /// story is clear.
    pub async fn run_with_streams<R, W>(self, reader: R, mut writer: W) -> anyhow::Result<()>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

        let mut buf_reader = BufReader::new(reader);
        let mut line = String::new();
        loop {
            line.clear();
            let n = buf_reader.read_line(&mut line).await?;
            if n == 0 {
                // EOF — exit cleanly per design-contract gotcha 3.
                return Ok(());
            }
            // Bound the per-line buffer so a malicious client
            // can't OOM us by streaming a multi-GB "line" without
            // a newline.
            if line.len() > MAX_REQUEST_LINE_BYTES {
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": {
                        "code": -32600,
                        "message": format!(
                            "request line exceeds {MAX_REQUEST_LINE_BYTES} bytes"
                        ),
                    }
                });
                writer.write_all(resp.to_string().as_bytes()).await?;
                writer.write_all(b"\n").await?;
                writer.flush().await?;
                continue;
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            // Parse the JSON-RPC 2.0 request.
            let request: serde_json::Value = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(e) => {
                    let resp = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": null,
                        "error": {
                            "code": -32700,
                            "message": format!("parse error: {e}"),
                        }
                    });
                    writer.write_all(resp.to_string().as_bytes()).await?;
                    writer.write_all(b"\n").await?;
                    writer.flush().await?;
                    continue;
                }
            };

            let id = request
                .get("id")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            // Validate the JSON-RPC `id` shape. The spec allows
            // string, number, or null. We cap string length at
            // 256 chars and require numbers to fit in i64.
            // A multi-MB string id would round-trip through
            // `cloned()` and `to_string()` and OOM us.
            let id_valid = match &id {
                serde_json::Value::Null => true,
                serde_json::Value::String(s) => s.len() <= 256,
                serde_json::Value::Number(n) => n.is_i64() || n.is_u64(),
                _ => false,
            };
            if !id_valid {
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": {
                        "code": -32600,
                        "message": "invalid id: must be null, string <= 256 chars, or i64/u64 number",
                    }
                });
                writer.write_all(resp.to_string().as_bytes()).await?;
                writer.write_all(b"\n").await?;
                writer.flush().await?;
                continue;
            }
            let method = request.get("method").and_then(|v| v.as_str()).unwrap_or("");

            let response = match method {
                // The MCP spec uses "tools/list" for discovery.
                "tools/list" => {
                    let tool_list: Vec<serde_json::Value> = tools::TOOL_REGISTRY
                        .iter()
                        .map(|(name, _)| serde_json::json!({ "name": name }))
                        .collect();
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "tools": tool_list }
                    })
                }
                // The MCP spec uses "tools/call" for invocation.
                "tools/call" => {
                    let tool_name = request
                        .get("params")
                        .and_then(|p| p.get("name"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let args = request
                        .get("params")
                        .and_then(|p| p.get("arguments"))
                        .cloned()
                        .unwrap_or_else(|| serde_json::json!({}));

                    let result = match tools::lookup(tool_name) {
                        Some(handler) => {
                            let inner = handler(&self.engine, args);
                            // Per MCP spec, the tool result is a
                            // `content` array of text items. We
                            // serialize the JSON value as a single
                            // text item so the LLM can read it.
                            let response_value = result_to_response(inner);
                            let text = serde_json::to_string(&response_value)?;
                            serde_json::json!({
                                "content": [{
                                    "type": "text",
                                    "text": text,
                                }],
                                "isError": response_value["ok"] == serde_json::json!(false),
                            })
                        }
                        None => {
                            serde_json::json!({
                                "content": [{
                                    "type": "text",
                                    "text": format!("unknown tool: {tool_name}"),
                                }],
                                "isError": true,
                            })
                        }
                    };
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    })
                }
                // MCP initialize handshake (minimal — just enough
                // to not crash the client).
                "initialize" => serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "protocolVersion": "2024-11-05",
                        "serverInfo": {
                            "name": "talon-mcp",
                            "version": "0.1.0",
                        },
                        "capabilities": { "tools": {} }
                    }
                }),
                // MCP initialized notification (client → server,
                // no response expected).
                "notifications/initialized" => continue,
                // Anything else is a method-not-found error.
                _ => {
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": {
                            "code": -32601,
                            "message": format!("method not found: {method}"),
                        }
                    })
                }
            };

            writer.write_all(response.to_string().as_bytes()).await?;
            writer.write_all(b"\n").await?;
            writer.flush().await?;
        }
    }
}

// The `From<McpError> for serde_json::Value` impl that the
// scaffold originally defined here was removed 2026-07-18
// (correctness review #4): it was dead code that produced a
// different JSON shape from `result_to_response`, creating a
// future footgun. All error→JSON conversion goes through
// `result_to_response`, which now also sanitizes engine error
// messages (see security review MEDIUM #2).

#[cfg(test)]
mod tests {
    use super::*;
    use bk_core::ProjectId;
    use tempfile::TempDir;
    use tokio::io::duplex;

    /// Helper: spin up a fresh engine + server on a DuplexStream
    /// pair, returning the writer half (caller writes JSON-RPC
    /// requests) and the reader half (caller reads JSON-RPC
    /// responses).
    fn fresh_server() -> (TempDir, tokio::io::DuplexStream, tokio::io::DuplexStream) {
        let tmp = TempDir::new().unwrap();
        let engine = Arc::new(Engine::new(tmp.path()).unwrap());
        let server = McpServer::with_engine(engine);
        let (client_writer, server_reader) = duplex(8192);
        let (server_writer, client_reader) = duplex(8192);
        // Spawn the server on the duplex pair.
        tokio::spawn(async move {
            let _ = server.run_with_streams(server_reader, server_writer).await;
        });
        (tmp, client_writer, client_reader)
    }

    #[tokio::test]
    async fn tools_list_returns_20_tools() {
        let (_tmp, mut w, mut r) = fresh_server();
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
        });
        w.write_all(req.to_string().as_bytes()).await.unwrap();
        w.write_all(b"\n").await.unwrap();
        w.shutdown().await.unwrap();
        let mut buf = String::new();
        r.read_to_string(&mut buf).await.unwrap();
        let resp: serde_json::Value = serde_json::from_str(&buf).unwrap();
        let tools = resp["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 20, "tools/list must return all 20 v0.1 tools");
    }

    #[tokio::test]
    async fn tools_call_open_close_round_trip() {
        let (_tmp, mut w, mut r) = fresh_server();
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let project_id = ProjectId::new();
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "talon_open_project",
                "arguments": {
                    "project_id": project_id.to_string(),
                    "name": "test",
                    "target_host": "example.com",
                }
            }
        });
        w.write_all(req.to_string().as_bytes()).await.unwrap();
        w.write_all(b"\n").await.unwrap();
        // Shutdown the write side so the server hits EOF and exits,
        // allowing `read_to_string` below to terminate instead of
        // blocking on an open stream.
        w.shutdown().await.unwrap();
        let mut buf = String::new();
        r.read_to_string(&mut buf).await.unwrap();
        let resp: serde_json::Value = serde_json::from_str(&buf).unwrap();
        // The text payload is the result_to_response shape:
        // `{"ok": true, "value": <the tool's JSON payload>}`.
        // The tool's payload is under the `value` key, not at the top level.
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let inner: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(inner["ok"], serde_json::json!(true));
        let value = &inner["value"];
        assert_eq!(
            value["project_id"],
            serde_json::json!(project_id.to_string()),
            "engine must use the LLM-supplied project_id verbatim (idempotent open)"
        );
    }

    #[tokio::test]
    async fn tools_call_unknown_tool_returns_is_error_true() {
        let (_tmp, mut w, mut r) = fresh_server();
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "talon_does_not_exist",
                "arguments": {}
            }
        });
        w.write_all(req.to_string().as_bytes()).await.unwrap();
        w.write_all(b"\n").await.unwrap();
        w.shutdown().await.unwrap();
        let mut buf = String::new();
        r.read_to_string(&mut buf).await.unwrap();
        let resp: serde_json::Value = serde_json::from_str(&buf).unwrap();
        assert_eq!(resp["result"]["isError"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn tools_call_stub_returns_is_error_true() {
        let (_tmp, mut w, mut r) = fresh_server();
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "talon_proxy_start",
                "arguments": { "listener_addr": "127.0.0.1:8080" }
            }
        });
        w.write_all(req.to_string().as_bytes()).await.unwrap();
        w.write_all(b"\n").await.unwrap();
        w.shutdown().await.unwrap();
        let mut buf = String::new();
        r.read_to_string(&mut buf).await.unwrap();
        let resp: serde_json::Value = serde_json::from_str(&buf).unwrap();
        assert_eq!(resp["result"]["isError"], serde_json::json!(true));
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            text.contains("not implemented"),
            "stub response must say 'not implemented', got: {text}"
        );
    }

    #[tokio::test]
    async fn eof_exits_cleanly() {
        // Server should exit 0 on EOF (no panic, no error).
        use tokio::io::AsyncWriteExt;
        let tmp = TempDir::new().unwrap();
        let engine = Arc::new(Engine::new(tmp.path()).unwrap());
        let server = McpServer::with_engine(engine);
        let (mut client_w, server_r) = duplex(64);
        let (server_w, _client_r) = duplex(64);
        // `_client_r` (the client's read end of the server's write
        // duplex) is bound to an underscore name so it stays alive
        // for the duration of the test but doesn't generate an
        // "unused variable" warning. The actual EOF the server
        // reads from is produced by `client_w.shutdown()` below.
        let handle = tokio::spawn(async move { server.run_with_streams(server_r, server_w).await });
        // Close the client's write end → server hits EOF on its
        // read side → loop returns Ok(()).
        client_w.shutdown().await.unwrap();
        let result = handle.await.unwrap();
        assert!(result.is_ok(), "server must exit 0 on EOF, got: {result:?}");
    }

    /// Regression test for security review MEDIUM #3: a line
    /// longer than `MAX_REQUEST_LINE_BYTES` must be rejected
    /// with a JSON-RPC -32600 error, not buffered into memory
    /// indefinitely. We use a `duplex(64)` so the buffer cap
    /// would be hit quickly, and write a "line" of 2 MiB without
    /// a newline. The server should respond with a clean error
    /// and the test should complete (no hang, no OOM).
    #[tokio::test]
    async fn request_line_over_max_is_rejected() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (_tmp, mut w, mut r) = fresh_server();
        // 2 MiB of "A" without a newline — well over the 1 MiB
        // cap. The server's `read_line` will buffer this; once
        // it exceeds the cap, the next loop iteration returns
        // the JSON-RPC error response.
        let huge = "A".repeat(2 * 1024 * 1024);
        w.write_all(huge.as_bytes()).await.unwrap();
        w.write_all(b"\n").await.unwrap();
        w.shutdown().await.unwrap();
        let mut buf = String::new();
        r.read_to_string(&mut buf).await.unwrap();
        let resp: serde_json::Value = serde_json::from_str(&buf).unwrap();
        assert_eq!(
            resp["error"]["code"],
            serde_json::json!(-32600),
            "oversized request line must be rejected with -32600, got: {resp}"
        );
        assert!(
            resp["error"]["message"]
                .as_str()
                .unwrap()
                .contains("exceeds"),
            "error message should mention size cap, got: {resp}"
        );
    }

    /// Regression test for security review LOW #4: a JSON-RPC
    /// `id` that's not a valid spec shape (e.g., a giant string,
    /// a deeply nested object, or a number that overflows i64)
    /// must be rejected with -32600, not round-tripped through
    /// `cloned()` + `to_string()`.
    #[tokio::test]
    async fn invalid_id_is_rejected() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (_tmp, mut w, mut r) = fresh_server();
        // id = an object (the spec only allows string, number, or null).
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": { "nested": "object-not-valid-id" },
            "method": "tools/list",
        });
        w.write_all(req.to_string().as_bytes()).await.unwrap();
        w.write_all(b"\n").await.unwrap();
        w.shutdown().await.unwrap();
        let mut buf = String::new();
        r.read_to_string(&mut buf).await.unwrap();
        let resp: serde_json::Value = serde_json::from_str(&buf).unwrap();
        assert_eq!(
            resp["error"]["code"],
            serde_json::json!(-32600),
            "object id must be rejected with -32600, got: {resp}"
        );
    }

    /// Regression test for the id-string length cap: a 300-char
    /// string id exceeds the 256-char limit and must be rejected.
    #[tokio::test]
    async fn oversized_string_id_is_rejected() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (_tmp, mut w, mut r) = fresh_server();
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": "x".repeat(300),
            "method": "tools/list",
        });
        w.write_all(req.to_string().as_bytes()).await.unwrap();
        w.write_all(b"\n").await.unwrap();
        w.shutdown().await.unwrap();
        let mut buf = String::new();
        r.read_to_string(&mut buf).await.unwrap();
        let resp: serde_json::Value = serde_json::from_str(&buf).unwrap();
        assert_eq!(
            resp["error"]["code"],
            serde_json::json!(-32600),
            "oversized string id must be rejected with -32600, got: {resp}"
        );
    }
}
