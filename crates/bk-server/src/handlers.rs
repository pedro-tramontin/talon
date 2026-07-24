//! HTTP handler bodies.
//!
//! These are 1:1 mirrors of the Tauri commands in
//! `app/src/commands/`. The differences (per the
//! `objective:` block):
//!
//! 1. `tauri::State` → `axum::extract::State<AppState>`.
//! 2. `Result<T, String>` → `Result<Json<T>, (StatusCode, String)>`.
//! 3. `app_handle.emit(...)` → `state.ws.broadcast(...)`
//!    (the WS clients get the event as a `WireEvent` JSON).

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use bk_core::scope::{MatchReplaceRule, ScopeRule};
use bk_core::{ExchangeId, ProjectId};
use serde::{Deserialize, Serialize};

use crate::state::AppState;

/// `GET /api/health` → 200 `{"ok":true}`.
pub async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true }))
}

/// `GET /api/projects` → list of project metadata.
pub async fn list_projects(
    State(state): State<AppState>,
) -> Result<Json<Vec<ProjectMeta>>, (StatusCode, String)> {
    let engine = state.store.clone();
    let ids = engine.open_ids();
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        match engine.get_project(id) {
            Ok(p) => out.push(ProjectMeta {
                id: p.info.id,
                name: p.info.name,
                target_host: p.info.target_host,
                db_filename: p.info.db_filename,
            }),
            Err(e) => {
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("list_projects: get_project({id}) failed: {e}"),
                ));
            }
        }
    }
    Ok(Json(out))
}

/// Minimal project metadata (mirrors
/// `app/src/commands/core.rs::ProjectMeta`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMeta {
    pub id: ProjectId,
    pub name: String,
    pub target_host: String,
    pub db_filename: String,
}

/// Query parameters for `GET /api/exchanges`.
#[derive(Debug, Deserialize)]
pub struct ListExchangesQuery {
    pub project_id: ProjectId,
    pub cursor: Option<u64>,
    pub limit: Option<u32>,
}

/// Summary of a single exchange (the list-view row).
///
/// **v0.6 (P2 #6 filter dropdowns, 2026-07-24):** the
/// `method`, `status`, and `tags` fields mirror the
/// Tauri `ExchangeSummary` DTO shape so the
/// browser-mode UI gets the same wire format. The
/// `tags` field is a `Vec<String>` (just the names);
/// the right-rail tag-picker still calls `list_tags`
/// for the full `Tag { id, name, color }` shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExchangeSummary {
    pub id: ExchangeId,
    pub project_id: ProjectId,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub duration_ns: u64,
    pub summary: String,
    pub scope_state: String,
    pub starred: bool,
    pub notes: String,
    /// HTTP method. v0.6 P2 #6.
    pub method: String,
    /// HTTP response status code (0 for blocked). v0.6 P2 #6.
    pub status: u16,
    /// Tag names attached to this exchange. v0.6 P2 #6.
    pub tags: Vec<String>,
}

/// Cursor-paginated list response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExchangeListPage {
    pub items: Vec<ExchangeSummary>,
    pub next_cursor: Option<u64>,
    pub total_in_page: usize,
}

/// `GET /api/exchanges?project_id=...&cursor=...&limit=...`.
///
/// v0.6 P2 #6: calls the new `list_recent_with_meta`
/// engine method so the response carries the `method`,
/// `status`, and `tags` fields. The wire shape is the
/// same as the Tauri `ExchangeSummary` DTO (this handler
/// is the browser-mode mirror).
pub async fn list_exchanges(
    State(state): State<AppState>,
    Query(q): Query<ListExchangesQuery>,
) -> Result<Json<ExchangeListPage>, (StatusCode, String)> {
    let engine = state.store.clone();
    let offset = q.cursor.unwrap_or(0);
    let limit = q.limit.unwrap_or(100).min(1000);
    let fetch = (offset as u32).saturating_add(limit);
    // v0.6 P2 #6: use `list_recent_with_meta` so the
    // new fields are populated (the regular
    // `list_recent` returns `HttpExchange` with
    // `method`/`status` from the row but
    // `tags: Vec::new()`).
    let all = engine
        .list_recent_with_meta(q.project_id, fetch)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("list_exchanges: {e}"),
            )
        })?;
    let start = offset as usize;
    let end = (start + limit as usize).min(all.len());
    let items: Vec<ExchangeSummary> = if start < all.len() {
        all[start..end]
            .iter()
            .map(|m| ExchangeSummary {
                scope_state: format!("{:?}", m.scope_state),
                id: m.id,
                project_id: m.project_id,
                timestamp: m.timestamp,
                duration_ns: m.duration_ns,
                summary: m.summary.clone(),
                starred: m.starred,
                notes: m.notes.clone(),
                // v0.6 P2 #6: pass through the new fields.
                method: m.method.clone(),
                status: m.status,
                tags: m.tags.clone(),
            })
            .collect()
    } else {
        Vec::new()
    };
    let next_cursor = if end < all.len() {
        Some(end as u64)
    } else {
        None
    };
    Ok(Json(ExchangeListPage {
        total_in_page: items.len(),
        items,
        next_cursor,
    }))
}

/// `GET /api/exchanges/:id?project_id=...`.
pub async fn get_exchange(
    State(state): State<AppState>,
    Path(id): Path<ExchangeId>,
    Query(q): Query<ProjectIdQuery>,
) -> Result<Json<bk_core::HttpExchange>, (StatusCode, String)> {
    state
        .store
        .get_exchange(q.project_id, id)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("get_exchange: {e}"),
            )
        })?
        .map(Json)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("exchange {id} not found")))
}

/// `DELETE /api/exchanges/:id?project_id=...`.
///
/// v0.6 P3 #9 (2026-07-24, delete exchange). Browser-mode
/// mirror of the Tauri `delete_exchange` command. The
/// engine's `Engine::delete_exchange` is unchanged from
/// its v0.5 form (it was already in place at HEAD); this
/// handler is the thin HTTP wrapper.
pub async fn delete_exchange(
    State(state): State<AppState>,
    Path(id): Path<ExchangeId>,
    Query(q): Query<ProjectIdQuery>,
) -> Result<StatusCode, (StatusCode, String)> {
    state.store.delete_exchange(q.project_id, id).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("delete_exchange: {e}"),
        )
    })?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct ProjectIdQuery {
    pub project_id: ProjectId,
}

/// Search query (POST body).
#[derive(Debug, Deserialize)]
pub struct SearchRequest {
    pub project_id: ProjectId,
    pub query: String,
    pub limit: Option<u32>,
}

/// `POST /api/exchanges` (search) — body: `{project_id, query,
/// limit?}`.
pub async fn search_exchanges(
    State(state): State<AppState>,
    Json(req): Json<SearchRequest>,
) -> Result<Json<Vec<ExchangeId>>, (StatusCode, String)> {
    if req.query.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "query is empty".to_string()));
    }
    let limit = req.limit.unwrap_or(1000).min(1000);
    state
        .store
        .search(req.project_id, &req.query, limit)
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("search: {e}")))
}

/// `POST /api/proxy/start`. The proxy-control closures are
/// provided by the `app` crate; if they're not configured,
/// return 503.
pub async fn start_proxy(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let start = state.start_proxy.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "proxy not configured".to_string(),
        )
    })?;
    // Pull the active project's rules. v1 uses the first
    // open project (mirrors the Tauri command's behavior).
    let project_id = state.store.open_ids().into_iter().next();
    let (scope_rules, match_replace_rules) = match project_id {
        Some(pid) => match state.store.get_project(pid) {
            Ok(p) => (p.settings.scope_rules, p.settings.match_replace_rules),
            Err(_) => (Vec::new(), Vec::new()),
        },
        None => (Vec::new(), Vec::new()),
    };
    let result = start(scope_rules, match_replace_rules).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("start_proxy: {e}"),
        )
    })?;
    Ok(Json(result))
}

/// `POST /api/proxy/stop`.
pub async fn stop_proxy(State(state): State<AppState>) -> Result<StatusCode, (StatusCode, String)> {
    let stop = state.stop_proxy.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "proxy not configured".to_string(),
        )
    })?;
    stop();
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/proxy/status`.
pub async fn proxy_status(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let status_fn = state.proxy_status.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "proxy not configured".to_string(),
        )
    })?;
    Ok(Json(status_fn()))
}

// ---------------------------------------------------------------------------
// §6.2 — Scope rules CRUD
// ---------------------------------------------------------------------------

/// `GET /api/scope/rules?project_id=...`.
pub async fn list_scope_rules(
    State(state): State<AppState>,
    Query(q): Query<ProjectIdQuery>,
) -> Result<Json<Vec<ScopeRule>>, (StatusCode, String)> {
    let project = state.store.get_project(q.project_id).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("list_scope_rules: {e}"),
        )
    })?;
    Ok(Json(project.settings.scope_rules))
}

/// `POST /api/scope/rules/add` — body: `{project_id, rule}`.
#[derive(Debug, Deserialize)]
pub struct AddScopeRuleRequest {
    pub project_id: ProjectId,
    pub rule: ScopeRule,
}

pub async fn add_scope_rule(
    State(state): State<AppState>,
    Json(req): Json<AddScopeRuleRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let mut project = state.store.get_project(req.project_id).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("add_scope_rule: {e}"),
        )
    })?;
    project.settings.scope_rules.push(req.rule);
    state.store.update_project(project.clone()).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("add_scope_rule persist: {e}"),
        )
    })?;
    state
        .store
        .save_settings(req.project_id, &project.settings)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("add_scope_rule save: {e}"),
            )
        })?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/scope/rules/remove` — body: `{project_id, index}`.
#[derive(Debug, Deserialize)]
pub struct RemoveScopeRuleRequest {
    pub project_id: ProjectId,
    pub index: usize,
}

pub async fn remove_scope_rule(
    State(state): State<AppState>,
    Json(req): Json<RemoveScopeRuleRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let mut project = state.store.get_project(req.project_id).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("remove_scope_rule: {e}"),
        )
    })?;
    if req.index >= project.settings.scope_rules.len() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("index {} out of bounds", req.index),
        ));
    }
    project.settings.scope_rules.remove(req.index);
    state.store.update_project(project.clone()).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("remove_scope_rule persist: {e}"),
        )
    })?;
    state
        .store
        .save_settings(req.project_id, &project.settings)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("remove_scope_rule save: {e}"),
            )
        })?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// §6.7 — Match & replace rules CRUD
// ---------------------------------------------------------------------------

/// `GET /api/scope/match-replace?project_id=...`.
pub async fn list_match_replace_rules(
    State(state): State<AppState>,
    Query(q): Query<ProjectIdQuery>,
) -> Result<Json<Vec<MatchReplaceRule>>, (StatusCode, String)> {
    let project = state
        .store
        .get_project(q.project_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("list_mr: {e}")))?;
    Ok(Json(project.settings.match_replace_rules))
}

/// `POST /api/scope/match-replace/add` — body: `{project_id, rule}`.
#[derive(Debug, Deserialize)]
pub struct AddMatchReplaceRuleRequest {
    pub project_id: ProjectId,
    pub rule: MatchReplaceRule,
}

pub async fn add_match_replace_rule(
    State(state): State<AppState>,
    Json(req): Json<AddMatchReplaceRuleRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let mut project = state
        .store
        .get_project(req.project_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("add_mr: {e}")))?;
    project.settings.match_replace_rules.push(req.rule);
    state.store.update_project(project.clone()).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("add_mr persist: {e}"),
        )
    })?;
    state
        .store
        .save_settings(req.project_id, &project.settings)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("add_mr save: {e}"),
            )
        })?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/scope/match-replace/remove` — body: `{project_id, index}`.
#[derive(Debug, Deserialize)]
pub struct RemoveMatchReplaceRuleRequest {
    pub project_id: ProjectId,
    pub index: usize,
}

pub async fn remove_match_replace_rule(
    State(state): State<AppState>,
    Json(req): Json<RemoveMatchReplaceRuleRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let mut project = state
        .store
        .get_project(req.project_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("remove_mr: {e}")))?;
    if req.index >= project.settings.match_replace_rules.len() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("index {} out of bounds", req.index),
        ));
    }
    project.settings.match_replace_rules.remove(req.index);
    state.store.update_project(project.clone()).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("remove_mr persist: {e}"),
        )
    })?;
    state
        .store
        .save_settings(req.project_id, &project.settings)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("remove_mr save: {e}"),
            )
        })?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Shared test helpers
// ---------------------------------------------------------------------------

/// Build a minimal `bk-server::AppState` for tests. The
/// `start_proxy` / `stop_proxy` / `proxy_status` closures
/// are not configured (the proxy routes return 503 in
/// tests).
#[allow(dead_code)]
pub async fn test_app_state(tmp: &std::path::Path) -> AppState {
    use crate::ws::WsHub;
    use bk_engine::Engine;
    let engine = Arc::new(Engine::new(tmp).expect("engine"));
    AppState::new(engine, WsHub::new(), None, None, None)
}
