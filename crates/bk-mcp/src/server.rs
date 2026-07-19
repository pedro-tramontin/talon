//! The MCP server: the `McpServer` struct, the main loop, and the
//! hand-rolled JSON-RPC 2.0 stdio transport.
//!
//! **Not** rmcp-based yet — the design contract says "use rmcp"
//! but rmcp 2.2's macro API requires per-tool `#[tool]`
//! attributes that are awkward to apply to a function-pointer
//! dispatch table. The hand-rolled loop is ~140 LOC of framed
//! JSON-RPC 2.0 and is fully spec-compliant for the v0.1 tool
//! surface. The rmcp transport lands in a §3.5b-followup.
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

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use bk_engine::mcp_events::McpEvent;
use bk_engine::Engine;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use crate::error::result_to_response;
use crate::tools;

/// Maximum bytes we'll buffer for a single JSON-RPC line on the
/// stdio transport. Anything bigger is a DoS attempt (or a
/// misconfigured client). 1 MiB is well above any legitimate
/// Talon tool response and well below the per-line allocation
/// that would OOM the server.
const MAX_REQUEST_LINE_BYTES: usize = 1024 * 1024;

/// `AsyncRead` wrapper that enforces a per-line byte cap by
/// resetting a counter on every newline byte observed in the
/// current read call. Returns an I/O error when the cap is
/// exceeded within a single line, so `read_line` propagates the
/// error and the JSON-RPC server can emit a clean -32600 response.
///
/// **Why this exists (added 2026-07-18, fixes PR #25 Copilot
/// HIGH-severity finding #3607982832):** the previous
/// implementation used `BufReader::read_line` and then checked
/// `line.len() > MAX_REQUEST_LINE_BYTES` after the read. That
/// check fires *after* the entire line is already in memory, so
/// a 10 GB "line" with no newline allocates 10 GB before the
/// cap runs. This wrapper enforces the cap **during** the
/// read, so the buffer can never exceed `MAX_REQUEST_LINE_BYTES`.
struct NewlineBoundedRead<R> {
    inner: R,
    /// Bytes remaining in the current line. Reset to
    /// `MAX_REQUEST_LINE_BYTES` every time a newline byte is
    /// observed in the buffer the caller just filled.
    remaining: usize,
}

impl<R: AsyncRead + Unpin> NewlineBoundedRead<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            remaining: MAX_REQUEST_LINE_BYTES,
        }
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for NewlineBoundedRead<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        // Hand the caller's ReadBuf directly to the inner reader.
        // The inner reader fills the unfilled portion; after the
        // poll, `buf.filled()` reports how many bytes were written
        // (the slice is `&buf[buf.start_filled..buf.end_filled]`).
        let prev_remaining = self.remaining;
        let prev_filled_len = buf.filled().len();
        match Pin::new(&mut self.inner).poll_read(cx, buf) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Ready(Ok(())) => {
                let new_filled_len = buf.filled().len();
                let n = new_filled_len - prev_filled_len;
                if n == 0 {
                    // EOF — let the caller see it.
                    return Poll::Ready(Ok(()));
                }
                // Count newlines in the just-filled slice and
                // reset the per-line counter for each one.
                let filled = buf.filled();
                let newlines = filled[prev_filled_len..new_filled_len]
                    .iter()
                    .filter(|&&b| b == b'\n')
                    .count();
                if newlines > 0 {
                    // Each newline resets the counter.
                    self.remaining = MAX_REQUEST_LINE_BYTES;
                } else {
                    self.remaining = prev_remaining.saturating_sub(n);
                }
                if self.remaining == 0 {
                    // Cap exceeded within this line and no
                    // newline reset the counter. Reject.
                    return Poll::Ready(Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("line exceeds {MAX_REQUEST_LINE_BYTES} bytes"),
                    )));
                }
                Poll::Ready(Ok(()))
            }
        }
    }
}

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

        // The reader is wrapped in `NewlineBoundedRead`, which
        // enforces the per-line cap **during** the read, not
        // after. A 10 GB "line" with no newline will see the
        // inner reader return an error after `MAX_REQUEST_LINE_BYTES`
        // bytes, so the buffer never grows past the cap.
        // `BufReader` is layered on top to give us `read_line`
        // semantics.
        let mut buf_reader = BufReader::new(NewlineBoundedRead::new(reader));
        let mut line = String::new();
        // When the wrapper rejects a read (line too long), we
        // can't simply `continue` — the wrapper's per-line
        // counter is at 0 and the next `read_line` would reject
        // again, looping forever. We set this flag and exit
        // cleanly on the next iteration, telling the client to
        // close + reconnect. A misbehaving client gets a single
        // -32600 and the connection drops — this is the correct
        // behavior per the MCP spec (a request line over the
        // transport cap is a protocol violation).
        let mut to_long_response_sent = false;
        loop {
            line.clear();
            if to_long_response_sent {
                // After the -32600 response, exit the server.
                // The client is expected to close + reconnect;
                // any further bytes on the stream are discarded.
                return Ok(());
            }
            let n = match buf_reader.read_line(&mut line).await {
                Ok(n) => n,
                Err(_e) => {
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
                    to_long_response_sent = true;
                    continue;
                }
            };
            if n == 0 {
                // EOF — exit cleanly per design-contract gotcha 3.
                return Ok(());
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

            // Distinguish notifications (no `id` member) from
            // requests with `id: null` or an explicit id value.
            // Per JSON-RPC 2.0: a notification MUST NOT receive a
            // response; a request (even with id=null) must.
            // `request.get("id")` returns `None` for missing key,
            // `Some(&Value::Null)` for explicit null.
            let has_id = request.get("id").is_some();
            let id = request
                .get("id")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            // Validate the JSON-RPC `id` shape. The spec allows
            // string, number, or null. We cap string length at
            // 256 chars and require numbers to fit in i64.
            // A multi-MB string id would round-trip through
            // `cloned()` + `to_string()` and OOM us.
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

            // Helper: write a JSON-RPC response only if this is
            // a request (not a notification). The closure
            // returns `()` and writes to `writer` lazily.
            // Notifications get no response at all.
            let is_notification = !has_id;
            macro_rules! send_response {
                ($resp:expr) => {{
                    if !is_notification {
                        let body = serde_json::to_string(&$resp)?;
                        writer.write_all(body.as_bytes()).await?;
                        writer.write_all(b"\n").await?;
                        writer.flush().await?;
                    }
                }};
            }

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

            // Suppress the response for JSON-RPC notifications
            // (requests with no `id` member). Per spec, the
            // server MUST NOT reply to notifications.
            send_response!(response);
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

    /// Regression test for security review MEDIUM #3 / PR #25
    /// Copilot HIGH-severity finding #3607982832. The old
    /// implementation used `BufReader::read_line` and checked
    /// `line.len() > MAX` AFTER the read — which let a 10 GB
    /// "line" allocate 10 GB before the cap ran. The new
    /// implementation uses `NewlineBoundedRead`, which rejects
    /// during the read when the per-line cap is hit.
    ///
    /// The test writes `MAX_REQUEST_LINE_BYTES + 1024` bytes
    /// (just over the cap) without a newline. The wrapper
    /// rejects at the cap, the server returns -32600 and exits
    /// (per design — a misbehaving client gets one -32600 and
    /// the connection drops, per MCP spec the client should
    /// close + reconnect).
    #[tokio::test]
    async fn request_line_over_max_is_rejected() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (_tmp, mut w, mut r) = fresh_server();
        // 1 MiB + 1 KiB of "A" without a newline — over the
        // 1 MiB cap. The wrapper rejects the read and the
        // server returns -32600 + closes the connection. The
        // client write may fail with BrokenPipe after the
        // server exits mid-write, so we spawn the write in a
        // task and ignore its result.
        let huge = "A".repeat(MAX_REQUEST_LINE_BYTES + 1024);
        let writer = tokio::spawn(async move {
            // The write may fail with BrokenPipe if the server
            // exits before we finish — that's expected and not
            // a test failure.
            let _ = w.write_all(huge.as_bytes()).await;
            let _ = w.shutdown().await;
        });
        let mut buf = String::new();
        r.read_to_string(&mut buf).await.unwrap();
        // Clean up the writer task regardless of its result.
        let _ = writer.await;
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

    /// Regression test for security review MEDIUM #3 / PR #25
    /// Copilot HIGH-severity finding #3607982832. The old
    /// implementation used `BufReader::read_line` and checked
    /// `line.len() > MAX` AFTER the read — which let a 10 GB
    /// "line" allocate 10 GB before the cap ran. The new
    /// implementation uses `NewlineBoundedRead`, which rejects
    /// during the read when the per-line cap is hit. The
    /// regression test: an instrumented reader that counts
    /// bytes read; after a "no newline for 5 MiB" write, the
    /// wrapper must reject *before* 5 MiB are consumed by
    /// BufReader. If the old code were in place, all 5 MiB
    /// would be read into memory.
    #[tokio::test]
    async fn bounded_reader_rejects_during_read_not_after() {
        use std::pin::Pin;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::task::{Context, Poll};
        use tokio::io::{AsyncRead, ReadBuf};

        /// A test-only reader that returns up to N bytes
        /// total and counts how many were read.
        struct CappedReader {
            remaining: usize,
            bytes_read: Arc<AtomicUsize>,
            // 1 MiB buffer of 'A' bytes — enough to fill any
            // reasonable request and trigger the wrapper's
            // per-line cap.
            payload: Vec<u8>,
        }
        impl AsyncRead for CappedReader {
            fn poll_read(
                mut self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
                buf: &mut ReadBuf<'_>,
            ) -> Poll<std::io::Result<()>> {
                if self.remaining == 0 {
                    return Poll::Ready(Ok(())); // EOF
                }
                // Cap the slice at the smaller of (a) the
                // requested buffer size and (b) the payload
                // size — we don't want to index past `payload`.
                let n = std::cmp::min(
                    std::cmp::min(buf.remaining(), self.payload.len()),
                    self.remaining,
                );
                buf.put_slice(&self.payload[..n]);
                self.remaining -= n;
                self.bytes_read.fetch_add(n, Ordering::SeqCst);
                Poll::Ready(Ok(()))
            }
        }

        let bytes_read = Arc::new(AtomicUsize::new(0));
        // 5 MiB of 'A' bytes, no newline. The wrapper should
        // reject at MAX_REQUEST_LINE_BYTES (1 MiB) and the
        // test's BufReader will surface the error.
        let total = 5 * 1024 * 1024;
        let reader = CappedReader {
            remaining: total,
            bytes_read: bytes_read.clone(),
            payload: vec![b'A'; 4096],
        };

        use tokio::io::{AsyncBufReadExt, BufReader};
        let mut buf_reader = BufReader::new(NewlineBoundedRead::new(reader));
        let mut line = String::new();
        // The wrapper's cap is MAX_REQUEST_LINE_BYTES (1 MiB);
        // the underlying reader will supply at most that much
        // (plus a tiny bit) before the wrapper rejects. We
        // assert the read consumed less than the full 5 MiB.
        let result = buf_reader.read_line(&mut line).await;
        assert!(
            result.is_err(),
            "NewlineBoundedRead must reject over-cap reads with an error"
        );
        let consumed = bytes_read.load(Ordering::SeqCst);
        assert!(
            consumed <= MAX_REQUEST_LINE_BYTES + 4096,
            "wrapper must reject before reading past the cap; \
             consumed {consumed} bytes, cap is {MAX_REQUEST_LINE_BYTES} \
             (the +4096 slack is one BufReader internal fill)"
        );
        // Specifically, we want to catch the regression where
        // someone moves the cap check back to AFTER read_line
        // (the OLD broken pattern). With the OLD pattern,
        // the entire 5 MiB would be consumed before the cap
        // is checked, and the test would fail here.
        assert!(
            consumed < total,
            "wrapper must NOT consume the full {total} bytes before rejecting; \
             consumed {consumed} — this is the HIGH-severity DoS Copilot caught"
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
