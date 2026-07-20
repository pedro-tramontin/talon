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
