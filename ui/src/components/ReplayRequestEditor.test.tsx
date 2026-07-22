// Tests for the ReplayRequestEditor (Phase 5 §5.4).
//
// The editor's "Send" button is the load-bearing
// interaction; we mock the Tauri `invoke` to assert
// `sendReplay` is called with the parsed request shape
// and the store is updated.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, fireEvent, act, waitFor } from "@testing-library/react";
import { replayStore } from "../state/replay";
import { exchangeStore } from "../state/exchange";
import { projectStore } from "../state/project";
import * as api from "../api";
import { ReplayRequestEditor } from "./ReplayRequestEditor";
import type { ExchangeId, ProjectId } from "../types/ids";

const REQ = {
  method: "GET" as const,
  url: "https://example.com/path",
  version: "HTTP/1.1" as const,
  headers: {} as Record<string, string>,
  body: { kind: "complete" as const, data: "aGVsbG8=" },
};

function newExchangeId(): ExchangeId {
  return crypto.randomUUID() as ExchangeId;
}

function newProjectId() {
  return crypto.randomUUID() as ProjectId;
}

function resetStores() {
  // Close all replay tabs.
  const replayState = replayStore.getState();
  for (const tab of replayState.tabs) {
    replayState.closeTab(tab.id);
  }
  // Reset exchange store.
  exchangeStore.setState({
    exchanges: [],
    selectedId: null,
    details: new Map(),
    detailsLru: [],
    filter: { text: "", status: "", method: "", tag: "" },
  });
  // Set a known active project.
  projectStore.setState({
    projects: [],
    activeProjectId: newProjectId(),
  });
}

describe("ReplayRequestEditor", () => {
  beforeEach(() => {
    resetStores();
    // Mock the Tauri `invoke`. The store + tests use the
    // public `sendReplay` wrapper; we mock the wrapper to
    // bypass the IPC.
    vi.spyOn(api, "sendReplay").mockResolvedValue({
      meta: {
        id: newExchangeId(),
        project_id: projectStore.getState().activeProjectId!,
        timestamp: new Date().toISOString(),
        duration_ns: 0,
        summary: "GET /path",
        scope_state: "unscoped",
        notes: "",
        starred: false,
      },
      request: REQ,
      response: {
        version: "HTTP/1.1",
        status: 200,
        status_text: "OK",
        headers: {},
        body: { kind: "complete", data: "d29ybGQ=" },
      },
      blocked_reason: null,
    });
  });
  afterEach(() => {
    resetStores();
    cleanup();
    vi.restoreAllMocks();
  });

  it("renders the editor with three textareas + a Send button", () => {
    const id = replayStore.getState().openTab({
      exchangeId: newExchangeId(),
      summary: "GET /path",
      request: REQ,
      response: null,
      projectId: projectStore.getState().activeProjectId!,
    });
    render(<ReplayRequestEditor tabId={id} />);
    expect(screen.getByTestId("replay-request-editor-line")).toBeTruthy();
    expect(
      screen.getByTestId("replay-request-editor-headers"),
    ).toBeTruthy();
    expect(screen.getByTestId("replay-request-editor-body")).toBeTruthy();
    expect(screen.getByTestId("replay-request-editor-send")).toBeTruthy();
  });

  it("clicking Send with a valid request calls sendReplay and updates the store", async () => {
    const id = replayStore.getState().openTab({
      exchangeId: newExchangeId(),
      summary: "GET /path",
      request: REQ,
      response: null,
      projectId: projectStore.getState().activeProjectId!,
    });
    render(<ReplayRequestEditor tabId={id} />);
    const send = screen.getByTestId("replay-request-editor-send");
    await act(async () => {
      fireEvent.click(send);
    });
    await waitFor(() => {
      expect(api.sendReplay).toHaveBeenCalledTimes(1);
    });
    // The tab's history has 1 entry.
    expect(replayStore.getState().tabs[0].history).toHaveLength(1);
    // The tab's latestResponse is the response from the mock.
    expect(replayStore.getState().tabs[0].latestResponse).not.toBeNull();
    // The exchange list has 1 entry (the replay result
    // was prepended via `unshiftExchange`).
    expect(exchangeStore.getState().exchanges).toHaveLength(1);
  });

  it("the `sending` flag disables the Send button while in flight", async () => {
    let resolveSend: ((v: unknown) => void) | null = null;
    vi.spyOn(api, "sendReplay").mockImplementation(
      (() =>
        new Promise((resolve) => {
          resolveSend = resolve;
        })) as unknown as typeof api.sendReplay,
    );
    const id = replayStore.getState().openTab({
      exchangeId: newExchangeId(),
      summary: "GET /path",
      request: REQ,
      response: null,
      projectId: projectStore.getState().activeProjectId!,
    });
    render(<ReplayRequestEditor tabId={id} />);
    const send = screen.getByTestId(
      "replay-request-editor-send",
    ) as HTMLButtonElement;
    act(() => {
      fireEvent.click(send);
    });
    // While the promise is pending, the button is disabled.
    expect(send.disabled).toBe(true);
    expect(
      screen.getByTestId("replay-request-editor-sending"),
    ).toBeTruthy();
    // Resolve the promise and assert the button re-enables.
    await act(async () => {
      resolveSend!({
        meta: {
          id: newExchangeId(),
          project_id: projectStore.getState().activeProjectId!,
          timestamp: new Date().toISOString(),
          duration_ns: 0,
          summary: "GET /path",
          scope_state: "unscoped",
          notes: "",
          starred: false,
        },
        request: REQ,
        response: null,
        blocked_reason: null,
      });
    });
    await waitFor(() => {
      expect(send.disabled).toBe(false);
    });
  });

  it("an error from sendReplay is recorded on the tab's history (response=null)", async () => {
    vi.spyOn(api, "sendReplay").mockRejectedValue(new Error("boom"));
    const id = replayStore.getState().openTab({
      exchangeId: newExchangeId(),
      summary: "GET /path",
      request: REQ,
      response: null,
      projectId: projectStore.getState().activeProjectId!,
    });
    render(<ReplayRequestEditor tabId={id} />);
    const send = screen.getByTestId("replay-request-editor-send");
    await act(async () => {
      fireEvent.click(send);
    });
    await waitFor(() => {
      expect(
        screen.getByTestId("replay-request-editor-error"),
      ).toBeTruthy();
    });
    expect(replayStore.getState().tabs[0].history).toHaveLength(1);
    expect(replayStore.getState().tabs[0].history[0].response).toBeNull();
  });
});
