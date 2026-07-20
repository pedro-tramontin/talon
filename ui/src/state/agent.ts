// Zustand store for agent run state.
//
// In v0.1 the store is a thin wrapper around:
//   - a map of `run_id` -> `AgentRun` (goal, status, events, pendingConfirm)
//   - a per-run setTimeout handle for the 5-min auto-deny on the UI side
//     (mirrors the Rust-side `CONFIRM_TIMEOUT_SECS` in `app/src/agent.rs`).
//
// We use the vanilla `createStore` + `useStore` pattern (rather than
// the React-only `create`) so the store can be subscribed to from
// outside React (e.g. the wire-bus listener) without dragging React
// into the bridge module.
//
// ## §4.3-4.4 migration
//
// The §3.5d version of this store subscribed to the Tauri-typed
// `agent_event` channel via `onAgentEvent(...)` from `ui/src/api.ts`.
// Phase 4 §4.2 introduced a single additive `wire_event` channel
// (the §4.0 `bk_events::WireEvent` envelope, see
// `crates/bk-events/src/lib.rs`) that fan-ins ALL three event
// sources (engine / agent / proxy) into one shape. The §4.3-4.4
// migration switches this store's internal subscription to the
// new `WireClient.subscribe('agent_event', ...)` path. The
// external API (`startRun`, `cancelRun`, `respondConfirm`, the
// `useAgentStore` selector hook) is UNCHANGED — only the
// internal subscribe path changes.
//
// The subscription is wired LAZILY on the first `startRun` call
// (Pitfall #37 from the §3.5d session: no module-level
// `let capturedHandlers`; the wireClient singleton is initialized
// on first use). The WireClient is exported by `ui/src/lib/ws.ts`
// and is connected from `App.tsx` on mount.

import { createStore, useStore } from "zustand";
import type { StoreApi } from "zustand/vanilla";
import {
  agentCancel,
  agentConfirmWrite,
  agentStart,
  onConfirmRequest,
  onConfirmResponse,
} from "../api";
import { getWireClient } from "../lib/ws";
import type {
  AgentConfig,
  AgentEvent,
  ConfirmRequestPayload,
  ConfirmResponsePayload,
} from "../types/agent";

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
  /** Apply a confirmation request from the `agent_confirm_request` channel. */
  handleConfirmRequest: (payload: ConfirmRequestPayload) => void;
  /** Apply a confirmation response from the `agent_confirm_response` channel. */
  handleConfirmResponse: (payload: ConfirmResponsePayload) => void;
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
 * Build the vanilla store. The wire-bus subscription
 * (`wireClient.subscribe('agent_event', ...)`) and the two
 * remaining Tauri-typed subscriptions
 * (`agent_confirm_request`, `agent_confirm_response`) are wired
 * lazily on the first `startRun` call (rather than at module
 * init), which avoids TDZ-on-test-import issues with `vi.mock`
 * hoisting and keeps the store testable without mocking the
 * Tauri listen bridge. `startRun` awaits `ensureSubscribed()`
 * so all three listeners are wired BEFORE the Rust side is
 * asked to emit any events for the new run.
 *
 * The two confirm channels are still typed-listener
 * subscriptions (they're not on the wire bus in §4.2 — the
 * confirm channels are point-to-point RPC patterns, not the
 * fan-in event-bus; they stay on their own dedicated Tauri
 * channels).
 */
/**
 * Module-private function the factory binds its lazy-subscribe
 * reset to. Set by `createAgentStore` on first call (it's
 * per-factory-instance, but the singleton `agentStore` is the
 * only instance in practice). Tests call the exported
 * `resetAgentTestState()` below.
 */
let _resetAgentStateForTests: () => void = () => {};

function createAgentStore() {
  // Lazily-initialized unlistens for the two Tauri event
  // channels we still listen to (confirm request + response).
  // Set by `ensureSubscribed`; cleared by HMR teardown if the
  // module is re-imported.
  let requestUnlisten: (() => void) | null = null;
  let responseUnlisten: (() => void) | null = null;
  // The agent-event unlisten is a closure that detaches the
  // wire-bus handler. Captured on first `ensureSubscribed` so
  // the HMR teardown can call it.
  let agentEventUnsubscribe: (() => void) | null = null;

  /**
   * Test-only: clear the lazy-subscribe state so the next
   * `startRun` re-subscribes to the wire bus + the two
   * confirm channels. Production code never calls this; the
   * §4.3-4.4 vitest harness uses it to keep tests
   * independent. Defined inside the factory so it can
   * close-over the three unlisten references.
   */
  const resetForTests = () => {
    if (agentEventUnsubscribe) {
      agentEventUnsubscribe();
      agentEventUnsubscribe = null;
    }
    if (requestUnlisten) {
      requestUnlisten();
      requestUnlisten = null;
    }
    if (responseUnlisten) {
      responseUnlisten();
      responseUnlisten = null;
    }
  };
  // Register the reset hook at the module level so the
  // exported `resetAgentTestState()` can reach it.
  _resetAgentStateForTests = resetForTests;


  const ensureSubscribed = async () => {
    // The wire bus is the new path for `agent_event`. We
    // subscribe ONCE (the WireClient owns the per-kind handler
    // set) — re-subscribing just adds a duplicate handler.
    if (!agentEventUnsubscribe) {
      try {
        const client = getWireClient();
        agentEventUnsubscribe = client.subscribe(
          "agent_event",
          (payload) => {
            // The wire envelope is type-erased; the Rust
            // `agent_event` payload IS an `AgentEvent` JSON
            // value. We cast and forward.
            agentStore
              .getState()
              .handleEvent(payload as AgentEvent);
          },
        );
      } catch (e) {
        console.error("failed to subscribe wire 'agent_event':", e);
      }
    }
    if (!requestUnlisten) {
      try {
        requestUnlisten = await onConfirmRequest((payload) => {
          agentStore.getState().handleConfirmRequest(payload);
        });
      } catch (e) {
        console.error(
          "failed to subscribe to agent_confirm_request:",
          e,
        );
      }
    }
    if (!responseUnlisten) {
      try {
        responseUnlisten = await onConfirmResponse((payload) => {
          agentStore.getState().handleConfirmResponse(payload);
        });
      } catch (e) {
        console.error(
          "failed to subscribe to agent_confirm_response:",
          e,
        );
      }
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
      // Wire the wire-bus agent_event handler + the two
      // confirm listeners BEFORE asking Rust to start a run.
      // Without this, the Rust side can emit early
      // `agent_event`s (and even terminal events on a fast LLM
      // response) before the listener is attached, losing the
      // first and possibly final events for the run.
      await ensureSubscribed();
      const runId = await agentStart(goal, config, runContext);
      set((state) => {
        // If `handleEvent` already created the run entry (because
        // events raced in before this `set` call), preserve its
        // events and status — only fill in the goal/activeRunId
        // and upgrade the empty `goal` placeholder if necessary.
        const existing = state.runs[runId];
        if (existing) {
          return {
            runs: {
              ...state.runs,
              [runId]: { ...existing, goal: existing.goal || goal },
            },
            activeRunId: runId,
          };
        }
        return {
          runs: {
            ...state.runs,
            [runId]: {
              goal,
              status: "running",
              events: [],
            },
          },
          activeRunId: runId,
        };
      });
    },

    handleEvent(event) {
      // Every event carries an `agent_id`. We use that as the run
      // key (it's the same UUID the Rust side hands out at start).
      const runId = extractAgentId(event);
      if (!runId) return;
      set((state) => {
        const existing = state.runs[runId];
        // "Create on first event": if a run entry doesn't exist
        // yet (because events raced ahead of `agentStart` resolving
        // and `startRun` calling `set`), synthesize one with an
        // empty `goal` placeholder. `startRun` will upgrade the
        // goal when it lands.
        const base: AgentRun = existing ?? {
          goal: "",
          status: "running",
          events: [],
        };
        const next: AgentRun = {
          ...base,
          events: [...base.events, event],
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

    handleConfirmRequest(payload) {
      // The Rust side has asked the WebView to confirm a write
      // tool call. Set the `pendingConfirm` field on the run so
      // the modal appears. If the run entry doesn't exist yet
      // (e.g. the request raced ahead of `startRun`), synthesize
      // a stub — `startRun` will backfill the goal.
      const runId = payload.run_id;
      set((state) => {
        const existing = state.runs[runId];
        const base: AgentRun = existing ?? {
          goal: "",
          status: "running",
          events: [],
        };
        return {
          runs: {
            ...state.runs,
            [runId]: {
              ...base,
              pendingConfirm: {
                toolName: payload.tool_name,
                args: payload.args,
                since: Date.now(),
              },
            },
          },
        };
      });
    },

    handleConfirmResponse(payload) {
      // The Rust side has resolved the confirmation (user
      // answered, timeout fired, or run was cancelled). Clear
      // `pendingConfirm` so the modal closes; the
      // `agent_confirm_response` event is the canonical signal
      // — `respondConfirm` clears the field optimistically and
      // this handler is a no-op if the field is already gone.
      const runId = payload.run_id;
      set((state) => {
        const existing = state.runs[runId];
        if (!existing || !existing.pendingConfirm) return {};
        const { pendingConfirm: _drop, ...rest } = existing;
        return {
          runs: { ...state.runs, [runId]: rest },
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
 * Test-only: reset the lazy-subscribe state so the next
 * `startRun` re-subscribes to the wire bus + the two
 * confirm channels. Production code never calls this; the
 * §4.3-4.4 vitest harness uses it to keep tests
 * independent.
 */
export function resetAgentTestState(): void {
  _resetAgentStateForTests();
}

/**
 * React hook for the agent store. Use with a selector to limit
 * re-renders to the slice you care about (e.g.
 * `useAgentStore((s) => s.runs[s.activeRunId ?? ""])`).
 */
export function useAgentStore<T>(selector: (state: AgentStore) => T): T {
  return useStore(agentStore, selector);
}
