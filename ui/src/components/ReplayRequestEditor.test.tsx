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

  // Phase 7 C-B.5: Raw / Pretty sub-tabs + same-tab re-sync.

  it("switching to the Pretty tab with a JSON body renders the JsonTreeView", async () => {
    const req = {
      method: "POST" as const,
      url: "https://example.com/api",
      version: "HTTP/1.1" as const,
      headers: {} as Record<string, string>,
      body: { kind: "complete" as const, data: btoa('{"a":1}') },
    };
    const id = replayStore.getState().openTab({
      exchangeId: newExchangeId(),
      summary: "POST /api",
      request: req,
      response: null,
      projectId: projectStore.getState().activeProjectId!,
    });
    render(<ReplayRequestEditor tabId={id} />);
    await act(async () => {
      fireEvent.click(screen.getByTestId("replay-request-editor-tab-pretty"));
    });
    expect(
      screen.getByTestId("replay-request-editor-pretty-json"),
    ).toBeTruthy();
  });

  it("switching to the Pretty tab with a form-data body renders the key-value table", async () => {
    const req = {
      method: "POST" as const,
      url: "https://example.com/api",
      version: "HTTP/1.1" as const,
      headers: {
        "content-type": "application/x-www-form-urlencoded",
      } as Record<string, string>,
      body: { kind: "complete" as const, data: btoa("a=1&b=hello%20world") },
    };
    const id = replayStore.getState().openTab({
      exchangeId: newExchangeId(),
      summary: "POST /api",
      request: req,
      response: null,
      projectId: projectStore.getState().activeProjectId!,
    });
    render(<ReplayRequestEditor tabId={id} />);
    await act(async () => {
      fireEvent.click(screen.getByTestId("replay-request-editor-tab-pretty"));
    });
    expect(
      screen.getByTestId("replay-request-editor-pretty-form"),
    ).toBeTruthy();
    expect(
      screen.getByTestId("replay-request-editor-pretty-form-row-0"),
    ).toBeTruthy();
  });

  it("switching to the Pretty tab with an unrecognized body shows the fallback message", async () => {
    const req = {
      method: "POST" as const,
      url: "https://example.com/api",
      version: "HTTP/1.1" as const,
      headers: {} as Record<string, string>,
      body: { kind: "complete" as const, data: btoa("just some text") },
    };
    const id = replayStore.getState().openTab({
      exchangeId: newExchangeId(),
      summary: "POST /api",
      request: req,
      response: null,
      projectId: projectStore.getState().activeProjectId!,
    });
    render(<ReplayRequestEditor tabId={id} />);
    await act(async () => {
      fireEvent.click(screen.getByTestId("replay-request-editor-tab-pretty"));
    });
    expect(
      screen.getByTestId("replay-request-editor-pretty-fallback"),
    ).toBeTruthy();
  });

  it("the Raw tab renders the textarea unchanged after a Pretty switch + back", async () => {
    const req = {
      method: "POST" as const,
      url: "https://example.com/api",
      version: "HTTP/1.1" as const,
      headers: {} as Record<string, string>,
      body: { kind: "complete" as const, data: btoa('{"a":1}') },
    };
    const id = replayStore.getState().openTab({
      exchangeId: newExchangeId(),
      summary: "POST /api",
      request: req,
      response: null,
      projectId: projectStore.getState().activeProjectId!,
    });
    render(<ReplayRequestEditor tabId={id} />);
    // The body textarea is shown by default.
    const body = screen.getByTestId(
      "replay-request-editor-body",
    ) as HTMLTextAreaElement;
    expect(body.value).toBe('{"a":1}');
    // Switch to Pretty.
    await act(async () => {
      fireEvent.click(screen.getByTestId("replay-request-editor-tab-pretty"));
    });
    // Back to Raw.
    await act(async () => {
      fireEvent.click(screen.getByTestId("replay-request-editor-tab-raw"));
    });
    // The textarea is back, with the same value.
    const bodyAfter = screen.getByTestId(
      "replay-request-editor-body",
    ) as HTMLTextAreaElement;
    expect(bodyAfter.value).toBe('{"a":1}');
  });

  it("a same-tab setDraft (the 'fork from history' path) re-syncs the textareas", async () => {
    const id = replayStore.getState().openTab({
      exchangeId: newExchangeId(),
      summary: "GET /path",
      request: REQ,
      response: null,
      projectId: projectStore.getState().activeProjectId!,
    });
    render(<ReplayRequestEditor tabId={id} />);
    // Initial state: the body is the base64-decoded "hello".
    const body = screen.getByTestId(
      "replay-request-editor-body",
    ) as HTMLTextAreaElement;
    expect(body.value).toBe("hello");
    // Simulate the "fork from history" path: the
    // ReplayHistoryPanel calls `setDraft` with a new
    // request in the same tab. The same-tab re-sync
    // effect re-syncs the textareas.
    const newReq = {
      method: "POST" as const,
      url: "https://example.com/forked",
      version: "HTTP/1.1" as const,
      headers: { "x-fork": "1" } as Record<string, string>,
      body: { kind: "complete" as const, data: btoa("forked body") },
    };
    await act(async () => {
      replayStore.getState().setDraft(id, newReq);
    });
    await waitFor(() => {
      const bodyAfter = screen.getByTestId(
        "replay-request-editor-body",
      ) as HTMLTextAreaElement;
      expect(bodyAfter.value).toBe("forked body");
    });
    const line = screen.getByTestId(
      "replay-request-editor-line",
    ) as HTMLInputElement;
    expect(line.value).toBe("POST https://example.com/forked");
    const headers = screen.getByTestId(
      "replay-request-editor-headers",
    ) as HTMLTextAreaElement;
    expect(headers.value).toContain("x-fork: 1");
  });

  // v0.5+ post-batch gap-fix P1 #4 (2026-07-24):
  // when the tab was opened from a `body_truncated: true`
  // `openReplayTab` descriptor, the editor renders a
  // "Response body truncated to 1 MB" notice at the top.
  it("renders the body-truncated notice when the tab has bodyTruncated=true", () => {
    const id = replayStore.getState().openTab({
      exchangeId: newExchangeId(),
      summary: "GET /path",
      request: REQ,
      response: null,
      projectId: projectStore.getState().activeProjectId!,
      bodyTruncated: true,
    });
    render(<ReplayRequestEditor tabId={id} />);
    expect(
      screen.getByTestId("replay-request-editor-body-truncated-notice"),
    ).toBeInTheDocument();
  });

  // v0.5+ post-batch gap-fix P1 #4: the notice is
  // HIDDEN when the tab has bodyTruncated=false (the
  // default for cache-hit openTab calls).
  it("does not render the body-truncated notice when bodyTruncated=false", () => {
    const id = replayStore.getState().openTab({
      exchangeId: newExchangeId(),
      summary: "GET /path",
      request: REQ,
      response: null,
      projectId: projectStore.getState().activeProjectId!,
      bodyTruncated: false,
    });
    render(<ReplayRequestEditor tabId={id} />);
    expect(
      screen.queryByTestId("replay-request-editor-body-truncated-notice"),
    ).toBeNull();
  });
});
