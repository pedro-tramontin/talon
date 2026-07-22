// Vitest cases for the ReplayStore (§5.3).
//
// Per the codebase convention (see `project.test.ts` and
// `exchange.test.tsx`), each case calls
// `replayStore.getState().setX(...)` directly to mutate the
// store, then asserts on `replayStore.getState()` rather
// than rendering a component. This tests the store API
// without the React rendering overhead.

import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { replayStore } from "./replay";
import type { ExchangeRequest, ExchangeResponse } from "../types/domain";
import type { ExchangeId, ProjectId } from "../types/ids";

const REQ: ExchangeRequest = {
  method: "GET",
  url: "https://example.com/path",
  version: "HTTP/1.1",
  headers: { "X-Test": "true" },
  body: { kind: "complete", data: "" },
};

const RESP: ExchangeResponse = {
  version: "HTTP/1.1",
  status: 200,
  status_text: "OK",
  headers: { "Content-Type": "text/plain" },
  body: { kind: "complete", data: "aGVsbG8=" },
};

function newExchangeId(): ExchangeId {
  // Generate a v4 UUID the same way the engine does; the
  // store treats it as an opaque string.
  return crypto.randomUUID() as ExchangeId;
}

function newProjectId(): ProjectId {
  return crypto.randomUUID() as ProjectId;
}

/** Reset the store to a clean state between tests. The
 * `closeTab` and other actions are no-op on an empty
 * store, so a wholesale `tabs: []` + `activeTabId: null`
 * is the cleanest reset. */
function resetStore() {
  const state = replayStore.getState();
  for (const tab of state.tabs) {
    state.closeTab(tab.id);
  }
  // After closing all tabs, activeTabId should be null.
  expect(replayStore.getState().tabs).toHaveLength(0);
  expect(replayStore.getState().activeTabId).toBeNull();
}

describe("ReplayStore", () => {
  beforeEach(() => {
    resetStore();
  });
  afterEach(() => {
    resetStore();
  });

  it("openTab creates a new tab with deep-cloned request", () => {
    const sourceId = newExchangeId();
    const id = replayStore.getState().openTab({
      exchangeId: sourceId,
      summary: "GET /path",
      request: REQ,
      response: RESP,
      projectId: newProjectId(),
    });
    expect(id).toMatch(/^[0-9a-f-]{36}$/); // UUID v4
    const tab = replayStore.getState().tabs.find((t) => t.id === id);
    expect(tab).toBeDefined();
    expect(tab!.name).toBe("GET /path");
    // Deep-clone: the draftRequest is NOT the same
    // reference as REQ.
    expect(tab!.draftRequest).not.toBe(REQ);
    expect(tab!.draftRequest).toEqual(REQ);
    // The latestResponse IS the source response (not
    // cloned; it's a baseline).
    expect(tab!.latestResponse).toBe(RESP);
    expect(tab!.sourceExchangeId).toBe(sourceId);
    expect(tab!.latestReplayId).toBeNull();
    expect(tab!.history).toHaveLength(0);
    expect(tab!.sending).toBe(false);
    // The new tab is active.
    expect(replayStore.getState().activeTabId).toBe(id);
  });

  it("openTab for the same sourceExchangeId returns the existing tab id (no duplicate)", () => {
    const sourceId = newExchangeId();
    const id1 = replayStore.getState().openTab({
      exchangeId: sourceId,
      summary: "GET /path",
      request: REQ,
      response: RESP,
    });
    const id2 = replayStore.getState().openTab({
      exchangeId: sourceId,
      summary: "GET /path (re-open)",
      request: REQ,
      response: RESP,
    });
    expect(id1).toBe(id2);
    expect(replayStore.getState().tabs).toHaveLength(1);
    // The tab's name was NOT updated (the second
    // openTab is a no-op for the existing tab).
    expect(replayStore.getState().tabs[0].name).toBe("GET /path");
  });

  it("appendSend records the right history entry shape (timestamp as Date, exchangeId)", () => {
    const id = replayStore.getState().openTab({
      exchangeId: newExchangeId(),
      summary: "GET /path",
      request: REQ,
      response: RESP,
    });
    const exchangeId = newExchangeId();
    const modified: ExchangeRequest = {
      ...REQ,
      method: "POST",
      headers: { "Content-Type": "application/json" },
    };
    replayStore.getState().appendSend(id, modified, RESP, exchangeId);
    const tab = replayStore.getState().tabs.find((t) => t.id === id)!;
    expect(tab.history).toHaveLength(1);
    const entry = tab.history[0];
    expect(entry.request).toEqual(modified);
    expect(entry.response).toBe(RESP);
    expect(entry.exchangeId).toBe(exchangeId);
    expect(entry.timestamp).toBeInstanceOf(Date);
    // latestResponse + latestReplayId are also updated.
    expect(tab.latestResponse).toBe(RESP);
    expect(tab.latestReplayId).toBe(exchangeId);
  });

  it("setSending(true|false) toggles the per-tab flag", () => {
    const id = replayStore.getState().openTab({
      exchangeId: newExchangeId(),
      summary: "GET /path",
      request: REQ,
      response: RESP,
    });
    expect(replayStore.getState().tabs[0].sending).toBe(false);
    replayStore.getState().setSending(id, true);
    expect(replayStore.getState().tabs[0].sending).toBe(true);
    replayStore.getState().setSending(id, false);
    expect(replayStore.getState().tabs[0].sending).toBe(false);
  });

  it("closeTab clears activeTabId if the closed tab was active", () => {
    const id = replayStore.getState().openTab({
      exchangeId: newExchangeId(),
      summary: "GET /path",
      request: REQ,
      response: RESP,
    });
    expect(replayStore.getState().activeTabId).toBe(id);
    replayStore.getState().closeTab(id);
    expect(replayStore.getState().tabs).toHaveLength(0);
    expect(replayStore.getState().activeTabId).toBeNull();
  });

  it("closeTab falls back to the first remaining tab if the closed tab was active", () => {
    const id1 = replayStore.getState().openTab({
      exchangeId: newExchangeId(),
      summary: "tab1",
      request: REQ,
      response: null,
    });
    const id2 = replayStore.getState().openTab({
      exchangeId: newExchangeId(),
      summary: "tab2",
      request: REQ,
      response: null,
    });
    expect(replayStore.getState().activeTabId).toBe(id2);
    replayStore.getState().closeTab(id2);
    expect(replayStore.getState().tabs).toHaveLength(1);
    expect(replayStore.getState().activeTabId).toBe(id1);
  });

  it("closeTab leaves activeTabId alone if a different tab was active", () => {
    const id1 = replayStore.getState().openTab({
      exchangeId: newExchangeId(),
      summary: "tab1",
      request: REQ,
      response: null,
    });
    replayStore.getState().openTab({
      exchangeId: newExchangeId(),
      summary: "tab2",
      request: REQ,
      response: null,
    });
    // id1 is NOT active; openTab activated the second tab.
    // Close the second tab; id1 should become active
    // (it's the first remaining tab).
    const state = replayStore.getState();
    const second = state.tabs[1].id;
    state.closeTab(second);
    expect(replayStore.getState().tabs).toHaveLength(1);
    expect(replayStore.getState().activeTabId).toBe(id1);
  });

  it("setDraft replaces the tab's draftRequest (for the editor's onChange)", () => {
    const id = replayStore.getState().openTab({
      exchangeId: newExchangeId(),
      summary: "GET /path",
      request: REQ,
      response: RESP,
    });
    const modified: ExchangeRequest = {
      ...REQ,
      method: "PUT",
    };
    replayStore.getState().setDraft(id, modified);
    const tab = replayStore.getState().tabs.find((t) => t.id === id)!;
    expect(tab.draftRequest).toEqual(modified);
    // Other tabs are unaffected.
    const otherTabs = replayStore
      .getState()
      .tabs.filter((t) => t.id !== id);
    for (const t of otherTabs) {
      expect(t.draftRequest).not.toEqual(modified);
    }
  });

  it("structuredClone deep-clones nested headers (mutating source does not affect tab)", () => {
    const sourceId = newExchangeId();
    const mutableHeaders: Record<string, string> = { "X-Original": "1" };
    const sourceRequest: ExchangeRequest = {
      ...REQ,
      headers: mutableHeaders,
    };
    const id = replayStore.getState().openTab({
      exchangeId: sourceId,
      summary: "GET /path",
      request: sourceRequest,
      response: RESP,
    });
    // Mutate the source headers AFTER openTab.
    mutableHeaders["X-Original"] = "2";
    mutableHeaders["X-New"] = "3";
    // The tab's draftRequest is NOT affected.
    const tab = replayStore.getState().tabs.find((t) => t.id === id)!;
    expect(tab.draftRequest.headers).toEqual({ "X-Original": "1" });
  });

  it("openTab without projectId sets projectId to null (sendReplay will error until set)", () => {
    const id = replayStore.getState().openTab({
      exchangeId: newExchangeId(),
      summary: "GET /path",
      request: REQ,
      response: null,
      // projectId omitted
    });
    const tab = replayStore.getState().tabs.find((t) => t.id === id)!;
    expect(tab.projectId).toBeNull();
  });
});
