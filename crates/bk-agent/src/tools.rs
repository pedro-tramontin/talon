//! Tool-calling layer for the agent.
//!
//! Each read-only tool is declared with its OpenAI function-calling
//! schema and a lightweight argument parser.  The dispatch itself
//! reuses `bk_mcp::tools::lookup` so the engine interaction stays
//! identical between the MCP server and the internal agent, but the
//! JSON schema exposed to the LLM is owned here and restricted to the
//! read-only subset by default.

use crate::{AgentError, Result};
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
    TOOL_SCHEMAS
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
pub fn execute(engine: &Engine, allowed_tools: &[String], call: &FunctionCall) -> Result<Value> {
    if !allowed_tools.contains(&call.name) {
        return Err(AgentError::ToolNotAllowed {
            tool: call.name.clone(),
        });
    }

    let args: Value = serde_json::from_str(&call.arguments)
        .map_err(|e| AgentError::Other(anyhow::anyhow!("invalid tool args: {e}")))?;

    let handler = bk_mcp::tools::lookup(&call.name).ok_or_else(|| AgentError::UnknownTool {
        tool: call.name.clone(),
    })?;

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
}
