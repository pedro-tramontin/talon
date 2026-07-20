// Tests for the `useAgentStore` (ui/src/state/agent.ts).
//
// §4.3-4.4 MIGRATION:
//   - Old: the store subscribed to the typed `agent_event`
//     Tauri channel via `onAgentEvent(...)` from `ui/src/api.ts`.
//   - New: the store subscribes to the `WireClient.subscribe(
//     'agent_event', ...)` path. The `WireClient` is a
//     singleton accessor (`getWireClient()` / `setWireClient()`)
//     in `ui/src/lib/ws.ts`; tests inject a fake client via
//     `setWireClient(...)` BEFORE the first `getWireClient()`
//     call.
//
// The external API (`startRun`, `cancelRun`, `respondConfirm`,
// the `useAgentStore` selector hook) is UNCHANGED. The tests
// below exercise both the internal WireClient subscribe path
// (the migration) and the unchanged external API.
//
// The subscribe happens lazily in `ensureSubscribed`, called
// from `startRun`. The tests below trigger subscription by
// calling `startRun` once (the Tauri invoke is mocked).

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { agentStore, resetAgentTestState } from "./agent";
import { setWireClient, WireClient } from "../lib/ws";
import type { AgentEvent } from "../types/agent";

// Mock the Tauri IPC so `startRun` can be invoked from the
// tests without a real Tauri runtime. `agentStart` returns a
// run id; the other commands are no-ops.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async (cmd: string) => {
    if (cmd === "agent_start") {
      // Return a fresh run id per call. Tests look at the
      // activeRunId afterward to assert the registration.
      return `run-${Math.random().toString(36).slice(2, 10)}`;
    }
    return undefined;
  }),
}));

vi.mock("@tauri-apps/api/event", () => ({
  // The confirm channels are still typed-listener
  // subscriptions (they're NOT on the wire bus); the mock
  // returns a no-op unlisten so `ensureSubscribed` resolves
  // cleanly.
  listen: vi.fn(async () => () => undefined),
}));

type AgentEventHandler = (payload: unknown) => void;
let capturedHandlers: AgentEventHandler[] = [];

/**
 * Build a fresh `WireClient` with a wrapped `subscribe` that
 * records every `agent_event` registration so tests can fire
 * events by calling the captured handler directly. We don't
 * actually intercept the call — the real WireClient holds its
 * own set too.
 */
function freshWireClient(): WireClient {
  capturedHandlers = [];
  const client = new WireClient();
  const realSubscribe = client.subscribe.bind(client);
  client.subscribe = ((
    kind: Parameters<typeof realSubscribe>[0],
    handler: Parameters<typeof realSubscribe>[1],
  ) => {
    if (kind === "agent_event") {
      capturedHandlers.push(handler as AgentEventHandler);
    }
    return realSubscribe(kind, handler);
  }) as typeof client.subscribe;
  return client;
}

function resetStore() {
  agentStore.setState({
    runs: {},
    activeRunId: null,
    confirmTimeouts: new Map(),
  });
  // Clear the lazy-subscribe state so the next `startRun`
  // re-subscribes against the freshly-installed WireClient
  // (the closure-captured unlistens from the previous
  // client's `subscribe` would otherwise skip the new one).
  resetAgentTestState();
  capturedHandlers = [];
}

/**
 * Start a run and wait for the lazy `ensureSubscribed` to
 * register the wire-bus `agent_event` handler. The
 * `startRun` call awaits `ensureSubscribed` BEFORE the
 * Rust-side `agent_start` is invoked, so by the time
 * `startRun` resolves the handler is registered.
 */
async function startRunAndSubscribe(goal: string): Promise<string> {
  const before = capturedHandlers.length;
  await agentStore.getState().startRun(goal, {
    api_base: "http://localhost:11434/v1",
    api_key: "test",
    model: "qwen2.5-coder:32b",
    max_iterations: 1,
    allowed_tools: [],
  });
  // The wire-bus subscribe happens synchronously inside
  // `ensureSubscribed`; `startRun` awaits it. So once
  // `startRun` resolves, capturedHandlers has grown.
  expect(capturedHandlers.length).toBeGreaterThan(before);
  // The `startRun` set created a new run entry; return its id
  // (it's the activeRunId).
  return agentStore.getState().activeRunId ?? "";
}

/**
 * Simulate the Rust side firing an `agent_event` over the
 * wire bus. We call every captured handler directly with
 * the payload — this is the same path the real WireClient
 * takes (the `dispatch` method invokes each registered
 * handler).
 */
function fireAgentEvent(event: AgentEvent) {
  for (const h of capturedHandlers) h(event);
}

beforeEach(() => {
  resetStore();
  setWireClient(freshWireClient());
});

afterEach(() => {
  resetStore();
  setWireClient(null);
});

describe("useAgentStore (WireClient-migrated)", () => {
  it("starts with no runs and no active id", () => {
    expect(agentStore.getState().runs).toEqual({});
    expect(agentStore.getState().activeRunId).toBeNull();
  });

  it("appends an event to the right run when fired via the wire bus", async () => {
    const runId = await startRunAndSubscribe("test goal");
    expect(runId).not.toBe("");

    fireAgentEvent({
      event: "agent_message",
      agent_id: runId,
      text: "hello from the agent",
    });

    const run = agentStore.getState().runs[runId];
    expect(run).toBeDefined();
    expect(run.events).toHaveLength(1);
    expect(run.events[0].event).toBe("agent_message");
  });

  it("transitions status to 'finished' on agent_finished", async () => {
    const runId = await startRunAndSubscribe("test goal");
    fireAgentEvent({
      event: "agent_finished",
      agent_id: runId,
      answer: "done",
      iterations: 3,
    });
    expect(agentStore.getState().runs[runId].status).toBe("finished");
  });

  it("transitions status to 'cancelled' on the 'cancelled by user' error", async () => {
    const runId = await startRunAndSubscribe("test goal");
    fireAgentEvent({
      event: "agent_error",
      agent_id: runId,
      error: "cancelled by user",
    });
    expect(agentStore.getState().runs[runId].status).toBe("cancelled");
  });

  it("transitions status to 'error' on any other error", async () => {
    const runId = await startRunAndSubscribe("test goal");
    fireAgentEvent({
      event: "agent_error",
      agent_id: runId,
      error: "llm provider 500",
    });
    expect(agentStore.getState().runs[runId].status).toBe("error");
  });

  it("useAgentStore selector returns the requested slice", async () => {
    const runId = await startRunAndSubscribe("hello");
    fireAgentEvent({
      event: "agent_started",
      agent_id: runId,
      goal: "hello",
      model: "qwen2.5-coder:32b",
    });
    // The hook itself is a thin wrapper; we assert the
    // selector behavior on the underlying store.
    const sel = (s: ReturnType<typeof agentStore.getState>) =>
      s.runs[runId]?.events.length ?? 0;
    expect(sel(agentStore.getState())).toBe(1);
  });
});
