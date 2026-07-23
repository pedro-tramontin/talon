// Vitest cases for the `ReplayHistoryPanel` component
// (Phase 5 §5.5 + Phase 7 C-B.5 Fork button).
//
// The panel:
//   - shows the empty state when no history
//   - renders rows in newest-first order
//   - clicking a row calls `setDraft` (load, not auto-send)
//   - clicking the Fork button calls `setDraft` (same
//     effect as the row click; Phase 7 C-B.5 UI affordance)
//
// The test uses the in-memory `replayStore` directly
// (no IPC mock — the panel only reads + writes the store).

import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { replayStore } from "../state/replay";
import { projectStore } from "../state/project";
import { ReplayHistoryPanel } from "./ReplayHistoryPanel";
import type { ExchangeId, ProjectId } from "../types/ids";

const REQ_A = {
  method: "GET" as const,
  url: "https://a.test/api",
  version: "HTTP/1.1" as const,
  headers: {} as Record<string, string>,
  body: { kind: "complete" as const, data: "" },
};
const REQ_B = {
  method: "POST" as const,
  url: "https://b.test/api",
  version: "HTTP/1.1" as const,
  headers: {} as Record<string, string>,
  body: { kind: "complete" as const, data: "" },
};

function newExchangeId(): ExchangeId {
  return crypto.randomUUID() as ExchangeId;
}
function newProjectId(): ProjectId {
  return crypto.randomUUID() as ProjectId;
}

function resetStores() {
  for (const tab of replayStore.getState().tabs) {
    replayStore.getState().closeTab(tab.id);
  }
  projectStore.setState({ projects: [], activeProjectId: newProjectId() });
}

describe("ReplayHistoryPanel", () => {
  beforeEach(() => {
    resetStores();
  });
  afterEach(() => {
    resetStores();
    cleanup();
  });

  it("shows the empty state when the tab has no history", () => {
    const id = replayStore.getState().openTab({
      exchangeId: newExchangeId(),
      summary: "GET /",
      request: REQ_A,
      response: null,
      projectId: projectStore.getState().activeProjectId!,
    });
    render(<ReplayHistoryPanel tabId={id} />);
    expect(
      screen.getByTestId("replay-history-panel-empty"),
    ).toBeTruthy();
  });

  it("renders a row per history entry, newest-first", () => {
    const id = replayStore.getState().openTab({
      exchangeId: newExchangeId(),
      summary: "GET /",
      request: REQ_A,
      response: null,
      projectId: projectStore.getState().activeProjectId!,
    });
    // Append two sends directly to the store.
    act(() => {
      replayStore.getState().appendSend(id, REQ_A, null, null);
      replayStore.getState().appendSend(id, REQ_B, null, null);
    });
    render(<ReplayHistoryPanel tabId={id} />);
    const rows = screen.getAllByTestId("replay-history-panel-row");
    expect(rows).toHaveLength(2);
    // Newest first: the most recently appended entry is at the top.
    expect(rows[0].getAttribute("data-history-index")).toBe("1");
    expect(rows[1].getAttribute("data-history-index")).toBe("0");
  });

  it("clicking the Fork button calls setDraft with the historical request", () => {
    const id = replayStore.getState().openTab({
      exchangeId: newExchangeId(),
      summary: "GET /",
      request: REQ_A,
      response: null,
      projectId: projectStore.getState().activeProjectId!,
    });
    act(() => {
      replayStore.getState().appendSend(id, REQ_B, null, null);
    });
    render(<ReplayHistoryPanel tabId={id} />);
    const fork = screen.getByTestId("replay-history-panel-fork-0");
    act(() => {
      fireEvent.click(fork);
    });
    // The tab's draftRequest is now REQ_B.
    expect(replayStore.getState().tabs[0].draftRequest.url).toBe(
      REQ_B.url,
    );
  });
});
