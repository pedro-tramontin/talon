//! Error types for the MCP server and the JSON-RPC error-code
//! mapping from `EngineError`.
//!
//! Per the design contract (gotcha 2):
//!
//! - `InvalidArgs` (our internal) → JSON-RPC code -32602 (Invalid params)
//! - `EngineError::ProjectNotOpen(_)` → -32602 (the LLM can fix this by
//!   calling `talon_open_project` first)
//! - `EngineError::Core(_)` / `EngineError::Store(_)` → -32603
//!   (Internal error; the LLM can surface the message to the user)
//!
//! The mapping happens in [`McpError::jsonrpc_code`].

use bk_engine::EngineError;
use thiserror::Error;

/// Errors that can occur inside an MCP tool handler.
///
/// Every variant has a well-defined JSON-RPC error code via
/// [`McpError::jsonrpc_code`]. The string returned by
/// `Display` is the `message` field in the JSON-RPC error response.
#[derive(Debug, Error)]
pub enum McpError {
    /// The LLM passed a missing or malformed argument.
    /// Maps to JSON-RPC -32602.
    #[error("invalid args: {0}")]
    InvalidArgs(String),

    /// The LLM called a tool that doesn't exist.
    /// Maps to JSON-RPC -32601.
    #[error("unknown tool: {0}")]
    UnknownTool(String),

    /// A downstream engine call failed. The variant holds the
    /// original `EngineError` so the caller can pattern-match
    /// on the cause; the JSON-RPC code is decided by
    /// [`McpError::jsonrpc_code`].
    #[error("engine error: {0}")]
    Engine(#[from] EngineError),

    /// JSON serialization/deserialization failed when marshaling
    /// arguments or results. Maps to JSON-RPC -32603.
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    /// The tool is a Phase N stub that hasn't been implemented
    /// yet. Maps to JSON-RPC -32603 (internal error — the LLM
    /// surfaces this and moves on; see design-contract gotcha
    /// for the stub-tool rationale).
    #[error("not implemented in v0.1, lands in phase {phase}: {tool}")]
    NotImplemented {
        /// The MCP tool name the LLM called.
        tool: &'static str,
        /// Which Talon phase will implement it.
        phase: &'static str,
    },

    /// A catch-all for unexpected internal errors. Maps to
    /// JSON-RPC -32603.
    #[error("internal error: {0}")]
    Internal(String),
}

impl McpError {
    /// The JSON-RPC 2.0 error code for this variant.
    ///
    /// Per the MCP spec (and design-contract gotcha 2):
    /// - `-32600` Invalid Request
    /// - `-32601` Method not found
    /// - `-32602` Invalid params
    /// - `-32603` Internal error
    pub fn jsonrpc_code(&self) -> i32 {
        match self {
            McpError::InvalidArgs(_) => -32602,
            McpError::UnknownTool(_) => -32601,
            McpError::Engine(EngineError::ProjectNotOpen(_)) => -32602,
            // Wildcard arm (added 2026-07-18): if a new
            // EngineError variant is added in the future, it
            // maps to -32603 (internal error) without breaking
            // the build. The specific arms above take priority.
            McpError::Engine(_) => -32603,
            McpError::Serde(_) => -32603,
            McpError::NotImplemented { .. } => -32603,
            McpError::Internal(_) => -32603,
        }
    }
}

/// Convenience: convert a `Result<T, McpError>` into a JSON-RPC
/// response value. The MCP transport expects either a `result` or
/// an `error` — never both.
///
/// **Security note (added 2026-07-18):** `McpError::Engine(_)` for
/// non-`ProjectNotOpen` variants carries the original `EngineError`
/// whose `Display` impl may embed SQLite error text, file paths,
/// or SQL fragments. We log the full error server-side and emit
/// a generic message to the caller. The structured `code` field
/// still tells the LLM what went wrong (-32602 vs -32603) so it
/// can react correctly.
///
/// `ProjectNotOpen` is the exception: the LLM can recover from it
/// by calling `talon_open_project` first, so we keep the message
/// (which is just "project not open: <uuid>" — no SQL/path leak).
/// The uuid is one the LLM supplied itself in the first place.
pub fn result_to_response<T: serde::Serialize>(result: Result<T, McpError>) -> serde_json::Value {
    match result {
        Ok(value) => serde_json::json!({ "ok": true, "value": value }),
        Err(e) => {
            let message = match &e {
                // `ProjectNotOpen` is LLM-actionable and carries
                // no leakable content. Keep the message.
                McpError::Engine(bk_engine::EngineError::ProjectNotOpen(_)) => e.to_string(),
                // All other engine errors get sanitized to a
                // generic string. The full error is logged.
                McpError::Engine(inner) => {
                    tracing::error!(error = %inner, "mcp engine error");
                    "internal engine error".to_string()
                }
                _ => e.to_string(),
            };
            serde_json::json!({
                "ok": false,
                "error": {
                    "code": e.jsonrpc_code(),
                    "message": message,
                }
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_args_maps_to_minus_32602() {
        let e = McpError::InvalidArgs("project_id missing".into());
        assert_eq!(e.jsonrpc_code(), -32602);
    }

    #[test]
    fn unknown_tool_maps_to_minus_32601() {
        let e = McpError::UnknownTool("talon_does_not_exist".into());
        assert_eq!(e.jsonrpc_code(), -32601);
    }

    #[test]
    fn not_implemented_maps_to_minus_32603() {
        let e = McpError::NotImplemented {
            tool: "talon_proxy_start",
            phase: "3.6",
        };
        assert_eq!(e.jsonrpc_code(), -32603);
    }

    #[test]
    fn result_to_response_ok_shape() {
        let v = result_to_response::<i32>(Ok(42));
        assert_eq!(v["ok"], serde_json::json!(true));
        assert_eq!(v["value"], serde_json::json!(42));
    }

    #[test]
    fn result_to_response_err_shape() {
        let v: serde_json::Value =
            result_to_response::<i32>(Err(McpError::InvalidArgs("bad".into())));
        assert_eq!(v["ok"], serde_json::json!(false));
        assert_eq!(v["error"]["code"], serde_json::json!(-32602));
        assert!(v["error"]["message"].as_str().unwrap().contains("bad"));
    }

    /// **Integration test for security review MEDIUM #2**
    /// (added 2026-07-18, fixes PR #25 Copilot finding
    /// #3607982870): when a tool handler returns an
    /// `McpError::Engine(_)` carrying a `StoreError` whose
    /// `Display` impl would otherwise leak SQL/SQLite/path
    /// text to the LLM, `result_to_response` must sanitize
    /// the message to a generic string. The structured `code`
    /// field is preserved so the LLM can still distinguish
    /// "fix your args" from "internal problem".
    #[test]
    fn engine_error_message_is_sanitized_in_response() {
        // Construct a `StoreError` whose Display impl would
        // leak a path. `StoreError::Invalid(String)` formats
        // as "invalid input: <text>" — simulating a SQLite
        // error that references a DB path or SQL fragment.
        let leaky = bk_store::StoreError::Invalid(
            "sqlite error: UNIQUE constraint failed: exchanges.id (at /home/user/.local/share/talon/projects/acme-bb-2026-07-18.db)"
                .into(),
        );
        let engine_err: bk_engine::EngineError = leaky.into();
        let v: serde_json::Value = result_to_response::<i32>(Err(McpError::Engine(engine_err)));
        assert_eq!(v["ok"], serde_json::json!(false));
        assert_eq!(
            v["error"]["code"],
            serde_json::json!(-32603),
            "EngineError → -32603 (Internal error) per design contract"
        );
        let message = v["error"]["message"].as_str().unwrap();
        // The sanitized message must NOT contain the path, the
        // SQL fragment, or the SQLite error text.
        assert_eq!(
            message, "internal engine error",
            "EngineError message must be sanitized to a generic string, got: {message}"
        );
        assert!(
            !message.contains("UNIQUE constraint"),
            "sanitized message must not contain SQL error text"
        );
        assert!(
            !message.contains("/home/user"),
            "sanitized message must not contain file paths"
        );
        assert!(
            !message.contains("acme-bb"),
            "sanitized message must not contain the project name (leaks project structure)"
        );
    }

    /// **Integration test for security review MEDIUM #2
    /// (companion):** when a tool handler returns
    /// `McpError::Engine(EngineError::ProjectNotOpen(_))`,
    /// the message is *kept* (the LLM can react — "call
    /// talon_open_project first") but the code is still
    /// -32602 (Invalid params, LLM-actionable).
    ///
    /// Why we keep the message here: `ProjectNotOpen` is
    /// recoverable by the LLM, and the message is just
    /// "project not open: <uuid>" — no SQL/path leak.
    #[test]
    fn engine_error_project_not_open_message_is_kept_but_code_is_invalid_params() {
        let engine_err = bk_engine::EngineError::ProjectNotOpen("acme-bb-2026-07-18".into());
        let v: serde_json::Value = result_to_response::<i32>(Err(McpError::Engine(engine_err)));
        assert_eq!(
            v["error"]["code"],
            serde_json::json!(-32602),
            "ProjectNotOpen maps to -32602 (Invalid params, LLM-actionable)"
        );
        let message = v["error"]["message"].as_str().unwrap();
        // The LLM can see the project id so it can call
        // talon_open_project with the right id. This is
        // intentional and not a leak (the LLM already
        // supplied the id in the first place).
        assert!(
            message.contains("acme-bb-2026-07-18"),
            "ProjectNotOpen message should keep the project id, got: {message}"
        );
    }
}
