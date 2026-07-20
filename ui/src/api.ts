// Typed wrapper around the Tauri IPC bridge. v0.1 only has `greet`.
// As we add commands, the types here become the contract between
// Rust and the React app.
//
// The `invoke` import is from `@tauri-apps/api/core` in Tauri 2. Older
// guides (Tauri 1) used `@tauri-apps/api/tauri` — that path is gone.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// Re-export the agent type definitions so consumers can import
// them from a single place. The shapes live in `./types/agent.ts`
// (hand-rolled mirrors of the bk-agent Rust schema).
export type {
  AgentConfig,
  AgentEvent,
  ConfirmRequestPayload,
  ConfirmResponsePayload,
} from "./types/agent";

// Re-export the §4.x DTO types from `./types/domain` (ProjectMeta,
// ExchangeSummary, ProxyStatus, ...). Components import these
// through `../api` to keep a single import surface.
export type {
  ProjectMeta,
  ExchangeSummary,
  ExchangeListPage,
  ExchangeDetail,
  SocketAddr,
  ProxyStatus,
  ScopeState,
  ProxyState,
} from "./types/domain";

export interface Greeting {
  message: string;
  version: string;
}

export async function greet(): Promise<Greeting> {
  return await invoke<Greeting>("greet");
}

/**
 * Start an agent run. The Rust side returns a `run_id` (UUID v4)
 * immediately; the actual run streams events over the `agent_event`
 * Tauri event channel. Subscribe to those with `onAgentEvent`.
 */
export async function agentStart(
  goal: string,
  config: import("./types/agent").AgentConfig,
  runContext: {
    project_name: string;
    project_id: string;
    target_host: string;
  },
): Promise<string> {
  return await invoke<string>("agent_start", {
    goal,
    config,
    runContext,
  });
}

/**
 * Respond to a pending write-tool confirmation. The Rust side wakes
 * the waiting `oneshot::Receiver` with the user's choice; on the UI
 * side the modal closes when the corresponding `agent_event` (or
 * `agent_confirm_response`) lands.
 */
export async function agentConfirmWrite(
  runId: string,
  allowed: boolean,
  remember: boolean,
): Promise<void> {
  await invoke<void>("agent_confirm_write", {
    runId,
    allowed,
    remember,
  });
}

/**
 * Cancel a running agent. The Rust side sets the cancel flag; if a
 * confirmation is pending it is woken with a "deny" response.
 */
export async function agentCancel(runId: string): Promise<void> {
  await invoke<void>("agent_cancel", { runId });
}

/**
 * Subscribe to the `agent_event` Tauri event channel. Each
 * `AgentEvent` from the running agent is forwarded here. Returns an
 * `UnlistenFn` that detaches the listener when called.
 */
export function onAgentEvent(
  handler: (event: import("./types/agent").AgentEvent) => void,
): Promise<UnlistenFn> {
  return listen<import("./types/agent").AgentEvent>("agent_event", (e) => {
    handler(e.payload);
  });
}

/**
 * Subscribe to confirmation requests. The handler receives the
 * `ConfirmRequestPayload` for each write tool the agent wants to
 * call. Returns an `UnlistenFn` for cleanup.
 */
export function onConfirmRequest(
  handler: (payload: import("./types/agent").ConfirmRequestPayload) => void,
): Promise<UnlistenFn> {
  return listen<import("./types/agent").ConfirmRequestPayload>(
    "agent_confirm_request",
    (e) => {
      handler(e.payload);
    },
  );
}

/**
 * Subscribe to confirmation responses. The handler receives the
 * `ConfirmResponsePayload` for each resolution (allow, deny, timeout,
 * cancelled) so the modal can close.
 */
export function onConfirmResponse(
  handler: (payload: import("./types/agent").ConfirmResponsePayload) => void,
): Promise<UnlistenFn> {
  return listen<import("./types/agent").ConfirmResponsePayload>(
    "agent_confirm_response",
    (e) => {
      handler(e.payload);
    },
  );
}

// ---------------------------------------------------------------------------
// §4.1 Tauri command wrappers
//
// These mirror the `#[tauri::command]` functions in `app/src/commands.rs`.
// The Rust side returns `Result<T, String>`; we surface the `Err` as a
// thrown `Error` so React code can `await ... .catch(err => ...)` the
// standard way. The OK path is unwrapped to `T` (Tauri's `invoke<T>`
// already does the JSON decode + error-string check).

import type {
  ExchangeId as DomainExchangeId,
  ProjectId as DomainProjectId,
} from "./types/ids";
import type {
  ExchangeDetail,
  ExchangeListPage,
  ProjectMeta,
  ProxyStatus,
} from "./types/domain";

/** `open_project(name, target_host) -> ProjectMeta`. */
export async function openProject(
  name: string,
  target_host: string,
): Promise<ProjectMeta> {
  return await invoke<ProjectMeta>("open_project", {
    name,
    targetHost: target_host,
  });
}

/** `close_project(id) -> ()`. */
export async function closeProject(id: DomainProjectId): Promise<void> {
  await invoke<void>("close_project", { id });
}

/** `list_exchanges(project_id, cursor?, limit?) -> ExchangeListPage`. */
export async function listExchanges(
  project_id: DomainProjectId,
  cursor: number | null = null,
  limit: number | null = null,
): Promise<ExchangeListPage> {
  return await invoke<ExchangeListPage>("list_exchanges", {
    projectId: project_id,
    cursor,
    limit,
  });
}

/** `get_exchange(project_id, id) -> Option<ExchangeDetail>`. */
export async function getExchange(
  project_id: DomainProjectId,
  id: DomainExchangeId,
): Promise<ExchangeDetail | null> {
  return await invoke<ExchangeDetail | null>("get_exchange", {
    projectId: project_id,
    id,
  });
}

/**
 * `update_notes(project_id, id, notes) -> ()`. Persists
 * the per-exchange notes string. The Rust side enforces
 * a 64KB cap; over-cap input is rejected with a
 * user-readable error string. v1 fires this on the
 * NotesPanel's `onBlur`; the panel also exposes a
 * manual "Save" button for the keyboard-driven case.
 */
export async function updateNotes(
  project_id: DomainProjectId,
  id: DomainExchangeId,
  notes: string,
): Promise<void> {
  await invoke<void>("update_notes", {
    projectId: project_id,
    id,
    notes,
  });
}

/** `proxy_status() -> ProxyStatus`. */
export async function proxyStatus(): Promise<ProxyStatus> {
  return await invoke<ProxyStatus>("proxy_status");
}

/** `start_proxy() -> ()`. Idempotent on the Rust side. */
export async function startProxy(): Promise<void> {
  await invoke<void>("start_proxy");
}

/** `stop_proxy() -> ()`. */
export async function stopProxy(): Promise<void> {
  await invoke<void>("stop_proxy");
}

/**
 * `search_exchanges(project_id, query, limit) -> ExchangeId[]`.
 *
 * FTS5 search wrapper. Returns matching exchange IDs ranked
 * by FTS5's BM25 (best first). The React side intersects the
 * returned IDs with the in-memory exchange list to render
 * the filtered rows.
 *
 * Errors:
 * - `"query is empty"` — the query was empty / whitespace.
 * - `"search_exchanges failed: ..."` — any other backend
 *   error (project not open, malformed FTS5 query, etc.).
 */
export async function searchExchanges(
  project_id: DomainProjectId,
  query: string,
  limit: number = 1000,
): Promise<DomainExchangeId[]> {
  return await invoke<DomainExchangeId[]>("search_exchanges", {
    projectId: project_id,
    query,
    limit,
  });
}
