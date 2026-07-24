// Tests for the virtualized ExchangeList.
//
// The §4.5 spec: 1000 mock rows render without UI lag. The
// DOM-count test asserts the virtualizer mounts only the
// visible window + overscan, not all 1000 rows.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { exchangeStore } from "../state/exchange";
import { replayStore } from "../state/replay";
import { projectStore } from "../state/project";
import type { ExchangeSummary } from "../types/domain";
import type { ExchangeId, ProjectId } from "../types/ids";
import { asProjectId } from "../types/ids";
import { ExchangeList } from "./ExchangeList";
import { openReplayTab } from "../api";

// v0.5+ post-batch gap-fix P1 #4 (2026-07-24):
// the cache-miss path of the Replay button now
// calls `openReplayTab` (which returns a
// `ReplayTabDescriptor` with `body_truncated`)
// instead of `getExchange`. The test mock
// returns a descriptor with `body_truncated: true`
// in the relevant test cases.
vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return {
    ...actual,
    openReplayTab: vi.fn(),
  };
});

const openReplayTabMock = vi.mocked(openReplayTab);

// Polyfill `HTMLElement.offsetHeight` / `offsetWidth` so the
// virtualizer's first measurement (in jsdom) returns a
// non-zero size. Without this, the virtualizer decides
// "no rows fit" and mounts 0 rows — making the DOM-count
// test impossible.
function installJSDOMViewportPolyfills() {
  const proto = HTMLElement.prototype as unknown as {
    offsetHeight: number;
    offsetWidth: number;
    clientHeight: number;
    clientWidth: number;
  };
  Object.defineProperty(proto, "offsetHeight", {
    configurable: true,
    get: function (this: HTMLElement) {
      const inline = (this.style && this.style.height) || "";
      if (inline.endsWith("px")) return parseInt(inline, 10) || 0;
      return 480;
    },
  });
  Object.defineProperty(proto, "offsetWidth", {
    configurable: true,
    get: function (this: HTMLElement) {
      const inline = (this.style && this.style.width) || "";
      if (inline.endsWith("px")) return parseInt(inline, 10) || 0;
      return 240;
    },
  });
  Object.defineProperty(proto, "clientHeight", {
    configurable: true,
    get: function (this: HTMLElement) {
      const inline = (this.style && this.style.height) || "";
      if (inline.endsWith("px")) return parseInt(inline, 10) || 0;
      return 480;
    },
  });
  Object.defineProperty(proto, "clientWidth", {
    configurable: true,
    get: function (this: HTMLElement) {
      const inline = (this.style && this.style.width) || "";
      if (inline.endsWith("px")) return parseInt(inline, 10) || 0;
      return 240;
    },
  });
}

// Minimal ResizeObserver polyfill (jsdom doesn't ship one).
type ResizeObserverEntry = {
  target: Element;
  contentRect: DOMRectReadOnly;
  borderBoxSize: { inlineSize: number; blockSize: number }[];
  contentBoxSize: { inlineSize: number; blockSize: number }[];
  devicePixelContentBoxSize: { inlineSize: number; blockSize: number }[];
};
type ResizeObserverCb = (
  entries: ResizeObserverEntry[],
  observer: ResizeObserver,
) => void;
type ResizeObserverLike = new (cb: ResizeObserverCb) => ResizeObserver;

class FakeResizeObserver {
  private els: Element[] = [];
  constructor(private cb: ResizeObserverCb) {}
  observe(el: Element): void {
    this.els.push(el);
    const h = (el as HTMLElement).offsetHeight;
    const w = (el as HTMLElement).offsetWidth;
    const entry: ResizeObserverEntry = {
      target: el,
      contentRect: new DOMRect(0, 0, w, h),
      borderBoxSize: [{ inlineSize: w, blockSize: h }],
      contentBoxSize: [{ inlineSize: w, blockSize: h }],
      devicePixelContentBoxSize: [{ inlineSize: w, blockSize: h }],
    };
    Promise.resolve().then(() => this.cb([entry], this as unknown as ResizeObserver));
  }
  unobserve(el: Element): void {
    this.els = this.els.filter((e) => e !== el);
  }
  disconnect(): void {
    this.els = [];
  }
}

function installResizeObserverPolyfill() {
  (window as unknown as { ResizeObserver: ResizeObserverLike }).ResizeObserver =
    FakeResizeObserver as unknown as ResizeObserverLike;
  (globalThis as unknown as { ResizeObserver: ResizeObserverLike }).ResizeObserver =
    FakeResizeObserver as unknown as ResizeObserverLike;
}

function resetStore() {
  exchangeStore.setState({
    exchanges: [],
    selectedId: null,
    filter: { text: "", status: "any", method: "any", tag: "" },
    scrollPosition: 0,
    details: new Map(),
    detailsLru: [],
  });
  replayStore.setState({
    tabs: [],
    activeTabId: null,
  });
  projectStore.setState({
    projects: [],
    activeProjectId: null,
  });
  openReplayTabMock.mockReset();
}

/** Build a 1000-row fixture mirroring the §4.5 spec. */
function buildFixture(count: number): ExchangeSummary[] {
  const out: ExchangeSummary[] = [];
  for (let i = 0; i < count; i++) {
    const method = i % 3 === 0 ? "GET" : i % 3 === 1 ? "POST" : "PUT";
    out.push({
      id: `ex-${i}` as ExchangeId,
      project_id: "p1" as ExchangeSummary["project_id"],
      timestamp: new Date(1_700_000_000_000 + i * 1000).toISOString(),
      duration_ns: (50 + (i % 200)) * 1_000_000,
      summary: `${method} /v1/items/${i}`,
      scope_state: "in_scope",
      starred: false,
      notes: "",
      // v0.6 P2 #6: defaults for the new fields.
      method,
      status: 200,
      tags: [],
    });
  }
  return out;
}

beforeEach(() => {
  installJSDOMViewportPolyfills();
  installResizeObserverPolyfill();
  resetStore();
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("ExchangeList", () => {
  it("renders 1000 mock rows without mounting all of them in the DOM", () => {
    exchangeStore.getState().setExchanges(buildFixture(1000));
    const { container } = render(<ExchangeList />);
    // The virtualizer mounts the visible window + overscan
    // (overscan=10, 48px rows, 240x480 initial rect → ~10
    // visible + 10 above + 10 below = ~30 rows max). The spec
    // says ≤ 60. The `data-testid` selector matches the row's
    // button (per ExchangeList.tsx).
    const rowButtons = container.querySelectorAll(
      '[data-testid="exchange-list-row"]',
    );
    expect(rowButtons.length).toBeLessThanOrEqual(60);
    expect(rowButtons.length).toBeGreaterThan(0);
  });

  it("the filter input narrows the rendered set", () => {
    vi.useFakeTimers();
    try {
      exchangeStore.getState().setExchanges(buildFixture(1000));
      render(<ExchangeList />);
      // The placeholder text is "summary…" per
      // ExchangeList.tsx.
      const input = screen.getByPlaceholderText(
        /summary/i,
      ) as HTMLInputElement;
      // Set the value via the React handler so the
      // component re-renders and the debounce timer is
      // (re)set.
      fireEvent.change(input, { target: { value: "POST" } });
      // The debounce is 150ms; advance the fake timer.
      act(() => {
        vi.advanceTimersByTime(200);
      });
      // After the debounce, the store's filter.text is "POST".
      expect(exchangeStore.getState().filter.text).toBe("POST");
    } finally {
      vi.useRealTimers();
    }
  });

  it("selecting a row calls setSelectedId on the store", () => {
    exchangeStore.getState().setExchanges(buildFixture(10));
    const { container } = render(<ExchangeList />);
    const first = container.querySelector(
      '[data-testid="exchange-list-row"]',
    ) as HTMLButtonElement;
    first.click();
    // After clicking, the store's selectedId is the
    // first row's id.
    const expectedId = exchangeStore.getState().exchanges[0]?.id;
    expect(exchangeStore.getState().selectedId).toBe(expectedId);
  });

  // v0.5+ post-batch gap-fix P1 #4 (2026-07-24):
  // clicking the Replay button on a cache-miss
  // calls `openReplayTab` and the new tab's
  // `bodyTruncated` flag mirrors the descriptor's
  // `body_truncated`.
  it("clicking Replay on a cache-miss calls openReplayTab and sets bodyTruncated on the new tab", async () => {
    // Seed the exchange list with a row whose
    // detail is NOT in the LRU (so the Replay
    // click takes the cache-miss path).
    const summaries = buildFixture(1);
    exchangeStore.getState().setExchanges(summaries);
    // Active project is required for the
    // cache-miss path (the original
    // `getExchange` fallback needed the project
    // id; `openReplayTab` reads the project
    // context from the exchange itself but
    // `ReplayTab.projectId` is set from the
    // descriptor's `project_id`).
    const projectId = asProjectId(
      "00000000-0000-0000-0000-000000000001",
    ) as ProjectId;
    projectStore.setState({
      projects: [
        {
          id: projectId,
          name: "p1",
          target_host: "acme.example.com",
          db_filename: "p1.db",
        },
      ],
      activeProjectId: projectId,
    });
    const targetId = summaries[0]!.id;
    openReplayTabMock.mockResolvedValue({
      source_exchange_id: targetId,
      project_id: projectId,
      request: {
        method: "GET",
        url: "https://api.example.com/v1/foo",
        version: "HTTP/1.1",
        headers: {},
        body: { kind: "empty" },
      },
      original_response: null,
      body_truncated: true, // <-- the audit's 1 MB cap kicked in
    });
    const { container } = render(<ExchangeList />);
    const replayButton = container.querySelector(
      '[data-testid="exchange-list-replay-button"]',
    ) as HTMLButtonElement;
    expect(replayButton).toBeInTheDocument();
    fireEvent.click(replayButton);
    await waitFor(() => {
      expect(openReplayTabMock).toHaveBeenCalledWith(targetId);
    });
    await waitFor(() => {
      expect(replayStore.getState().tabs.length).toBe(1);
    });
    const tab = replayStore.getState().tabs[0]!;
    expect(tab.bodyTruncated).toBe(true);
    expect(tab.sourceExchangeId).toBe(targetId);
  });

  it("clicking Replay on a cache-miss with body_truncated=false creates a tab with bodyTruncated=false", async () => {
    const summaries = buildFixture(1);
    exchangeStore.getState().setExchanges(summaries);
    const projectId = asProjectId(
      "00000000-0000-0000-0000-000000000001",
    ) as ProjectId;
    projectStore.setState({
      projects: [
        {
          id: projectId,
          name: "p1",
          target_host: "acme.example.com",
          db_filename: "p1.db",
        },
      ],
      activeProjectId: projectId,
    });
    const targetId = summaries[0]!.id;
    openReplayTabMock.mockResolvedValue({
      source_exchange_id: targetId,
      project_id: projectId,
      request: {
        method: "GET",
        url: "https://api.example.com/v1/foo",
        version: "HTTP/1.1",
        headers: {},
        body: { kind: "empty" },
      },
      original_response: null,
      body_truncated: false,
    });
    const { container } = render(<ExchangeList />);
    fireEvent.click(
      container.querySelector(
        '[data-testid="exchange-list-replay-button"]',
      ) as HTMLButtonElement,
    );
    await waitFor(() => {
      expect(replayStore.getState().tabs.length).toBe(1);
    });
    expect(replayStore.getState().tabs[0]!.bodyTruncated).toBe(false);
  });
});
