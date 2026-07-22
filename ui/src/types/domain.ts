// Domain DTOs for the §4.3-4.4 Capture UI.
//
// These mirror the Rust-side types from `bk_core` (Project,
// ExchangeMeta) and `app/proxy_handle` / `app::commands`
// (ProxyStatus, ExchangeSummary, ExchangeListPage, ExchangeDetail,
// ProjectMeta). They are hand-rolled to match the serde-derived
// JSON shapes — same pattern as `types/agent.ts` — because the
// surface is small and we don't run a TypeScript generator in
// v0.1.
//
// Wire format notes:
//   * UUIDs are serialized as strings on the wire (the Rust
//     `Id<T>` uses `uuid::Uuid`).
//   * Timestamps are ISO-8601 strings on the wire (the Rust
//     `chrono::DateTime<Utc>` serializes as RFC-3339).
//   * `SocketAddr` is the standard `ip:port` string form (the
//     Rust `std::net::SocketAddr::Display` impl).

import type { ExchangeId, ProjectId } from "./ids";

/**
 * Project metadata returned by `open_project` and the projects
 * list, mirror of `app::commands::ProjectMeta` (see
 * `app/src/commands.rs`). The full `Project` (with
 * `ProjectSettings`) is NOT carried in the UI store — settings
 * are an editor concern, not a list concern.
 */
export interface ProjectMeta {
  readonly id: ProjectId;
  readonly name: string;
  readonly target_host: string;
  /** SQLite file name (e.g. `acme-2026-07-20.db`). */
  readonly db_filename: string;
}

/**
 * Summary row for the exchange list, mirror of
 * `app::commands::ExchangeSummary` (see `app/src/commands.rs`).
 * The full `HttpExchange` (with `Request` / `Response` / `Body`)
 * is loaded lazily via `get_exchange` on detail view — the list
 * only carries this thin row shape.
 *
 * Note: `scope_state` is a string here (matching the Rust
 * `app::commands::ExchangeSummary` shape). The downstream
 * `bk_core::ExchangeMeta` uses a proper enum, but the wire
 * payload of `list_exchanges` ships the loose string. We type
 * it as `ScopeState` and rely on the Rust side to only emit
 * the four known values.
 */
export interface ExchangeSummary {
  readonly id: ExchangeId;
  readonly project_id: ProjectId;
  /** ISO-8601 UTC timestamp. */
  readonly timestamp: string;
  /** Request duration in nanoseconds. */
  readonly duration_ns: number;
  /** Short label like "GET /api/users". */
  readonly summary: string;
  readonly scope_state: ScopeState;
  readonly starred: boolean;
  readonly notes: string;
}

export type ScopeState = "in_scope" | "out_of_scope" | "blocked" | "unscoped";

/**
 * A page of exchanges returned by `list_exchanges`, mirror of
 * `app::commands::ExchangeListPage`. `next_cursor` is `null`
 * when the page is the last one — UI callers loop until they
 * see `null`.
 */
export interface ExchangeListPage {
  readonly items: ExchangeSummary[];
  /** `null` when this is the last page. */
  readonly next_cursor: number | null;
  readonly total_in_page: number;
}

/**
 * Request body shape, mirror of `bk_core::Body` (see
 * `crates/bk-core/src/model.rs`). The `kind` tag discriminates
 * the variant:
 *   - `complete`: the body is fully buffered; `data` is the
 *     raw bytes. **v0.5 wire form:** a base64 string
 *     (e.g. `"aGVsbG8="` for "hello"). **v0.1 wire form (still
 *     accepted for backwards compat):** a JSON array of byte
 *     values, e.g. `[104, 101, 108, 108, 111]`. The Rust
 *     deserializer (the `body_complete_data_serde` module
 *     in `crates/bk-core/src/model.rs`) accepts both; the UI
 *     distinguishes by `typeof data === "string"` (new) vs.
 *     `Array.isArray(data)` (legacy). To decode to bytes,
 *     use the `decodeBodyToBytes` helper in
 *     `InspectorPanel.tsx` (it handles both forms). **Do NOT**
 *     call `new Uint8Array(data)` directly — for the v0.5
 *     string form, that would create a UTF-8 view of the
 *     base64 chars, not the decoded bytes.
 *   - `streaming`: the body is on the wire; only the
 *     `content_length` is known.
 *   - `empty`: no body (e.g., a GET with no payload).
 */
export type ExchangeBody =
  | { readonly kind: "complete"; readonly data: string | readonly number[] }
  | { readonly kind: "streaming"; readonly content_length: number | null }
  | { readonly kind: "empty" };

/**
 * Request portion of an `ExchangeDetail`, mirror of
 * `bk_core::Request`. `method` is the HTTP method string
 * (e.g. `"GET"`); `url` is the full request URL; `version`
 * is the protocol version (e.g. `"HTTP/1.1"`); `headers`
 * is the header map (lowercased keys, single-string values
 * per the §4.6 Rust serde shape).
 */
export interface ExchangeRequest {
  readonly method: string;
  readonly url: string;
  readonly version: string;
  readonly headers: Readonly<Record<string, string>>;
  readonly body: ExchangeBody;
}

/**
 * Response portion of an `ExchangeDetail`, mirror of
 * `bk_core::Response`. The `status` is a 3-digit HTTP code
 * (or `null` for an in-flight request — the wrapper sees
 * `ExchangeDetail.response` as nullable in that case).
 */
export interface ExchangeResponse {
  readonly version: string;
  readonly status: number;
  readonly status_text: string;
  readonly headers: Readonly<Record<string, string>>;
  readonly body: ExchangeBody;
}

/**
 * Metadata block of an `ExchangeDetail`, mirror of
 * `bk_core::ExchangeMeta`. The `id` and `project_id` are
 * the same UUIDs the list view sees (so a click in the list
 * can correlate to a detail row).
 */
export interface ExchangeDetailMeta {
  readonly id: ExchangeId;
  readonly project_id: ProjectId;
  /** ISO-8601 UTC timestamp. */
  readonly timestamp: string;
  readonly duration_ns: number;
  readonly summary: string;
  readonly scope_state: ScopeState;
  readonly notes: string;
  readonly starred: boolean;
}

/**
 * Full exchange detail returned by `get_exchange`, mirror of
 * `app::commands::ExchangeDetail = bk_core::HttpExchange`
 * (see `app/src/commands.rs` and
 * `crates/bk-core/src/model.rs`). The Rust side serializes
 * the whole `HttpExchange` — request + response bodies
 * included — so the UI does not need a second round-trip to
 * render the inspector. `response` is `null` for an
 * in-flight request or a blocked one (the
 * `blocked_reason` field explains why).
 */
export interface ExchangeDetail {
  readonly meta: ExchangeDetailMeta;
  readonly request: ExchangeRequest;
  readonly response: ExchangeResponse | null;
  readonly blocked_reason: string | null;
}

/**
 * `SocketAddr` as it appears on the wire. The Rust
 * `std::net::SocketAddr` serializes via serde as a single
 * `"ip:port"` string (e.g. `"127.0.0.1:8080"`, or
 * `"[::1]:8080"` for IPv6). We mirror that exactly so the
 * IPC payload round-trips cleanly.
 */
export type SocketAddr = string;

/**
 * Proxy status DTO, mirror of
 * `app::proxy_handle::ProxyStatus` (see
 * `app/src/proxy_handle.rs`).
 */
export interface ProxyStatus {
  readonly state: ProxyState;
  readonly listener_addr: SocketAddr | null;
  readonly ca_fingerprint: string | null;
  readonly last_error: string | null;
}

export type ProxyState = "stopped" | "running" | "error";

// ---------------------------------------------------------------------------
// Phase 6 (§6.6 + §6.7) — scope rules and match & replace rules.
// Mirror of `bk_core::ScopeRule` / `bk_core::MatchReplaceRule` /
// `bk_core::MatchAction` / `bk_core::ScopeRuleKind` /
// `bk_core::MatchReplaceTarget`. The Rust types are
// `#[non_exhaustive]` (per the Phase 10 plugin-system design
// contract), so we hand-roll the TS sides to match the v1
// variants exactly. v2 additions will land as new string
// variants here in lockstep with the Rust changes.
// ---------------------------------------------------------------------------

/**
 * Action to apply when a `ScopeRule` matches a URL.
 * Wire form is the `serde(rename_all = "snake_case")` string
 * from `bk_core::MatchAction`.
 */
export type MatchAction = "in_scope" | "out_of_scope" | "block";

/**
 * What part of a URL a `ScopeRule` matches against.
 * Wire form is the `serde(rename_all = "snake_case")` string
 * from `bk_core::ScopeRuleKind`.
 */
export type ScopeRuleKind = "host" | "path_prefix" | "path_regex";

/**
 * Scope rule, mirror of `bk_core::ScopeRule`. The `pattern` is
 * interpreted per `kind` (host equality / `*.` wildcard /
 * path-prefix / path-regex). The `label` is what the UI shows
 * in the rule list. The `priority` is the tiebreak: higher
 * wins; equal priority uses first-declared.
 */
export interface ScopeRule {
  readonly kind: ScopeRuleKind;
  readonly pattern: string;
  readonly action: MatchAction;
  readonly label: string;
  readonly priority: number;
}

/**
 * What part of a request/response a `MatchReplaceRule`
 * rewrites. Wire form is the `serde(rename_all = "snake_case")`
 * string from `bk_core::MatchReplaceTarget`.
 */
export type MatchReplaceTarget =
  | "request_url"
  | "request_header"
  | "request_body"
  | "response_header"
  | "response_body";

/**
 * Match & replace rule, mirror of `bk_core::MatchReplaceRule`.
 * Disabled rules (`enabled: false`) are kept in the list but
 * skipped at apply time. Higher `priority` runs first; equal
 * priority uses input order.
 */
export interface MatchReplaceRule {
  readonly target: MatchReplaceTarget;
  readonly case_insensitive: boolean;
  readonly is_regex: boolean;
  readonly pattern: string;
  readonly replace: string;
  readonly enabled: boolean;
  readonly priority: number;
}
