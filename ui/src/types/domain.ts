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
 * Full exchange detail returned by `get_exchange`, mirror of
 * `app::commands::ExchangeDetail`. §4.6 wires the detail view
 * to this shape; for now the Capture route shows the empty
 * state and the right-rail tabs.
 */
export interface ExchangeDetail {
  readonly id: ExchangeId;
  readonly project_id: ProjectId;
  /** ISO-8601 UTC timestamp. */
  readonly timestamp: string;
  readonly duration_ns: number;
  readonly summary: string;
  readonly scope_state: ScopeState;
  readonly starred: boolean;
  readonly notes: string;
  /** Method + URL of the original request. */
  readonly method: string;
  readonly url: string;
  /** Response status code (or `null` for an in-flight request). */
  readonly status: number | null;
}

/**
 * `SocketAddr` as it appears on the wire. The `ip` is a string
 * (v4 or v6 — v6 is bracketed, e.g. `[::1]:8080`).
 */
export interface SocketAddr {
  readonly ip: string;
  readonly port: number;
}

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
