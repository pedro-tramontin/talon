//! The 20-tool registry for the MCP server.
//!
//! Every tool wraps an `Engine` method. The LLM-visible shape (the
//! JSON input schema) is documented in the design contract and is
//! **stable** for v0.1.
//!
//! Each tool is a `ToolHandler` — a function pointer
//! `fn(&Engine, serde_json::Value) -> Result<serde_json::Value, McpError>`.
//! The [`TOOL_REGISTRY`] constant is the dispatch table.
//!
//! ## Tool categories
//!
//! - **Project lifecycle (3):** `talon_open_project`,
//!   `talon_close_project`, `talon_list_open_projects`
//! - **Exchange CRUD (7):** `talon_insert_exchange`,
//!   `talon_get_exchange`, `talon_list_recent`, `talon_search`,
//!   `talon_update_notes`, `talon_set_starred`,
//!   `talon_delete_exchange`
//! - **Tag CRUD (5):** `talon_upsert_tag`, `talon_list_tags`,
//!   `talon_attach_tag`, `talon_detach_tag`,
//!   `talon_list_tags_for_exchange`
//! - **Phase 3 stub (2):** `talon_proxy_start`, `talon_proxy_stop`
//! - **Phase 7 stub (2):** `talon_fuzz_start`, `talon_fuzz_stop`
//! - **Config (1):** `talon_get_config`
//!
//! Total: 3 + 7 + 5 + 2 + 2 + 1 = 20.

use bk_core::{ExchangeId, ProjectId, TagId};
use bk_engine::Engine;
use bk_store::tags::NewTag;
use serde_json::{json, Value};

use crate::error::McpError;

/// A tool handler: takes the engine + the JSON args the LLM sent,
/// returns the JSON response payload (or an `McpError`).
///
/// The MCP transport (`server.rs`) is responsible for wrapping the
/// returned `Value` in the MCP `content` array shape
/// (`[{ "type": "text", "text": <json string> }, ...]`) and for
/// serializing errors to JSON-RPC error responses.
pub type ToolHandler = fn(&Engine, Value) -> Result<Value, McpError>;

/// The full dispatch table. Indexed by MCP tool name at server
/// startup; unknown tool names at request time are handled
/// directly by `server::run_with_streams` (it returns a
/// `tools/call` response with `isError: true` and a text
/// payload "unknown tool: <name>", without ever invoking this
/// registry — see `server.rs` line ~395 for the dispatch).
pub static TOOL_REGISTRY: &[(&str, ToolHandler)] = &[
    // Project lifecycle
    ("talon_open_project", talon_open_project as ToolHandler),
    ("talon_close_project", talon_close_project),
    ("talon_list_open_projects", talon_list_open_projects),
    // Exchange CRUD
    ("talon_insert_exchange", talon_insert_exchange),
    ("talon_get_exchange", talon_get_exchange),
    ("talon_list_recent", talon_list_recent),
    ("talon_search", talon_search),
    ("talon_update_notes", talon_update_notes),
    ("talon_set_starred", talon_set_starred),
    ("talon_delete_exchange", talon_delete_exchange),
    // Tag CRUD
    ("talon_upsert_tag", talon_upsert_tag),
    ("talon_list_tags", talon_list_tags),
    ("talon_attach_tag", talon_attach_tag),
    ("talon_detach_tag", talon_detach_tag),
    ("talon_list_tags_for_exchange", talon_list_tags_for_exchange),
    // Phase 3 stubs
    ("talon_proxy_start", talon_proxy_start),
    ("talon_proxy_stop", talon_proxy_stop),
    // Phase 7 stubs
    ("talon_fuzz_start", talon_fuzz_start),
    ("talon_fuzz_stop", talon_fuzz_stop),
    // Config
    ("talon_get_config", talon_get_config),
];

/// Look up a tool handler by name. Returns `None` if the name
/// doesn't match any registered tool.
pub fn lookup(name: &str) -> Option<ToolHandler> {
    TOOL_REGISTRY
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, h)| *h)
}

// ---------------------------------------------------------------------------
// Argument-parsing helpers
// ---------------------------------------------------------------------------
//
// Every tool handler starts with the same 3-4 lines: pull a string
// out of the args, parse it as a UUID-typed ID, return
// `McpError::InvalidArgs` on missing/malformed. These helpers keep
// the handler bodies short and consistent.

fn require_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, McpError> {
    args.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::InvalidArgs(format!("{key} required")))
}

fn require_project_id(args: &Value) -> Result<ProjectId, McpError> {
    let s = require_str(args, "project_id")?;
    s.parse::<ProjectId>()
        .map_err(|e| McpError::InvalidArgs(format!("project_id: {e}")))
}

fn require_exchange_id(args: &Value) -> Result<ExchangeId, McpError> {
    let s = require_str(args, "exchange_id")?;
    s.parse::<ExchangeId>()
        .map_err(|e| McpError::InvalidArgs(format!("exchange_id: {e}")))
}

fn require_tag_id(args: &Value) -> Result<TagId, McpError> {
    let s = require_str(args, "tag_id")?;
    s.parse::<TagId>()
        .map_err(|e| McpError::InvalidArgs(format!("tag_id: {e}")))
}

fn optional_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| v.as_str())
}

// ---- Input size caps (defense against LLM-supplied unbounded strings).
// ----
// These caps exist because every string field here crosses a trust
// boundary (the MCP client) and ends up in SQLite + the FTS5 index +
// every subsequent response. A multi-MB string would bloat the DB,
// the FTS index, and the LLM context window. The caps are tuned
// for "legitimate bug-bounty workflow" use; if a real user needs
// more, raise the cap and re-evaluate the FTS5 reindex cost.

const MAX_SUMMARY_LEN: usize = 512;
const MAX_NAME_LEN: usize = 200;
const MAX_NOTES_LEN: usize = 64 * 1024; // 64 KiB
const MAX_COLOR_LEN: usize = 32; // "#rrggbb" or "#rrggbbaa"
const MAX_VERSION_LEN: usize = 32; // "0.1.0" + room for future semver
const MAX_BLOCKED_REASON_LEN: usize = 512; // "scope: out of scope (api.example.com)"

fn require_bounded_str<'a>(
    args: &'a Value,
    key: &str,
    max_len: usize,
) -> Result<&'a str, McpError> {
    let s = require_str(args, key)?;
    if s.len() > max_len {
        return Err(McpError::InvalidArgs(format!(
            "{key} exceeds {max_len} bytes (got {})",
            s.len()
        )));
    }
    Ok(s)
}

/// Optional bounded string: returns `None` if the key is
/// missing, `Some(s)` if the key is present and within the
/// cap, or an `InvalidArgs` error if the key is present and
/// exceeds the cap.
fn optional_bounded_str(
    args: &Value,
    key: &str,
    max_len: usize,
) -> Result<Option<String>, McpError> {
    match args.get(key).and_then(|v| v.as_str()) {
        None => Ok(None),
        Some(s) if s.len() > max_len => Err(McpError::InvalidArgs(format!(
            "{key} exceeds {max_len} bytes (got {})",
            s.len()
        ))),
        Some(s) => Ok(Some(s.to_string())),
    }
}

// ---------------------------------------------------------------------------
// Project lifecycle
// ---------------------------------------------------------------------------

/// `talon_open_project` — `Engine::open_project`.
///
/// Input: `{ project_id, name, target_host }`. The `project_id`
/// is the LLM-supplied UUID; the engine uses it verbatim
/// (idempotent: re-opening the same id refreshes the row but
/// doesn't change the id). The `talon_version` field is
/// optional and defaults to `"0.1.0"`.
pub fn talon_open_project(engine: &Engine, args: Value) -> Result<Value, McpError> {
    let name = require_bounded_str(&args, "name", MAX_NAME_LEN)?.to_string();
    let target_host = require_bounded_str(&args, "target_host", MAX_NAME_LEN)?.to_string();
    let project_id: ProjectId = require_project_id(&args)?;
    let version = optional_bounded_str(&args, "talon_version", MAX_VERSION_LEN)?
        .unwrap_or_else(|| "0.1.0".to_string());
    // Build a default Project (which generates a fresh id, sane
    // timestamps, derived db_filename, default settings), then
    // override the id with the LLM-supplied one.
    let mut project = bk_core::Project::new(name, target_host, version);
    project.info.id = project_id;
    let pool = engine.open_project(&project).map_err(McpError::from)?;
    Ok(json!({
        "project_id": project.info.id.to_string(),
        "db_filename": project.info.db_filename,
        "ok": true,
        // We don't expose the SQLite pool over MCP (it's not
        // serializable). The presence of "ok" is the signal.
        "_pool_open": pool.max_size() > 0,
    }))
}

/// `talon_close_project` — `Engine::close_project`.
pub fn talon_close_project(engine: &Engine, args: Value) -> Result<Value, McpError> {
    let id = require_project_id(&args)?;
    engine.close_project(id);
    Ok(json!({ "ok": true, "project_id": id.to_string() }))
}

/// `talon_list_open_projects` — `Engine::open_ids`.
pub fn talon_list_open_projects(engine: &Engine, _args: Value) -> Result<Value, McpError> {
    let ids: Vec<String> = engine.open_ids().iter().map(|id| id.to_string()).collect();
    Ok(json!({ "ok": true, "project_ids": ids }))
}

// ---------------------------------------------------------------------------
// Exchange CRUD
// ---------------------------------------------------------------------------

/// `talon_insert_exchange` — `Engine::insert_exchange`.
///
/// The full `HttpExchange` is reconstructed from the JSON args
/// (`request` object, optional `response`, `summary`). Bodies are
/// base64 in the LLM-visible shape to keep the JSON parseable.
pub fn talon_insert_exchange(engine: &Engine, args: Value) -> Result<Value, McpError> {
    let project_id = require_project_id(&args)?;
    let request_json = args
        .get("request")
        .ok_or_else(|| McpError::InvalidArgs("request required".into()))?;
    let response_json = args.get("response");
    let summary = require_bounded_str(&args, "summary", MAX_SUMMARY_LEN)?.to_string();
    let blocked_reason = optional_bounded_str(&args, "blocked_reason", MAX_BLOCKED_REASON_LEN)?;

    let request: bk_core::Request = serde_json::from_value(request_json.clone())
        .map_err(|e| McpError::InvalidArgs(format!("request: {e}")))?;
    // Cap the request body at 16 MiB. The proxy's `Body::Streaming`
    // variant handles arbitrarily large uploads, but a tool
    // call that materializes a 1 GB body in the MCP server's
    // address space is a DoS vector.
    if request.body.len() > 16 * 1024 * 1024 {
        return Err(McpError::InvalidArgs(format!(
            "request body exceeds 16 MiB (got {} bytes)",
            request.body.len()
        )));
    }
    let response: Option<bk_core::Response> = match response_json {
        Some(v) => Some(
            serde_json::from_value(v.clone())
                .map_err(|e| McpError::InvalidArgs(format!("response: {e}")))?,
        ),
        None => None,
    };
    if let Some(ref r) = response {
        if r.body.len() > 16 * 1024 * 1024 {
            return Err(McpError::InvalidArgs(format!(
                "response body exceeds 16 MiB (got {} bytes)",
                r.body.len()
            )));
        }
    }

    // v0.6 P2 #6: extract the denormalized fields
    // from the local `request` and `response` so the
    // inserted row carries the right values (the
    // `insert` path will overwrite them anyway, but
    // we keep the in-memory struct consistent for
    // the `ExchangeInserted` event).
    let method = request.method.as_str().to_owned();
    let status = response.as_ref().map(|r| r.status).unwrap_or(0);
    let ex = bk_core::HttpExchange {
        meta: bk_core::ExchangeMeta {
            id: ExchangeId::new(),
            project_id,
            timestamp: chrono::Utc::now(),
            duration_ns: 0,
            summary: summary.clone(),
            scope_state: bk_core::ScopeState::InScope,
            notes: String::new(),
            starred: false,
            method,
            status,
            tags: Vec::new(),
        },
        request,
        response,
        blocked_reason,
    };
    engine
        .insert_exchange(project_id, &ex)
        .map_err(McpError::from)?;
    Ok(json!({
        "ok": true,
        "exchange_id": ex.meta.id.to_string(),
        "summary": summary,
    }))
}

/// `talon_get_exchange` — `Engine::get_exchange`.
pub fn talon_get_exchange(engine: &Engine, args: Value) -> Result<Value, McpError> {
    let project_id = require_project_id(&args)?;
    let id = require_exchange_id(&args)?;
    let ex = engine
        .get_exchange(project_id, id)
        .map_err(McpError::from)?;
    serde_json::to_value(&ex).map_err(McpError::from)
}

/// `talon_list_recent` — `Engine::list_recent`.
pub fn talon_list_recent(engine: &Engine, args: Value) -> Result<Value, McpError> {
    let project_id = require_project_id(&args)?;
    // Cap at 1000 — a single tool response with > 1000 rows
    // would balloon the LLM context and risk OOM. Real use is
    // < 100; 1000 is the "user is bulk-exporting" headroom.
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(50)
        .min(1000) as u32;
    let rows = engine
        .list_recent(project_id, limit)
        .map_err(McpError::from)?;
    Ok(json!({
        "ok": true,
        "count": rows.len(),
        "exchanges": rows,
    }))
}

/// `talon_search` — `Engine::search`.
pub fn talon_search(engine: &Engine, args: Value) -> Result<Value, McpError> {
    let project_id = require_project_id(&args)?;
    let query = require_str(&args, "query")?;
    // Cap the FTS5 query string at 1024 bytes — pathological
    // patterns can DoS the FTS5 parser (deep `NEAR`/parentheses).
    // Legitimate bug-bounty search terms are < 100 bytes.
    if query.len() > 1024 {
        return Err(McpError::InvalidArgs("query exceeds 1024 bytes".into()));
    }
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(10)
        .min(100) as u32;
    let ids = engine
        .search(project_id, query, limit)
        .map_err(McpError::from)?;
    let id_strs: Vec<String> = ids.iter().map(|id| id.to_string()).collect();
    Ok(json!({
        "ok": true,
        "count": id_strs.len(),
        "exchange_ids": id_strs,
    }))
}

/// `talon_update_notes` — `Engine::update_notes`.
pub fn talon_update_notes(engine: &Engine, args: Value) -> Result<Value, McpError> {
    let project_id = require_project_id(&args)?;
    let id = require_exchange_id(&args)?;
    let notes = require_bounded_str(&args, "notes", MAX_NOTES_LEN)?.to_string();
    engine
        .update_notes(project_id, id, &notes)
        .map_err(McpError::from)?;
    Ok(json!({ "ok": true, "exchange_id": id.to_string() }))
}

/// `talon_set_starred` — `Engine::set_starred`.
pub fn talon_set_starred(engine: &Engine, args: Value) -> Result<Value, McpError> {
    let project_id = require_project_id(&args)?;
    let id = require_exchange_id(&args)?;
    let starred = args
        .get("starred")
        .and_then(|v| v.as_bool())
        .ok_or_else(|| McpError::InvalidArgs("starred (bool) required".into()))?;
    engine
        .set_starred(project_id, id, starred)
        .map_err(McpError::from)?;
    Ok(json!({
        "ok": true,
        "exchange_id": id.to_string(),
        "starred": starred,
    }))
}

/// `talon_delete_exchange` — `Engine::delete_exchange`.
pub fn talon_delete_exchange(engine: &Engine, args: Value) -> Result<Value, McpError> {
    let project_id = require_project_id(&args)?;
    let id = require_exchange_id(&args)?;
    engine
        .delete_exchange(project_id, id)
        .map_err(McpError::from)?;
    Ok(json!({ "ok": true, "exchange_id": id.to_string() }))
}

// ---------------------------------------------------------------------------
// Tag CRUD
// ---------------------------------------------------------------------------

/// `talon_upsert_tag` — `Engine::tag_upsert`.
///
/// `tag_upsert` returns the `TagId` (idempotent: same name → same
/// id within a project). We need the full `Tag` (with `name` and
/// `color`) for the response payload, so we look it up via
/// `list_tags`. Cheap (≤ a few tags in v1).
pub fn talon_upsert_tag(engine: &Engine, args: Value) -> Result<Value, McpError> {
    let project_id = require_project_id(&args)?;
    let name = require_bounded_str(&args, "name", MAX_NAME_LEN)?.to_string();
    let color = optional_str(&args, "color")
        .map(|c| {
            if c.len() > MAX_COLOR_LEN {
                return Err(McpError::InvalidArgs(format!(
                    "color exceeds {MAX_COLOR_LEN} bytes"
                )));
            }
            Ok(c.to_string())
        })
        .transpose()?;
    let tag_id = engine
        .tag_upsert(project_id, NewTag { name, color })
        .map_err(McpError::from)?;
    // Resolve the full Tag for the response. list_tags returns all
    // tags in the project; in v1 that's typically < 10, so the
    // O(n) scan is fine. When tag counts grow, switch to a
    // `get_tag` store method (deferred to §3.6 if needed).
    let tags = engine.list_tags(project_id).map_err(McpError::from)?;
    let tag = tags.into_iter().find(|t| t.id == tag_id).ok_or_else(|| {
        McpError::Internal(format!(
            "tag_upsert returned id {tag_id:?} but list_tags did not find it"
        ))
    })?;
    Ok(json!({
        "ok": true,
        "tag_id": tag.id.to_string(),
        "name": tag.name,
        "color": tag.color,
    }))
}

/// `talon_list_tags` — `Engine::list_tags`.
pub fn talon_list_tags(engine: &Engine, args: Value) -> Result<Value, McpError> {
    let project_id = require_project_id(&args)?;
    let tags = engine.list_tags(project_id).map_err(McpError::from)?;
    Ok(json!({
        "ok": true,
        "count": tags.len(),
        "tags": tags,
    }))
}

/// `talon_attach_tag` — `Engine::tag_attach`.
pub fn talon_attach_tag(engine: &Engine, args: Value) -> Result<Value, McpError> {
    let project_id = require_project_id(&args)?;
    let tag_id = require_tag_id(&args)?;
    let exchange_id = require_exchange_id(&args)?;
    engine
        .tag_attach(project_id, tag_id, exchange_id)
        .map_err(McpError::from)?;
    Ok(json!({
        "ok": true,
        "tag_id": tag_id.to_string(),
        "exchange_id": exchange_id.to_string(),
    }))
}

/// `talon_detach_tag` — `Engine::tag_detach`.
pub fn talon_detach_tag(engine: &Engine, args: Value) -> Result<Value, McpError> {
    let project_id = require_project_id(&args)?;
    let tag_id = require_tag_id(&args)?;
    let exchange_id = require_exchange_id(&args)?;
    engine
        .tag_detach(project_id, tag_id, exchange_id)
        .map_err(McpError::from)?;
    Ok(json!({
        "ok": true,
        "tag_id": tag_id.to_string(),
        "exchange_id": exchange_id.to_string(),
    }))
}

/// `talon_list_tags_for_exchange` — `Engine::list_tags_for_exchange`.
pub fn talon_list_tags_for_exchange(engine: &Engine, args: Value) -> Result<Value, McpError> {
    let project_id = require_project_id(&args)?;
    let exchange_id = require_exchange_id(&args)?;
    let tags = engine
        .list_tags_for_exchange(project_id, exchange_id)
        .map_err(McpError::from)?;
    Ok(json!({
        "ok": true,
        "count": tags.len(),
        "tags": tags,
    }))
}

// ---------------------------------------------------------------------------
// Phase 3 stubs (proxy_start, proxy_stop)
// ---------------------------------------------------------------------------
//
// These return `McpError::NotImplemented` per the design-contract
// rationale: the LLM needs a stable tool list, so the v0.1 surface
// already includes the names even though the handlers aren't
// implemented yet. When the real proxy work lands in §3.6/§3.7,
// these handlers get replaced with the real implementations at the
// same tool name (no breaking change for the LLM).

/// `talon_proxy_start` — stub for Phase 3.
pub fn talon_proxy_start(_engine: &Engine, _args: Value) -> Result<Value, McpError> {
    Err(McpError::NotImplemented {
        tool: "talon_proxy_start",
        phase: "3.6",
    })
}

/// `talon_proxy_stop` — stub for Phase 3.
pub fn talon_proxy_stop(_engine: &Engine, _args: Value) -> Result<Value, McpError> {
    Err(McpError::NotImplemented {
        tool: "talon_proxy_stop",
        phase: "3.6",
    })
}

// ---------------------------------------------------------------------------
// Phase 7 stubs (fuzz_start, fuzz_stop)
// ---------------------------------------------------------------------------

/// `talon_fuzz_start` — stub for Phase 7.
pub fn talon_fuzz_start(_engine: &Engine, _args: Value) -> Result<Value, McpError> {
    Err(McpError::NotImplemented {
        tool: "talon_fuzz_start",
        phase: "7",
    })
}

/// `talon_fuzz_stop` — stub for Phase 7.
pub fn talon_fuzz_stop(_engine: &Engine, _args: Value) -> Result<Value, McpError> {
    Err(McpError::NotImplemented {
        tool: "talon_fuzz_stop",
        phase: "7",
    })
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// `talon_get_config` — read the current engine config.
///
/// The `Engine` doesn't expose its config dir as a public method
/// yet, so this returns the engine's open-project count and the
/// set of registered tool names. When Phase 4 lands the config
/// surface, this gets a real implementation.
pub fn talon_get_config(engine: &Engine, _args: Value) -> Result<Value, McpError> {
    Ok(json!({
        "ok": true,
        "open_projects": engine.open_count(),
        "registered_tools": TOOL_REGISTRY.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
    }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use bk_core::ProjectId;
    use tempfile::TempDir;

    fn fresh_engine() -> (TempDir, Engine) {
        let tmp = TempDir::new().unwrap();
        let engine = Engine::new(tmp.path()).unwrap();
        (tmp, engine)
    }

    #[test]
    fn registry_has_exactly_20_tools() {
        // Per the design contract, the v0.1 surface is exactly 20 tools.
        // Adding a tool without a phase-N justification is a breaking change.
        assert_eq!(
            TOOL_REGISTRY.len(),
            20,
            "v0.1 surface must be exactly 20 tools (per design contract). Found: {:?}",
            TOOL_REGISTRY.iter().map(|(n, _)| *n).collect::<Vec<_>>()
        );
    }

    #[test]
    fn registry_names_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for (name, _) in TOOL_REGISTRY {
            assert!(
                seen.insert(*name),
                "duplicate tool name in registry: {name}"
            );
        }
    }

    #[test]
    fn lookup_returns_handler_for_known_tool() {
        let h = lookup("talon_search");
        assert!(h.is_some(), "talon_search must be in the registry");
    }

    #[test]
    fn lookup_returns_none_for_unknown_tool() {
        assert!(lookup("talon_does_not_exist").is_none());
    }

    #[test]
    fn invalid_args_error_message_includes_key() {
        let args = json!({});
        let err = require_str(&args, "project_id").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("project_id"),
            "error message must mention missing key, got: {msg}"
        );
    }

    #[test]
    fn require_project_id_rejects_malformed_uuid() {
        let args = json!({ "project_id": "not-a-uuid" });
        let err = require_project_id(&args).unwrap_err();
        assert!(matches!(err, McpError::InvalidArgs(_)));
    }

    #[test]
    fn require_project_id_accepts_valid_uuid() {
        let id = ProjectId::new();
        let args = json!({ "project_id": id.to_string() });
        let parsed = require_project_id(&args).unwrap();
        assert_eq!(parsed, id);
    }

    /// `require_bounded_str` enforces a max byte length. Without
    /// this cap, a malicious LLM could stuff multi-MB strings
    /// into the `notes`/`summary`/`name` fields and bloat the
    /// SQLite DB + FTS5 index + every subsequent response.
    #[test]
    fn require_bounded_str_rejects_oversize() {
        let args = json!({ "x": "a".repeat(100) });
        // 100 chars is fine when the cap is 200.
        assert!(require_bounded_str(&args, "x", 200).is_ok());
        // 100 chars exceeds a 50-byte cap.
        let err = require_bounded_str(&args, "x", 50).unwrap_err();
        assert!(matches!(err, McpError::InvalidArgs(ref m) if m.contains("exceeds 50 bytes")));
    }

    /// `talon_search` rejects queries over 1024 bytes (defense
    /// against FTS5 DoS via pathological patterns). The
    /// underlying FTS5 parser is a state machine that can
    /// consume non-trivial CPU on adversarial inputs.
    #[test]
    fn talon_search_rejects_oversized_query() {
        let (_tmp, engine) = fresh_engine();
        let project_id = ProjectId::new();
        // Open a project so the engine has somewhere to look.
        let mut project = bk_core::Project::new("test", "example.com", "0.1.0");
        project.info.id = project_id;
        engine.open_project(&project).unwrap();
        let args = json!({
            "project_id": project_id.to_string(),
            "query": "A".repeat(2000),
        });
        let err = talon_search(&engine, args).unwrap_err();
        assert!(matches!(err, McpError::InvalidArgs(ref m) if m.contains("1024")));
    }

    #[test]
    fn open_close_project_round_trip() {
        let (_tmp, engine) = fresh_engine();
        let open_args = json!({
            "project_id": ProjectId::new().to_string(),
            "name": "test",
            "target_host": "example.com",
        });
        let open_result = talon_open_project(&engine, open_args).unwrap();
        assert_eq!(open_result["ok"], json!(true));
        let project_id = open_result["project_id"].as_str().unwrap().to_string();

        let list_result = talon_list_open_projects(&engine, json!({})).unwrap();
        let ids = list_result["project_ids"].as_array().unwrap();
        assert_eq!(ids.len(), 1, "exactly one project should be open");

        let close_args = json!({ "project_id": project_id });
        let close_result = talon_close_project(&engine, close_args).unwrap();
        assert_eq!(close_result["ok"], json!(true));

        let list_after = talon_list_open_projects(&engine, json!({})).unwrap();
        assert_eq!(
            list_after["project_ids"].as_array().unwrap().len(),
            0,
            "project list should be empty after close"
        );
    }

    #[test]
    fn stub_tools_return_not_implemented() {
        let (_tmp, engine) = fresh_engine();
        for (tool, phase) in [
            ("talon_proxy_start", "3.6"),
            ("talon_proxy_stop", "3.6"),
            ("talon_fuzz_start", "7"),
            ("talon_fuzz_stop", "7"),
        ] {
            let h = lookup(tool).expect(tool);
            let err = h(&engine, json!({})).unwrap_err();
            match err {
                McpError::NotImplemented { tool: t, phase: p } => {
                    assert_eq!(t, tool);
                    assert_eq!(p, phase);
                }
                other => panic!("{tool} should return NotImplemented, got: {other:?}"),
            }
        }
    }

    #[test]
    fn get_config_lists_20_tools() {
        let (_tmp, engine) = fresh_engine();
        let result = talon_get_config(&engine, json!({})).unwrap();
        let tools = result["registered_tools"].as_array().unwrap();
        assert_eq!(tools.len(), 20);
    }
}
