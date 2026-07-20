//! Tool-calling layer for the agent.
//!
//! Each read-only tool is declared with its OpenAI function-calling
//! schema and a lightweight argument parser.  The dispatch itself
//! reuses `bk_mcp::tools::lookup` so the engine interaction stays
//! identical between the MCP server and the internal agent, but the
//! JSON schema exposed to the LLM is owned here and restricted to the
//! read-only subset by default.

use crate::Result;
use async_openai::types::chat::{ChatCompletionTool, ChatCompletionTools, FunctionObject};
use bk_engine::Engine;
use serde_json::{json, Value};
use std::sync::LazyLock;

/// Schema for one tool: name, description, and the JSON schema the
/// LLM uses to generate arguments.
#[derive(Debug, Clone)]
pub struct ToolSchema {
    /// Tool name, e.g. `talon_search`.
    pub name: &'static str,
    /// Human-readable description.
    pub description: &'static str,
    /// JSON Schema object for the tool arguments.
    pub parameters: Value,
}

/// All tools the agent knows about, in OpenAI function-calling format.
pub static TOOL_SCHEMAS: LazyLock<Vec<ToolSchema>> = LazyLock::new(|| {
    vec![
        ToolSchema {
            name: "talon_list_recent",
            description: "List the most recent HTTP exchanges for a project.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "project_id": {
                        "type": "string",
                        "description": "UUID of the project to query.",
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum rows to return (1-1000).",
                        "minimum": 1,
                        "maximum": 1000,
                        "default": 50,
                    },
                },
                "required": ["project_id"],
            }),
        },
        ToolSchema {
            name: "talon_search",
            description: "Full-text search over captured exchanges using FTS5.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "project_id": {
                        "type": "string",
                        "description": "UUID of the project to query.",
                    },
                    "query": {
                        "type": "string",
                        "description": "FTS5 query string.",
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum rows to return (1-100).",
                        "minimum": 1,
                        "maximum": 100,
                        "default": 10,
                    },
                },
                "required": ["project_id", "query"],
            }),
        },
        ToolSchema {
            name: "talon_get_exchange",
            description: "Fetch one HTTP exchange by its UUID.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "project_id": {
                        "type": "string",
                        "description": "UUID of the project that owns the exchange.",
                    },
                    "exchange_id": {
                        "type": "string",
                        "description": "UUID of the exchange to fetch.",
                    },
                },
                "required": ["project_id", "exchange_id"],
            }),
        },
        ToolSchema {
            name: "talon_list_tags",
            description: "List all tags defined in a project.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "project_id": {
                        "type": "string",
                        "description": "UUID of the project to query.",
                    },
                },
                "required": ["project_id"],
            }),
        },
        ToolSchema {
            name: "talon_list_tags_for_exchange",
            description: "List the tags attached to a specific exchange.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "project_id": {
                        "type": "string",
                        "description": "UUID of the project that owns the exchange.",
                    },
                    "exchange_id": {
                        "type": "string",
                        "description": "UUID of the exchange.",
                    },
                },
                "required": ["project_id", "exchange_id"],
            }),
        },
    ]
});

/// Convert all agent tool schemas into the `ChatCompletionTool`
/// shape expected by `async-openai` 0.41.1.
pub fn openai_tools() -> Vec<ChatCompletionTools> {
    openai_tools_from(&TOOL_SCHEMAS)
}

/// Render the OpenAI tool list from an explicit schema slice.
///
/// This is the per-run allowlist-filtered view: the agent loop passes
/// the schemas corresponding to `AgentConfig::allowed_tools` so the
/// LLM never sees tools it cannot actually call. `openai_tools()`
/// above is the full-registry convenience wrapper kept for tests
/// and any future caller that wants the complete catalog.
pub fn openai_tools_from(schemas: &[ToolSchema]) -> Vec<ChatCompletionTools> {
    schemas
        .iter()
        .map(|schema| {
            ChatCompletionTools::Function(ChatCompletionTool {
                function: FunctionObject {
                    name: schema.name.to_string(),
                    description: Some(schema.description.to_string()),
                    parameters: Some(schema.parameters.clone()),
                    strict: None,
                },
            })
        })
        .collect()
}

/// Execute one tool call, honoring the `allowed_tools` whitelist.
///
/// **Never aborts the agent run.** Common model mistakes — a
/// disallowed tool, an unknown tool, or invalid JSON in the
/// arguments — are returned to the LLM as an `{"ok": false, "error": ...}`
/// payload so the loop can continue and the LLM can try a
/// different approach (per the system-prompt contract). Only
/// true programmer errors (the `bk_mcp` lookup itself panicking,
/// for example) propagate as `Err`; those should not happen in
/// practice.
pub fn execute(engine: &Engine, allowed_tools: &[String], call: &FunctionCall) -> Result<Value> {
    if !allowed_tools.contains(&call.name) {
        return Ok(json!({
            "ok": false,
            "error": format!("tool {} is not in the allowed list", call.name),
        }));
    }

    let args: Value = match serde_json::from_str(&call.arguments) {
        Ok(v) => v,
        Err(e) => {
            return Ok(json!({
                "ok": false,
                "error": format!("invalid JSON arguments: {e}"),
            }));
        }
    };

    let handler = match bk_mcp::tools::lookup(&call.name) {
        Some(h) => h,
        None => {
            return Ok(json!({
                "ok": false,
                "error": format!("unknown tool: {}", call.name),
            }));
        }
    };

    // `bk_mcp` tool handlers return `Value` on success; on error they
    // return `bk_mcp::McpError`.  Convert the error to a string payload
    // rather than aborting the whole run, so the LLM can try a
    // different approach per the system prompt rules.
    match handler(engine, args.clone()) {
        Ok(value) => Ok(value),
        Err(err) => Ok(json!({
            "ok": false,
            "error": err.to_string(),
        })),
    }
}

/// Build a one-line summary of a tool result for the progress event bus.
pub fn summarize_result(tool_name: &str, value: &Value) -> String {
    if let Some(err) = value.get("error").and_then(|v| v.as_str()) {
        return format!("{tool_name} failed: {err}");
    }

    match tool_name {
        "talon_search" => {
            let count = value.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
            format!("found {count} matching exchange(s)")
        }
        "talon_list_recent" => {
            let count = value.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
            format!("returned {count} recent exchange(s)")
        }
        "talon_get_exchange" => value
            .get("meta")
            .and_then(|m| m.get("summary"))
            .and_then(|s| s.as_str())
            .map(|s| format!("returned exchange: {s}"))
            .unwrap_or_else(|| "returned exchange".to_string()),
        "talon_list_tags" => {
            let count = value.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
            format!("{count} tag(s)")
        }
        "talon_list_tags_for_exchange" => {
            let count = value.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
            format!("{count} tag(s) on exchange")
        }
        _ => "ok".to_string(),
    }
}

/// The name + arguments the LLM produced.  We keep a minimal struct
/// so the rest of the crate doesn't depend on the exact async-openai
/// tool-call type.
pub type FunctionCall = async_openai::types::chat::FunctionCall;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn default_schemas_are_five_read_only_tools() {
        assert_eq!(TOOL_SCHEMAS.len(), 5);
    }

    #[test]
    fn default_schema_names_match_read_only_list() {
        let names: Vec<_> = TOOL_SCHEMAS.iter().map(|s| s.name).collect();
        for &tool in crate::DEFAULT_READ_ONLY_TOOLS {
            assert!(names.contains(&tool), "missing schema for {tool}");
        }
    }

    #[test]
    fn summarize_search_counts_hits() {
        let value = json!({ "ok": true, "count": 3, "exchange_ids": [] });
        assert_eq!(
            summarize_result("talon_search", &value),
            "found 3 matching exchange(s)"
        );
    }

    #[test]
    fn summarize_error_result() {
        let value = json!({ "ok": false, "error": "boom" });
        assert_eq!(
            summarize_result("talon_search", &value),
            "talon_search failed: boom"
        );
    }

    #[test]
    fn openai_tools_from_slice_filters_to_passed_schemas() {
        // LE-1 / Copilot thread #1 regression: the agent must be able
        // to advertise only the per-run allowlist, not the whole
        // catalog. `openai_tools_from(&[])` is the empty case.
        let all = openai_tools();
        assert_eq!(all.len(), 5);

        let only_list_recent: Vec<_> = TOOL_SCHEMAS
            .iter()
            .filter(|s| s.name == "talon_list_recent")
            .cloned()
            .collect();
        let filtered = openai_tools_from(&only_list_recent);
        assert_eq!(filtered.len(), 1);
        // The filtered list must NOT mention any of the other tools.
        let names: Vec<String> = filtered
            .iter()
            .filter_map(|t| match t {
                ChatCompletionTools::Function(f) => Some(f.function.name.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(names, vec!["talon_list_recent".to_string()]);
    }

    #[test]
    fn openai_tools_from_empty_slice_returns_empty_list() {
        // When the run has no allowed tools, the request should
        // omit `tools`/`tool_choice` entirely. The helper still
        // returns an empty Vec, which the loop interprets as
        // "omit the field".
        let filtered = openai_tools_from(&[]);
        assert!(filtered.is_empty());
    }

    #[test]
    fn execute_tool_not_allowed_returns_ok_false_payload() {
        // Copilot thread #5: previously this returned Err and aborted
        // the whole run. Now it returns an `ok: false` payload so the
        // LLM can try a different tool.
        let tmp = tempfile::TempDir::new().unwrap();
        let engine = bk_engine::Engine::new(tmp.path()).unwrap();
        let allowed = vec!["talon_search".to_string()];
        let call = FunctionCall {
            name: "talon_delete_exchange".to_string(),
            arguments: "{}".to_string(),
        };
        let result = execute(&engine, &allowed, &call).expect("must not error");
        assert_eq!(result.get("ok"), Some(&json!(false)));
        let err = result.get("error").and_then(|v| v.as_str()).unwrap_or("");
        assert!(err.contains("not in the allowed list"), "got: {err}");
    }

    #[test]
    fn execute_unknown_tool_returns_ok_false_payload() {
        // To exercise the "unknown tool" branch we must put the tool
        // name in `allowed_tools` (so the allowlist check passes) and
        // ALSO have the `bk_mcp` registry not know about it.
        let tmp = tempfile::TempDir::new().unwrap();
        let engine = bk_engine::Engine::new(tmp.path()).unwrap();
        let allowed = vec!["talon_does_not_exist".to_string()];
        let call = FunctionCall {
            name: "talon_does_not_exist".to_string(),
            arguments: "{}".to_string(),
        };
        let result = execute(&engine, &allowed, &call).expect("must not error");
        assert_eq!(result.get("ok"), Some(&json!(false)));
        let err = result.get("error").and_then(|v| v.as_str()).unwrap_or("");
        assert!(err.contains("unknown tool"), "got: {err}");
    }

    #[test]
    fn execute_invalid_json_returns_ok_false_payload() {
        // Copilot thread #5 (part 2): bad JSON from the LLM used to
        // abort the run. Now it returns an `ok: false` payload with
        // a parse error so the LLM can retry with valid args.
        let tmp = tempfile::TempDir::new().unwrap();
        let engine = bk_engine::Engine::new(tmp.path()).unwrap();
        let allowed = vec!["talon_search".to_string()];
        let call = FunctionCall {
            name: "talon_search".to_string(),
            arguments: "{ not valid json".to_string(),
        };
        let result = execute(&engine, &allowed, &call).expect("must not error");
        assert_eq!(result.get("ok"), Some(&json!(false)));
        let err = result.get("error").and_then(|v| v.as_str()).unwrap_or("");
        assert!(
            err.contains("invalid JSON") || err.contains("JSON"),
            "got: {err}"
        );
    }
}
