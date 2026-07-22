//! Phase 5 — Replay Tauri commands.
//!
//! Two commands back the Replay feature:
//!
//! - [`open_replay_tab`]: hydrates a `ReplayTabDescriptor` for
//!   a given captured `ExchangeId`. The UI uses the descriptor
//!   to seed a new `ReplayTab` in the Zustand store. The Rust
//!   side looks up the exchange via `Engine::get_exchange` (the
//!   v0.5 LRU lives in JS-side `exchangeStore.details`; the
//!   Tauri command always goes through the engine, which is
//!   the source of truth).
//! - [`send_replay`]: takes the user's edited request, sends
//!   it to the real upstream, persists the resulting
//!   `HttpExchange` via `Engine::insert_exchange`, and returns
//!   it. A `replay_event` `WireEvent` is emitted so cross-tab
//!   observers can react.
//!
//! Both commands enforce a **1 MB response body cap** (D3 per
//! the v0.5-batch follow-up; the cap was originally optional
//! but shipped in Phase 5 to avoid the 10 MB JSON blob pain
//! in `open_replay_tab` and `send_replay`). Larger bodies are
//! truncated to 1 MB and a `body_truncated: true` flag is
//! set in the descriptor (or the persisted `ExchangeMeta.notes`
//! is annotated in `send_replay`).
//!
//! ## Spec drift corrections (vs. the plan file)
//!
//! The plan's `state.upstream` and `state.active_project`
//! references don't exist; the codebase uses Tauri's
//! `manage()` API with separate `State<'_, EngineArc>` types.
//! The plan's `bk_proxy::upstream::Upstream::send()` doesn't
//! exist (the `bk_proxy::upstream` module is free functions
//! like `build_request` / `forward_request` that work against
//! a `Pool` + `PooledConn`). For Phase 5 v1, `send_replay`
//! uses a fresh `hyper_util::client::legacy::Client` with
//! `HttpConnector` (plain HTTP) — sufficient for the smoke
//! test path in Part B (`http://127.0.0.1:0`); the production
//! HTTPS path with the v0.5 webpki-roots bundle is a v0.5+
//! follow-up that adds the `HttpsConnector` + `Pool` plumbing
//! here.

use std::time::Instant;

use bk_core::{
    Body, ExchangeId, ExchangeMeta, HeaderMap, HttpExchange, ProjectId, Request, Response, Version,
};
use bk_events::{ReplayEvent, ReplayEventKind, WireEventKind};
use bytes::Bytes;
use chrono::Utc;
use http::{Request as HttpRequest, Uri};
use http_body_util::{BodyExt, Full};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tracing::{info, warn};

use crate::commands::core::{is_valid_host_shape, EngineArc};
use crate::wire::{make_wire_event, WireEventSeq, WIRE_EVENT_LABEL};

/// The Tauri webview label. Re-exported here as the canonical
/// label for all `app.emit_to(...)` calls; `agent.rs` and
/// `wire_bus.rs` both have a `pub(crate) const WEBVIEW_LABEL`
/// but this module imports the one from `wire_bus` (the wire
/// event bus is the lowest-level emit site).
const WEBVIEW_LABEL: &str = crate::wire_bus::WEBVIEW_LABEL;

/// Maximum response body size shipped across the IPC bridge.
/// Bodies larger than this are truncated to `MAX_BODY_BYTES`
/// and the `body_truncated: true` flag is set (in the
/// `open_replay_tab` descriptor) so the UI can show a
/// "body too large, fetch on demand" message. Sized to keep
/// the JSON IPC payload under 1.5 MB even with the rest of
/// the exchange metadata.
pub(crate) const MAX_BODY_BYTES: usize = 1024 * 1024; // 1 MB

/// Payload returned by the `open_replay_tab` command. The UI
/// uses this to seed a new `ReplayTab` in the ReplayStore.
///
/// The `original_response` is the source exchange's response
/// at the time of opening — the `ReplayResponseViewer`
/// computes the diff against the latest response on every
/// send.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayTabDescriptor {
    /// The `ExchangeId` of the source exchange the replay tab
    /// was opened from. Stored in the
    /// `ReplayTab.source_exchange_id` field; the `openTab`
    /// action deduplicates by this id.
    pub source_exchange_id: ExchangeId,
    /// The `ProjectId` the source exchange belongs to. Used
    /// by `send_replay` to persist the new exchange in the
    /// right project (no global "active project" state
    /// exists).
    pub project_id: ProjectId,
    /// The request as the user can edit it. The UI
    /// `structuredClone`s this into `ReplayTab.draftRequest`.
    pub request: Request,
    /// The source exchange's response (baseline for diff).
    /// `None` if the source exchange had no response (e.g. a
    /// failed/blocked request).
    pub original_response: Option<Response>,
    /// `true` if the `original_response.body` was truncated
    /// to `MAX_BODY_BYTES`. The UI shows "body too large,
    /// fetch on demand" when this flag is set.
    pub body_truncated: bool,
}

/// `open_replay_tab(exchange_id) -> ReplayTabDescriptor`.
///
/// Looks up the exchange in the engine. Iterates the
/// `open_ids()` (the engine knows which projects are open)
/// and returns the first hit. Returns `Err("exchange not
/// found")` if the id is unknown to any open project.
///
/// **Spec drift note (D3 — 1 MB body cap):** the response
/// body is capped at `MAX_BODY_BYTES` before serialization.
/// The `body_truncated` flag in the descriptor signals the
/// truncation. v0.5+ will lazy-load large bodies via a
/// separate endpoint.
#[tauri::command]
pub async fn open_replay_tab(
    engine: State<'_, EngineArc>,
    exchange_id: String,
) -> Result<ReplayTabDescriptor, String> {
    let id: ExchangeId = exchange_id
        .parse()
        .map_err(|e: uuid::Error| format!("invalid exchange id: {e}"))?;

    for pid in engine.open_ids() {
        match engine.get_exchange(pid, id) {
            Ok(Some(exchange)) => {
                return Ok(build_descriptor(&exchange));
            }
            Ok(None) => continue,
            Err(e) => {
                warn!(project_id = %pid, exchange_id = %id, error = %e,
                      "open_replay_tab: get_exchange failed; trying next project");
                continue;
            }
        }
    }
    Err(format!("exchange {id} not found in any open project"))
}

fn build_descriptor(exchange: &HttpExchange) -> ReplayTabDescriptor {
    let (response, body_truncated) = match exchange.response.as_ref() {
        Some(resp) => {
            let (capped, truncated) = cap_response_body(resp);
            (Some(capped), truncated)
        }
        None => (None, false),
    };
    ReplayTabDescriptor {
        source_exchange_id: exchange.meta.id,
        project_id: exchange.meta.project_id,
        // Reuse the engine's already-deserialized Request;
        // the UI `structuredClone`s it on the JS side for the
        // `draftRequest` field. No copy needed here.
        request: exchange.request.clone(),
        original_response: response,
        body_truncated,
    }
}

fn cap_response_body(response: &Response) -> (Response, bool) {
    let (body, truncated) = match &response.body {
        Body::Complete { data } => {
            if data.len() > MAX_BODY_BYTES {
                let capped = Bytes::copy_from_slice(&data[..MAX_BODY_BYTES]);
                (Body::Complete { data: capped }, true)
            } else {
                (Body::Complete { data: data.clone() }, false)
            }
        }
        // Streaming and Empty bodies are returned as-is; no
        // cap needed (Empty is 0 bytes, Streaming is the body
        // descriptor without the bytes in memory).
        other => (other.clone(), false),
    };
    (
        Response {
            version: response.version,
            status: response.status,
            status_text: response.status_text.clone(),
            headers: response.headers.clone(),
            body,
        },
        truncated,
    )
}

/// `send_replay(project_id, request) -> HttpExchange`.
///
/// Takes the user's edited request directly (Tauri 2's IPC
/// bridge auto-deserializes the `bk_core::Request` struct via
/// its `Deserialize` impl; the v0.5+ refactor of the previous
/// `request_json: String` + `serde_json::from_str` pair),
/// sends it to the real upstream, persists the resulting
/// `HttpExchange` via `Engine::insert_exchange`, and returns
/// it. A `replay_event` `WireEvent` of kind `SendComplete` is
/// emitted; `SendFailed` on error.
///
/// **v0.5+ refactor (Phase 6 Part C, §C-A.3):** the argument
/// is now a `Request` instead of a `String`. Tauri 2's IPC
/// bridge serializes the `bk_core::Request` struct directly
/// (it derives `Deserialize`); the JS side drops the
/// `JSON.stringify` call. The 1 MB response body cap is
/// preserved.
///
/// **Spec drift note (D4 — Upstream API):** the plan's
/// `bk_proxy::upstream::Upstream::send()` doesn't exist. The
/// `bk_proxy::upstream` module is free functions; the
/// per-request send loop uses the `upstream_pool::Pool` +
/// `PooledConn` (which is what the proxy uses in
/// production). For Replay, we use a fresh
/// `hyper_util::client::legacy::Client` with `HttpConnector`
/// (plain HTTP) — sufficient for the smoke test path in
/// Part B (`http://127.0.0.1:0`); the production HTTPS path
/// is a v0.5+ follow-up that adds the `HttpsConnector` +
/// `Pool` plumbing here.
#[tauri::command]
pub async fn send_replay(
    app: AppHandle,
    engine: State<'_, EngineArc>,
    project_id: ProjectId,
    request: Request,
) -> Result<HttpExchange, String> {
    // 1. Validate the URL is non-empty (Tauri's auto-deserialize
    //    handles missing/malformed fields; this catches the
    //    "JSON parsed but URL is empty string" case).
    if request.url.as_str().is_empty() {
        return Err("request has empty URL".to_string());
    }

    // 2. Validate the target host BEFORE sending. The
    //    `is_valid_host_shape` helper is shared with
    //    `open_project` and `start_proxy`; reusing it keeps
    //    the validation logic in one place.
    let host = request
        .host()
        .ok_or_else(|| "request has no host".to_string())?;
    if !is_valid_host_shape(&host) {
        return Err(format!("invalid target host: {host}"));
    }

    // 3. Build the upstream response. We use a fresh
    //    `hyper_util::client::legacy::Client<HttpConnector,
    //    Full<Bytes>>` to send. The 1 MB cap is enforced on
    //    the response side.
    let started = Instant::now();
    let response = match send_upstream(&request, &host).await {
        Ok(r) => r,
        Err(e) => {
            let err = format!("upstream error: {e}");
            emit_failed(&app, project_id, err.clone());
            return Err(err);
        }
    };
    let duration_ns = started.elapsed().as_nanos() as u64;

    // 4. Build the `bk_core::Response` and persist.
    let method = request.method.clone();
    let path = request.url.path();
    let summary = format!("{} {}", method.as_str(), path);
    let notes = if response.body_was_truncated {
        format!(
            "Response body truncated to {} bytes (original was {} bytes).",
            MAX_BODY_BYTES, response.original_body_len
        )
    } else {
        String::new()
    };
    let exchange = HttpExchange {
        meta: ExchangeMeta {
            id: ExchangeId::new(),
            project_id,
            timestamp: Utc::now(),
            duration_ns,
            summary,
            scope_state: bk_core::ScopeState::Unscoped,
            notes,
            starred: false,
        },
        request,
        response: Some(response.bk_response),
        blocked_reason: None,
    };
    engine
        .insert_exchange(project_id, &exchange)
        .map_err(|e| format!("persist exchange: {e}"))?;

    // 5. Emit the `replay_event` WireEvent. The
    //    `WireEventSeq` is the process-global counter from
    //    `app::run`'s `manage(WireEventSeq::default())`. The
    //    consumer in `ui/src/lib/ws.ts` will get a new arm
    //    added in Part B; the `#[non_exhaustive]` design
    //    means the TS compile fails until that arm lands.
    let wire_seq = app.state::<WireEventSeq>().inner().clone();
    let wire = make_wire_event(
        &wire_seq,
        WireEventKind::Replay(ReplayEvent {
            tab_id: String::new(), // v0.5: source the tab_id from the JS-side dispatch context
            kind: ReplayEventKind::SendComplete,
            exchange_id: Some(exchange.meta.id),
            error: None,
        }),
        serde_json::json!({
            "exchange_id": exchange.meta.id,
            "project_id": project_id,
        }),
    );
    if let Err(e) = app.emit_to(WEBVIEW_LABEL, WIRE_EVENT_LABEL, &wire) {
        warn!(seq = wire.seq, error = %e, "send_replay: emit wire_event failed");
    }
    info!(
        exchange_id = %exchange.meta.id,
        project_id = %project_id,
        duration_ms = duration_ns / 1_000_000,
        "send_replay: complete"
    );

    Ok(exchange)
}

/// Internal helper: send the request via a fresh
/// `hyper_util::client::legacy::Client<HttpConnector, Full<Bytes>>`
/// and return a `bk_core::Response` with the body capped at
/// `MAX_BODY_BYTES`. The `original_body_len` and
/// `body_was_truncated` fields are returned for the caller to
/// record in `ExchangeMeta.notes`.
struct UpstreamResult {
    bk_response: Response,
    original_body_len: usize,
    body_was_truncated: bool,
}

async fn send_upstream(
    request: &Request,
    host: &str,
) -> Result<UpstreamResult, Box<dyn std::error::Error + Send + Sync>> {
    // 1. Build the `http::Request<Full<Bytes>>` for the
    //    `hyper` client. The URI scheme is `https` for
    //    production; the smoke test in Part B will set the
    //    host to `127.0.0.1` and use a custom listener (the
    //    real upstream of a captured request).
    let path_with_query = request.url.path();
    let path_and_query = if let Some(q) = request.url.query() {
        format!("{path_with_query}?{q}")
    } else {
        path_with_query.to_string()
    };
    let uri: Uri = format!("https://{host}{path_and_query}").parse()?;
    let method = request.method.clone();
    let body_bytes: Bytes = match &request.body {
        Body::Complete { data } => data.clone(),
        _ => Bytes::new(),
    };
    let mut upstream_req = HttpRequest::builder()
        .method(method)
        .uri(uri)
        .body(Full::new(body_bytes))?;
    for (k, v) in &request.headers {
        upstream_req.headers_mut().insert(k, v.clone());
    }

    // 2. Send via `hyper_util::client::legacy::Client`. The
    //    `HttpConnector` is plain HTTP (no TLS); the smoke
    //    test uses a local HTTP listener. The production
    //    HTTPS path is a v0.5+ follow-up.
    let client: Client<HttpConnector, Full<Bytes>> =
        Client::builder(TokioExecutor::new()).build(HttpConnector::new());
    let resp = client.request(upstream_req).await?;
    let (resp_parts, resp_body) = resp.into_parts();
    let collected = resp_body.collect().await?;
    let body_bytes: Bytes = collected.to_bytes();
    let original_body_len = body_bytes.len();
    let body_was_truncated = original_body_len > MAX_BODY_BYTES;
    let capped_body_bytes = if body_was_truncated {
        Bytes::copy_from_slice(&body_bytes[..MAX_BODY_BYTES])
    } else {
        body_bytes
    };
    let bk_response = Response {
        version: Version::HTTP_11,
        status: resp_parts.status.as_u16(),
        status_text: resp_parts
            .status
            .canonical_reason()
            .unwrap_or("")
            .to_string(),
        headers: resp_parts
            .headers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<HeaderMap>(),
        body: Body::Complete {
            data: capped_body_bytes,
        },
    };
    Ok(UpstreamResult {
        bk_response,
        original_body_len,
        body_was_truncated,
    })
}

fn emit_failed(app: &AppHandle, project_id: ProjectId, error: String) {
    let wire_seq = app.state::<WireEventSeq>().inner().clone();
    let wire = make_wire_event(
        &wire_seq,
        WireEventKind::Replay(ReplayEvent {
            tab_id: String::new(),
            kind: ReplayEventKind::SendFailed,
            exchange_id: None,
            error: Some(error.clone()),
        }),
        serde_json::json!({
            "project_id": project_id,
            "error": error,
        }),
    );
    let _ = app.emit_to(WEBVIEW_LABEL, WIRE_EVENT_LABEL, &wire);
}

// ---------------------------------------------------------------------------
// Phase 6 Part C, §C-A.4 — Replay history persistence commands
// ---------------------------------------------------------------------------

/// `list_replay_history(project_id, tab_id) -> Vec<ReplayHistoryEntry>`.
/// Returns every entry for the given tab, ordered by
/// `sequence_within_tab` ASC. Used by the UI's
/// `ReplayStore.openTab` action to rehydrate the tab's
/// in-memory `history` field.
#[tauri::command]
pub fn list_replay_history(
    engine: State<'_, EngineArc>,
    project_id: ProjectId,
    tab_id: String,
) -> Result<Vec<bk_engine::ReplayHistoryEntry>, String> {
    engine
        .list_replay_history(project_id, &tab_id)
        .map_err(|e| format!("list_replay_history failed: {e}"))
}

/// `append_replay_history(project_id, entry) -> ()`. Persists
/// a single replay send event. The UI's `ReplayStore.appendSend`
/// action calls this after the in-memory store update.
#[tauri::command]
pub fn append_replay_history(
    engine: State<'_, EngineArc>,
    project_id: ProjectId,
    entry: bk_engine::ReplayHistoryEntry,
) -> Result<(), String> {
    engine
        .append_replay_history(project_id, &entry)
        .map_err(|e| format!("append_replay_history failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bk_core::{HeaderMap, Method, Url};

    fn small_request() -> Request {
        Request {
            method: Method::GET,
            url: Url::parse("https://example.com/path?x=1").unwrap(),
            version: Version::HTTP_11,
            headers: HeaderMap::new(),
            body: Body::empty(),
        }
    }

    /// `build_descriptor` round-trips the exchange fields
    /// into the descriptor and DOES NOT truncate a small
    /// body.
    #[test]
    fn build_descriptor_does_not_truncate_small_body() {
        let request = small_request();
        let response = Response {
            version: Version::HTTP_11,
            status: 200,
            status_text: "OK".to_string(),
            headers: HeaderMap::new(),
            body: Body::Complete {
                data: Bytes::from_static(b"hello"),
            },
        };
        let exchange = HttpExchange {
            meta: ExchangeMeta {
                id: ExchangeId::new(),
                project_id: ProjectId::new(),
                timestamp: Utc::now(),
                duration_ns: 0,
                summary: "GET /path".to_string(),
                scope_state: bk_core::ScopeState::Unscoped,
                notes: String::new(),
                starred: false,
            },
            request: request.clone(),
            response: Some(response),
            blocked_reason: None,
        };
        let d = build_descriptor(&exchange);
        assert!(!d.body_truncated);
        assert_eq!(d.request.url, request.url);
        assert!(d.original_response.is_some());
    }

    /// `build_descriptor` truncates a > 1 MB body and sets
    /// `body_truncated: true`.
    #[test]
    fn build_descriptor_truncates_large_body() {
        let big: Bytes = Bytes::from(vec![0u8; MAX_BODY_BYTES + 1024]);
        let response = Response {
            version: Version::HTTP_11,
            status: 200,
            status_text: "OK".to_string(),
            headers: HeaderMap::new(),
            body: Body::Complete { data: big },
        };
        let exchange = HttpExchange {
            meta: ExchangeMeta {
                id: ExchangeId::new(),
                project_id: ProjectId::new(),
                timestamp: Utc::now(),
                duration_ns: 0,
                summary: "GET /big".to_string(),
                scope_state: bk_core::ScopeState::Unscoped,
                notes: String::new(),
                starred: false,
            },
            request: small_request(),
            response: Some(response),
            blocked_reason: None,
        };
        let d = build_descriptor(&exchange);
        assert!(d.body_truncated);
        let body_bytes = match d.original_response.unwrap().body {
            Body::Complete { data } => data,
            _ => panic!("expected Complete body"),
        };
        assert_eq!(body_bytes.len(), MAX_BODY_BYTES);
    }

    /// `cap_response_body` is a no-op for empty / streaming
    /// bodies (the `body_truncated` flag stays `false`).
    #[test]
    fn cap_response_body_handles_empty_and_streaming() {
        let resp = Response {
            version: Version::HTTP_11,
            status: 204,
            status_text: "No Content".to_string(),
            headers: HeaderMap::new(),
            body: Body::Empty,
        };
        let (capped, truncated) = cap_response_body(&resp);
        assert!(!truncated);
        matches!(capped.body, Body::Empty);

        let resp = Response {
            version: Version::HTTP_11,
            status: 200,
            status_text: "OK".to_string(),
            headers: HeaderMap::new(),
            body: Body::Streaming {
                content_length: Some(MAX_BODY_BYTES as u64 + 1024),
            },
        };
        let (capped, truncated) = cap_response_body(&resp);
        assert!(!truncated);
        matches!(capped.body, Body::Streaming { .. });
    }

    /// `is_valid_host_shape` is reachable from the replay
    /// module (smoke test for the `pub(crate)` exposure).
    #[test]
    fn is_valid_host_shape_is_reachable() {
        assert!(is_valid_host_shape("example.com"));
        assert!(is_valid_host_shape("127.0.0.1"));
        assert!(!is_valid_host_shape(""));
        assert!(!is_valid_host_shape("has space"));
        assert!(!is_valid_host_shape("scheme://x"));
    }
}
