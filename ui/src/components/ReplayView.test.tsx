// Tests for the ReplayView (Phase 5 §5.4 + §5.5).
//
// Mounts the ReplayView with a seeded ReplayStore and
// asserts the empty state, the tab bar, the history panel
// (empty + non-empty), and the request editor's re-sync on
// tab switch.

import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { cleanup, render, screen, fireEvent, act } from "@testing-library/react";
import { replayStore } from "../state/replay";
import { ReplayView } from "./ReplayView";
import type { ExchangeId } from "../types/ids";

const REQ_A = {
  method: "GET",
  url: "https://example.com/a",
  version: "HTTP/1.1",
  headers: {},
  body: { kind: "empty" as const },
};

const REQ_B = {
  method: "POST",
  url: "https://example.com/b",
  version: "HTTP/1.1",
  headers: { "Content-Type": "application/json" },
  body: { kind: "complete" as const, data: "eyJrZXkiOiJ2In0=" },
};

function resetStore() {
  const state = replayStore.getState();
  for (const tab of state.tabs) {
    state.closeTab(tab.id);
  }
}

function newExchangeId(): ExchangeId {
  return crypto.randomUUID() as ExchangeId;
}

describe("ReplayView", () => {
  beforeEach(() => {
    resetStore();
  });
  afterEach(() => {
    resetStore();
    cleanup();
  });

  it("renders the empty state when no tabs are open", () => {
    render(<ReplayView />);
    expect(screen.getByTestId("replay-view-empty")).toBeTruthy();
    expect(
      screen.getByText(/no replay tab open/i),
    ).toBeTruthy();
  });

  it("renders the active tab after openTab", () => {
    const id = replayStore.getState().openTab({
      exchangeId: newExchangeId(),
      summary: "GET /seed",
      request: REQ_A,
      response: null,
    });
    render(<ReplayView />);
    // Tab bar is rendered with one tab.
    const tabs = screen.getAllByTestId("replay-tab-bar-tab");
    expect(tabs).toHaveLength(1);
    expect(tabs[0].getAttribute("data-tab-id")).toBe(id);
  });

  it("clicking a tab in the tab bar activates it", () => {
    const id1 = replayStore.getState().openTab({
      exchangeId: newExchangeId(),
      summary: "tab1",
      request: REQ_A,
      response: null,
    });
    replayStore.getState().openTab({
      exchangeId: newExchangeId(),
      summary: "tab2",
      request: REQ_B,
      response: null,
    });
    expect(replayStore.getState().activeTabId).not.toBe(id1);
    render(<ReplayView />);
    const tabs = screen.getAllByTestId("replay-tab-bar-tab");
    act(() => {
      fireEvent.click(tabs[0]); // click the first tab (id1)
    });
    expect(replayStore.getState().activeTabId).toBe(id1);
  });

  it("the close button removes the tab and clears activeTabId if it was active", () => {
    replayStore.getState().openTab({
      exchangeId: newExchangeId(),
      summary: "tab1",
      request: REQ_A,
      response: null,
    });
    expect(replayStore.getState().activeTabId).not.toBeNull();
    render(<ReplayView />);
    const close = screen.getByTestId("replay-tab-bar-close");
    act(() => {
      fireEvent.click(close);
    });
    expect(replayStore.getState().tabs).toHaveLength(0);
    expect(replayStore.getState().activeTabId).toBeNull();
  });

  it("the request editor re-syncs the textareas on tab switch", () => {
    const id1 = replayStore.getState().openTab({
      exchangeId: newExchangeId(),
      summary: "tab1",
      request: REQ_A,
      response: null,
    });
    replayStore.getState().openTab({
      exchangeId: newExchangeId(),
      summary: "tab2",
      request: REQ_B,
      response: null,
    });
    expect(replayStore.getState().activeTabId).not.toBe(id1);
    render(<ReplayView />);
    // Switch to tab1.
    const tabs = screen.getAllByTestId("replay-tab-bar-tab");
    act(() => {
      fireEvent.click(tabs[0]);
    });
    // The request line textarea should now show REQ_A's
    // method + url.
    const line = screen.getByTestId(
      "replay-request-editor-line",
    ) as HTMLInputElement;
    expect(line.value).toContain("GET");
    expect(line.value).toContain("/a");
  });

  it("the history panel shows the empty-state when the tab has no sends", () => {
    replayStore.getState().openTab({
      exchangeId: newExchangeId(),
      summary: "tab1",
      request: REQ_A,
      response: null,
    });
    render(<ReplayView />);
    expect(screen.getByTestId("replay-history-panel-empty")).toBeTruthy();
    expect(screen.getByText(/no sends yet/i)).toBeTruthy();
  });

  it("the history panel renders the history rows newest-first when sends exist", () => {
    const id = replayStore.getState().openTab({
      exchangeId: newExchangeId(),
      summary: "tab1",
      request: REQ_A,
      response: null,
    });
    // Append 2 sends.
    const r1 = { ...REQ_A, method: "GET" as const, url: "/first" };
    const r2 = { ...REQ_A, method: "POST" as const, url: "/second" };
    replayStore.getState().appendSend(
      id,
      r1,
      { version: "HTTP/1.1", status: 200, status_text: "OK", headers: {}, body: { kind: "empty" as const } },
      null,
    );
    replayStore.getState().appendSend(
      id,
      r2,
      { version: "HTTP/1.1", status: 500, status_text: "Internal Server Error", headers: {}, body: { kind: "empty" as const } },
      null,
    );
    render(<ReplayView />);
    const rows = screen.getAllByTestId("replay-history-panel-row");
    expect(rows).toHaveLength(2);
    // Newest first: data-history-index of the first row is
    // 1 (the second append), the second row is 0.
    expect(rows[0].getAttribute("data-history-index")).toBe("1");
    expect(rows[1].getAttribute("data-history-index")).toBe("0");
  });

  it("clicking a history row loads the request into the editor without sending", () => {
    const id = replayStore.getState().openTab({
      exchangeId: newExchangeId(),
      summary: "tab1",
      request: REQ_A,
      response: null,
    });
    const r1 = { ...REQ_A, method: "PUT" as const, url: "/loaded" };
    replayStore.getState().appendSend(
      id,
      r1,
      null,
      null,
    );
    render(<ReplayView />);
    const row = screen.getByTestId("replay-history-panel-row");
    act(() => {
      fireEvent.click(row);
    });
    // The tab's draftRequest is now the historical request.
    const tab = replayStore.getState().tabs.find((t) => t.id === id)!;
    expect(tab.draftRequest.method).toBe("PUT");
    expect(tab.draftRequest.url).toBe("/loaded");
  });
});
