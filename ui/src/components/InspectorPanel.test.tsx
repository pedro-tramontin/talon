// Tests for the InspectorPanel (§4.7).
//
// Spec (§4.7):
//   - Inspector shows the selected exchange's request
//     details: query params, request headers, cookies,
//     JSON body (when parseable), and (when present)
//     response headers.
//   - The panel reads `selectedId` from the
//     `useExchangeStore` and fetches the full
//     `ExchangeDetail` via the `get_exchange` Tauri
//     command (the store only carries the thin
//     `ExchangeSummary` row).
//   - Empty state when no row is selected.
//
// We mock `get_exchange` via `vi.mock` so the test
// doesn't need a real Tauri runtime. The mock returns
// a fixed `ExchangeDetail`; the panel's `useEffect`
// fires once on mount and renders.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { exchangeStore } from "../state/exchange";
import { projectStore } from "../state/project";
import type { ExchangeDetail } from "../types/domain";
import type { ExchangeId, ProjectId } from "../types/ids";

// Mock the `get_exchange` IPC. The `updateNotes` mock
// is also pulled in (the api module exports both), but
// only `get_exchange` is exercised by the Inspector
// tests. The `vi.mock` factory must return an object
// that mirrors the module's surface so TypeScript
// doesn't complain.
vi.mock("../api", () => ({
  getExchange: vi.fn(),
  updateNotes: vi.fn(),
  greet: vi.fn(),
  agentStart: vi.fn(),
  agentConfirmWrite: vi.fn(),
  agentCancel: vi.fn(),
  onAgentEvent: vi.fn(),
  onConfirmRequest: vi.fn(),
  onConfirmResponse: vi.fn(),
  openProject: vi.fn(),
  closeProject: vi.fn(),
  listExchanges: vi.fn(),
  proxyStatus: vi.fn(),
  startProxy: vi.fn(),
  stopProxy: vi.fn(),
}));

import { getExchange } from "../api";
import { InspectorPanel } from "./InspectorPanel";

const sampleDetail: ExchangeDetail = {
  meta: {
    id: "00000000-0000-0000-0000-000000000001" as ExchangeId,
    project_id: "00000000-0000-0000-0000-0000000000aa" as ProjectId,
    timestamp: "2026-07-20T12:00:00.000Z",
    duration_ns: 1_500_000,
    summary: "GET /api/example?token=abc",
    scope_state: "in_scope",
    notes: "",
    starred: false,
  },
  request: {
    method: "GET",
    url: "https://example.test/api/example?token=abc&v=2",
    version: "HTTP/1.1",
    headers: {
      host: "example.test",
      accept: "application/json",
      cookie: "session=xyz; theme=dark",
    },
    body: {
      kind: "complete",
      data: Array.from(
        new TextEncoder().encode('{"hello":"world","n":1}'),
      ),
    },
  },
  response: {
    version: "HTTP/1.1",
    status: 200,
    status_text: "OK",
    headers: { "content-type": "application/json" },
    body: { kind: "complete", data: Array.from(new TextEncoder().encode("{}")) },
  },
  blocked_reason: null,
};

const mockGetExchange = vi.mocked(getExchange);

beforeEach(() => {
  // Set up the store: one project active, one exchange
  // selected (the sample row above).
  const projectId = sampleDetail.meta.project_id;
  const summary = {
    id: sampleDetail.meta.id,
    project_id: projectId,
    timestamp: sampleDetail.meta.timestamp,
    duration_ns: sampleDetail.meta.duration_ns,
    summary: sampleDetail.meta.summary,
    scope_state: sampleDetail.meta.scope_state,
    starred: false,
    notes: "",
  };
  projectStore.setState({
    projects: [
      {
        id: projectId,
        name: "acme",
        target_host: "example.test",
        db_filename: "acme.db",
      },
    ],
    activeProjectId: projectId,
  });
  exchangeStore.setState({
    selectedId: sampleDetail.meta.id,
    exchanges: [summary],
    // v0.5: start each test with an empty detail cache so
    // the cache-first path in `InspectorPanel` falls through
    // to the `mockResolvedValue(Once)` setup below. (The
    // afterEach ALSO resets the cache, so a test that
    // successfully cached a payload via `putDetail` doesn't
    // leak into the next test's first render.)
    details: new Map(),
    detailsLru: [],
  });
  mockGetExchange.mockResolvedValue(sampleDetail);
});

afterEach(() => {
  cleanup();
  exchangeStore.setState({
    selectedId: null,
    exchanges: [],
    // v0.5: reset the detail cache between tests so a
    // cached payload from a previous test doesn't satisfy
    // the v0.5 cache-first path and skip the
    // `mockResolvedValueOnce` setup in the current test.
    details: new Map(),
    detailsLru: [],
  });
  projectStore.setState({ projects: [], activeProjectId: null });
  vi.clearAllMocks();
});

describe("InspectorPanel", () => {
  it("renders the no-selection state when no row is selected", () => {
    exchangeStore.setState({ selectedId: null, exchanges: [] });
    render(<InspectorPanel />);
    expect(
      screen.getByTestId("inspector-panel-no-selection"),
    ).toBeInTheDocument();
  });

  it("renders query params, request headers, cookies, and JSON body", async () => {
    render(<InspectorPanel />);
    // Wait for the detail fetch to land.
    await waitFor(() => {
      expect(screen.getByTestId("inspector-panel")).toBeInTheDocument();
    });
    // Query params: `token=abc` and `v=2`.
    const query = screen.getByTestId("inspector-panel-query-params");
    expect(query.textContent).toContain("token");
    expect(query.textContent).toContain("abc");
    expect(query.textContent).toContain("v");
    expect(query.textContent).toContain("2");
    // Request headers: host, accept, cookie.
    const headers = screen.getByTestId("inspector-panel-request-headers");
    expect(headers.textContent).toContain("host");
    expect(headers.textContent).toContain("example.test");
    expect(headers.textContent).toContain("accept");
    expect(headers.textContent).toContain("application/json");
    // Cookies: session=xyz and theme=dark.
    const cookies = screen.getByTestId("inspector-panel-cookies");
    expect(cookies.textContent).toContain("session");
    expect(cookies.textContent).toContain("xyz");
    expect(cookies.textContent).toContain("theme");
    expect(cookies.textContent).toContain("dark");
    // Body: the JSON parser produces a pretty-printed
    // pre block with the original keys.
    const body = screen.getByTestId("inspector-panel-body-json");
    expect(body.textContent).toContain("hello");
    expect(body.textContent).toContain("world");
    expect(body.textContent).toContain("n");
  });

  it("falls back to a plain-text body when the body is not valid JSON", async () => {
    mockGetExchange.mockResolvedValueOnce({
      ...sampleDetail,
      request: {
        ...sampleDetail.request,
        body: {
          kind: "complete",
          data: Array.from(new TextEncoder().encode("not json")),
        },
      },
    });
    render(<InspectorPanel />);
    await waitFor(() => {
      expect(
        screen.getByTestId("inspector-panel-body-text"),
      ).toBeInTheDocument();
    });
    expect(
      screen.queryByTestId("inspector-panel-body-json"),
    ).not.toBeInTheDocument();
  });

  // Note: the binary-body case (HexViewer) lives in
  // RequestInspector.test.tsx, not here. InspectorPanel
  // shows the request body as JSON or plain text only;
  // binary bodies render as the plain-text fallback (the
  // TextDecoder fails). The HexViewer is the
  // RequestInspector's "binary" branch testid.
});
