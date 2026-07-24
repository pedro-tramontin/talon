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
  // Phase 6 (§6.6 + §6.7) additions.
  ScopeRule,
  ScopeRuleKind,
  MatchAction,
  MatchReplaceRule,
  MatchReplaceTarget,
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
  MatchReplaceRule,
  ScopeRule,
} from "./types/domain";
import type {
  ExchangeDetail,
  ExchangeListPage,
  ExchangeRequest,
  ExchangeResponse,
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

/**
 * `list_projects() -> ProjectMeta[]`. v0.5+ post-batch
 * P3 #9 gap-fix (2026-07-24): the `setProjects` Zustand
 * action was dead code because no Tauri command ever
 * populated the project list from the engine. This
 * wrapper surfaces the new `list_projects` Rust command
 * to the UI; the engine's `Engine::list_open_projects`
 * method returns the open-projects list (newest-first
 * by `created_at`). The UI's `App` startup hook calls
 * this once and pipes the result into
 * `projectStore.setProjects`.
 *
 * **Scope:** only currently-open projects are returned.
 * A "list every project ever opened on this machine"
 * command would require a global registry; that's a
 * later phase.
 */
export async function listProjects(): Promise<ProjectMeta[]> {
  return await invoke<ProjectMeta[]>("list_projects");
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

/**
 * `delete_exchange(project_id, id) -> ()`. v0.6 P3 #9
 * (2026-07-24, delete exchange).
 *
 * The Rust side removes the FTS5 row, CASCADEs the
 * `exchange_tags` join rows, and emits
 * `EngineEvent::ExchangeDeleted { id, project_id }` on
 * the wire bus. The UI's `useEngineEventHandler`
 * (in `routes/Capture.tsx`) wires that event to
 * `exchangeStore.removeExchange(id)`, so the local
 * list updates without an explicit `setExchanges(...)`
 * round-trip.
 *
 * **Caller is responsible for the confirm dialog.**
 * This is a destructive action that cannot be undone.
 * The ExchangeList's × button uses the
 * `<ConfirmDialog>` component to gate the call.
 */
export async function deleteExchange(
  project_id: DomainProjectId,
  id: DomainExchangeId,
): Promise<void> {
  await invoke<void>("delete_exchange", {
    projectId: project_id,
    id,
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

// ---------------------------------------------------------------------------
// Phase 5 — Replay (§5.1, §5.2, §5.3)
// ---------------------------------------------------------------------------

/**
 * Payload returned by `open_replay_tab(exchange_id)`. The UI
 * uses this to seed a new `ReplayTab` in the ReplayStore.
 */
export interface ReplayTabDescriptor {
  readonly source_exchange_id: DomainExchangeId;
  readonly project_id: DomainProjectId;
  readonly request: ExchangeRequest;
  readonly original_response: ExchangeResponse | null;
  readonly body_truncated: boolean;
}

export async function openReplayTab(
  exchangeId: DomainExchangeId,
): Promise<ReplayTabDescriptor> {
  return await invoke<ReplayTabDescriptor>("open_replay_tab", {
    exchangeId,
  });
}

export async function sendReplay(
  projectId: DomainProjectId,
  request: ExchangeRequest,
): Promise<ExchangeDetail> {
  // Phase 6 Part C (§C-A.3): the Rust side now takes the
  // `Request` struct directly (via Tauri 2's auto-deserialize),
  // not a JSON-stringified string. The `JSON.stringify` is
  // dropped; the IPC bridge serializes the `ExchangeRequest`
  // object to the wire shape automatically.
  return await invoke<ExchangeDetail>("send_replay", {
    projectId,
    request,
  });
}


// ---------------------------------------------------------------------------
// Phase 6 Part C (§C-A.4) — replay history persistence.
// ---------------------------------------------------------------------------

/**
 * A single replay history entry. The Rust `ReplayHistoryEntry`
 * struct serializes 1:1 to this shape. The UI's
 * `ReplayStore.openTab` action calls `listReplayHistory` to
 * rehydrate the tab's in-memory `history` field.
 */
export type ReplayHistoryEntry = {
  id: string;
  project_id: string;
  tab_id: string;
  request_exchange_id: string;
  response_exchange_id: string | null;
  timestamp: string;
  sequence_within_tab: number;
};

/**
 * List every replay history entry for a given tab, ordered
 * by `sequence_within_tab` ASC. Returns an empty array for a
 * tab that has no history.
 */
export async function listReplayHistory(
  projectId: DomainProjectId,
  tabId: string,
): Promise<ReplayHistoryEntry[]> {
  return await invoke<ReplayHistoryEntry[]>("list_replay_history", {
    projectId,
    tabId,
  });
}

/**
 * Persist a new replay history entry. The UI's
 * `ReplayStore.appendSend` action calls this after the
 * in-memory store update.
 */
export async function appendReplayHistory(
  projectId: DomainProjectId,
  entry: ReplayHistoryEntry,
): Promise<void> {
  await invoke<void>("append_replay_history", { projectId, entry });
}


// ---------------------------------------------------------------------------
// Phase 6 (§6.2 + §6.7) — scope rules + match & replace rules CRUD.
// ---------------------------------------------------------------------------

/**
 * List the active project's scope rules. Empty array if the
 * project has no rules. The UI's `ScopeRuleEditor` calls this
 * on mount to populate the list.
 */
export async function listScopeRules(
  projectId: DomainProjectId,
): Promise<ScopeRule[]> {
  return await invoke<ScopeRule[]>("list_scope_rules", { projectId });
}

/**
 * Append a new scope rule to the active project. The UI
 * generates the rule client-side (a default with empty
 * pattern) and the backend assigns no ID; the rule is
 * stored at the end of `Project::settings::scope_rules`.
 */
export async function addScopeRule(
  projectId: DomainProjectId,
  rule: ScopeRule,
): Promise<void> {
  await invoke<void>("add_scope_rule", { projectId, rule });
}

/**
 * Remove a scope rule by its index in `Project::settings::scope_rules`.
 * Returns the backend's error string on out-of-bounds (the
 * UI prevents this client-side, but the guard is in the
 * backend too).
 */
export async function removeScopeRule(
  projectId: DomainProjectId,
  index: number,
): Promise<void> {
  await invoke<void>("remove_scope_rule", { projectId, index });
}

/**
 * List the active project's match & replace rules.
 */
export async function listMatchReplaceRules(
  projectId: DomainProjectId,
): Promise<MatchReplaceRule[]> {
  return await invoke<MatchReplaceRule[]>("list_match_replace_rules", {
    projectId,
  });
}

/**
 * Append a new M&R rule.
 */
export async function addMatchReplaceRule(
  projectId: DomainProjectId,
  rule: MatchReplaceRule,
): Promise<void> {
  await invoke<void>("add_match_replace_rule", { projectId, rule });
}

/**
 * Remove an M&R rule by index.
 */
export async function removeMatchReplaceRule(
  projectId: DomainProjectId,
  index: number,
): Promise<void> {
  await invoke<void>("remove_match_replace_rule", { projectId, index });
}
