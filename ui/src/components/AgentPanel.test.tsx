// Tests for `AgentPanel`. The store is global (a module-level
// singleton), so each test resets it via `agentStore.setState` to
// keep tests isolated. We capture the `listen("agent_event", ...)`
// handler at mock time so we can synthesize events by calling the
// captured handler — that's how we exercise the "fire an event and
// see it in the DOM" path without standing up a real Tauri runtime.

import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AgentPanel } from "./AgentPanel";
import { agentStore } from "../state/agent";
import type { AgentEvent } from "../types/agent";

// Module-level state for the captured listener so we can fire
// events from individual `it` blocks. We re-define this in the mock
// factory below and capture into a top-level holder.
type AgentEventHandler = (event: AgentEvent) => void;
let capturedHandlers: AgentEventHandler[] = [];

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async () => undefined),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (eventName: string, handler: unknown) => {
    // Only capture the agent_event channel — the store's
    // subscription is the one we want to drive from tests.
    if (eventName === "agent_event") {
      capturedHandlers.push(handler as AgentEventHandler);
    }
    // Return a no-op unlisten; tests don't need to clean up.
    return () => undefined;
  }),
}));

function resetStore() {
  // Drop the map of runs and clear any timers the App-level
  // subscribe-into-effect might have installed. The auto-deny
  // subscriber is owned by App, not AgentPanel, so it's only
  // present if App is mounted.
  act(() => {
    agentStore.setState({
      runs: {},
      activeRunId: null,
      confirmTimeouts: new Map(),
    });
  });
  capturedHandlers = [];
}

beforeEach(() => {
  resetStore();
});

afterEach(() => {
  resetStore();
});

describe("AgentPanel", () => {
  it("does not render when there is no active run", () => {
    const { container } = render(<AgentPanel />);
    expect(container.firstChild).toBeNull();
  });

  it("renders the goal and the latest event for an active run", async () => {
    render(<AgentPanel />);
    // Manually populate the store to simulate a started run.
    act(() => {
      agentStore.setState({
        activeRunId: "run-1",
        runs: {
          "run-1": {
            goal: "summarize the last 10 exchanges",
            status: "running",
            events: [],
          },
        },
      });
    });
    expect(
      screen.getByTestId("agent-panel-goal").textContent,
    ).toContain("summarize the last 10 exchanges");

    // Simulate a Tauri agent_event by calling the store's
    // `handleEvent` directly. The auto-subscribe to the Tauri
    // listen channel is lazy (it only fires on the first
    // `startRun` call to avoid TDZ issues in tests), so the
    // canonical way to drive AgentPanel from a test is to
    // call `handleEvent` ourselves.
    act(() => {
      agentStore.getState().handleEvent({
        event: "agent_message",
        agent_id: "run-1",
        text: "hello from the agent",
      });
    });

    await waitFor(() =>
      expect(
        screen.getByTestId("agent-panel-latest").textContent,
      ).toContain("hello from the agent"),
    );
  });

  it("shows the Cancel button while running and hides it after finished", async () => {
    render(<AgentPanel />);
    act(() => {
      agentStore.setState({
        activeRunId: "run-2",
        runs: {
          "run-2": {
            goal: "do a thing",
            status: "running",
            events: [],
          },
        },
      });
    });
    expect(screen.getByTestId("agent-panel-cancel")).toBeInTheDocument();

    // Mark the run as finished; the cancel button should disappear.
    fireEvent.click(screen.getByTestId("agent-panel-cancel"));
    // We don't actually want to cancel — set status to "finished"
    // to assert the button disappears for finished runs.
    act(() => {
      agentStore.setState({
        runs: {
          "run-2": {
            goal: "do a thing",
            status: "finished",
            events: [],
          },
        },
      });
    });
    await waitFor(() =>
      expect(screen.queryByTestId("agent-panel-cancel")).toBeNull(),
    );
  });

  it("renders a ConfirmDialog when a pending confirm is set", () => {
    render(<AgentPanel />);
    act(() => {
      agentStore.setState({
        activeRunId: "run-3",
        runs: {
          "run-3": {
            goal: "delete something",
            status: "running",
            events: [],
            pendingConfirm: {
              toolName: "talon_delete_exchange",
              args: { id: "abc" },
              since: Date.now(),
            },
          },
        },
      });
    });
    expect(screen.getByTestId("confirm-dialog")).toBeInTheDocument();
    expect(
      screen.getByTestId("confirm-dialog-tool-name").textContent,
    ).toBe("talon_delete_exchange");
  });
});
