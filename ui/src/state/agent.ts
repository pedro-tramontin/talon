// Zustand store for agent run state.
//
// In v0.1 the store is a thin wrapper around:
//   - a map of `run_id` -> `AgentRun` (goal, status, events, pendingConfirm)
//   - a per-run setTimeout handle for the 5-min auto-deny on the UI side
//     (mirrors the Rust-side `CONFIRM_TIMEOUT_SECS` in `app/src/agent.rs`).
//
// We use the vanilla `createStore` + `useStore` pattern (rather than
// the React-only `create`) so the store can be subscribed to from
// outside React (e.g. the `onAgentEvent` listener) without dragging
// React into the bridge module.

import { createStore, useStore } from "zustand";
import type { StoreApi } from "zustand/vanilla";
import {
  agentCancel,
  agentConfirmWrite,
  agentStart,
  onAgentEvent,
} from "../api";
import type { AgentConfig, AgentEvent } from "../types/agent";

/**
 * How long a pending write-tool confirmation waits for a user
 * response before the UI auto-denies. MUST match the Rust-side
 * constant `CONFIRM_TIMEOUT_SECS` in `app/src/agent.rs`; both sides
 * have to agree so the modal closes at the same time the LLM gets
 * the "user did not respond" tool result.
 */
export const CONFIRM_TIMEOUT_SECS = 300;

/** Per-run state tracked by the UI. */
export type AgentRun = {
  /** The user-supplied goal. */
  goal: string;
  /** Current run status. `running` shows a Cancel button. */
  status: "running" | "finished" | "error" | "cancelled";
  /** Append-only log of every `AgentEvent` seen for this run. */
  events: AgentEvent[];
  /** A pending write-tool confirmation, if any. */
  pendingConfirm?: { toolName: string; args: unknown; since: number };
};

/** Top-level store shape. */
export type AgentStore = {
  /** All known runs keyed by `run_id`. */
  runs: Record<string, AgentRun>;
  /** The run currently displayed in `AgentPanel`. */
  activeRunId: string | null;
  /** Per-run setTimeout handles for the 5-min auto-deny on the UI side. */
  confirmTimeouts: Map<string, ReturnType<typeof setTimeout>>;
  /** Start a new run. Returns the `run_id` from Rust. */
  startRun: (goal: string, config: AgentConfig) => Promise<void>;
  /** Append a single `AgentEvent` to the right run. */
  handleEvent: (event: AgentEvent) => void;
  /** Respond to a pending confirmation. */
  respondConfirm: (
    runId: string,
    allowed: boolean,
    remember: boolean,
  ) => Promise<void>;
  /** Cancel a running agent. */
  cancelRun: (runId: string) => Promise<void>;
};

/**
 * Build the vanilla store. The `onAgentEvent` subscription is wired
 * lazily on the first `startRun` call (rather than at module init),
 * which avoids TDZ-on-test-import issues with `vi.mock` hoisting and
 * keeps the store testable without mocking the Tauri listen bridge.
 */
function createAgentStore() {
  // Lazily-initialized unlisten for the agent_event subscription.
  // Set by the first `startRun` call; cleared by HMR teardown if
  // the module is re-imported.
  let unlisten: (() => void) | null = null;

  const ensureSubscribed = async () => {
    if (unlisten) return;
    try {
      unlisten = await onAgentEvent((event) => {
        agentStore.getState().handleEvent(event);
      });
    } catch (e) {
      // Surface subscription errors in the console but don't
      // crash the store; the UI just won't get events.
      console.error("failed to subscribe to agent_event:", e);
    }
  };

  const store = createStore<AgentStore>((set, get) => ({
    runs: {},
    activeRunId: null,
    confirmTimeouts: new Map(),

    async startRun(goal, config) {
      // Hard-coded run-context for the v0.1 skeleton: the
      // `App` component fills these in once the project picker
      // lands. For now we ship placeholder values that match
      // the shapes the Rust side expects.
      const runContext = {
        project_name: "default",
        project_id: "00000000-0000-0000-0000-000000000000",
        target_host: "localhost",
      };
      // Wire the agent_event listener lazily (on first run). This
      // avoids subscribing at module init, which is hostile to
      // vi.mock-based test environments.
      void ensureSubscribed();
      const runId = await agentStart(goal, config, runContext);
      set((state) => ({
        runs: {
          ...state.runs,
          [runId]: {
            goal,
            status: "running",
            events: [],
          },
        },
        activeRunId: runId,
      }));
    },

      handleEvent(event) {
        // Every event carries an `agent_id`. We use that as the run
        // key (it's the same UUID the Rust side hands out at start).
        const runId = extractAgentId(event);
        if (!runId) return;
        set((state) => {
          const existing = state.runs[runId];
          if (!existing) {
            // Unknown run — drop. This can happen if the store
            // hasn't been mounted yet (race on page load).
            return {};
          }
          const next: AgentRun = {
            ...existing,
            events: [...existing.events, event],
          };
          // Update status from terminal events.
          if (event.event === "agent_finished") {
            next.status = "finished";
          } else if (event.event === "agent_error") {
            // The "cancelled by user" error from `agent_cancel` is
            // surfaced as a cancelled status; any other error
            // becomes `error`.
            if (event.error === "cancelled by user") {
              next.status = "cancelled";
            } else {
              next.status = "error";
            }
          }
          return {
            runs: { ...state.runs, [runId]: next },
          };
        });
      },

      async respondConfirm(runId, allowed, remember) {
        // Clear the local UI timeout so we don't auto-deny right
        // after the user already responded.
        const t = get().confirmTimeouts.get(runId);
        if (t !== undefined) {
          clearTimeout(t);
          get().confirmTimeouts.delete(runId);
        }
        // Clear the pending confirm optimistically; the
        // `agent_confirm_response` event from Rust will be a
        // no-op if the modal is already gone.
        set((state) => {
          const existing = state.runs[runId];
          if (!existing) return {};
          const { pendingConfirm: _drop, ...rest } = existing;
          return {
            runs: { ...state.runs, [runId]: rest },
          };
        });
        await agentConfirmWrite(runId, allowed, remember);
      },

      async cancelRun(runId) {
        // Clear the UI timeout; the Rust side will wake the
        // pending oneshot with a deny and emit a `cancelled by
        // user` agent_event.
        const t = get().confirmTimeouts.get(runId);
        if (t !== undefined) {
          clearTimeout(t);
          get().confirmTimeouts.delete(runId);
        }
        await agentCancel(runId);
      },
    }));

  return store;
}

/** Extract the run key from an `AgentEvent` (all variants have `agent_id`). */
function extractAgentId(event: AgentEvent): string | null {
  // All variants carry `agent_id`; the discriminated union
  // narrows correctly when we read `.agent_id`.
  if ("agent_id" in event && typeof event.agent_id === "string") {
    return event.agent_id;
  }
  return null;
}

// Singleton store for app-wide use. The `createAgentStore` call is
// idempotent: re-subscribing on HMR replaces the previous unlisten.
export const agentStore: StoreApi<AgentStore> = createAgentStore();

/**
 * React hook for the agent store. Use with a selector to limit
 * re-renders to the slice you care about (e.g.
 * `useAgentStore((s) => s.runs[s.activeRunId ?? ""])`).
 */
export function useAgentStore<T>(selector: (state: AgentStore) => T): T {
  return useStore(agentStore, selector);
}
