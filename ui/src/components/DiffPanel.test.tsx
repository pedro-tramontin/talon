// Tests for the DiffPanel (§4.7 v0.5+).
//
// Spec (§4.7 v0.5+):
//   - The diff is LCS-based (the `diff` package's
//     `diffLines` Myers algorithm). The naïve line-by-line
//     index comparison from v0.1 was replaced because it
//     couldn't represent insertions / pure deletions
//     correctly.
//   - Three line kinds: `added` (only in B), `removed`
//     (only in A), `context` (in both).
//   - Line numbers in the gutter (a, b) — useful for
//     cross-referencing in PR-style reviews.
//   - Large diffs are truncated with a "Show full diff"
//     button. The cap is `DIFF_MAX_LINES` rows.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { fireEvent } from "@testing-library/react";
import { exchangeStore } from "../state/exchange";
import { projectStore } from "../state/project";
import type { ExchangeDetail } from "../types/domain";
import type { ExchangeId, ProjectId } from "../types/ids";

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
import { DiffPanel } from "./DiffPanel";

const CURRENT_ID = "00000000-0000-0000-0000-000000000001" as ExchangeId;
const PREVIOUS_ID = "00000000-0000-0000-0000-000000000002" as ExchangeId;
const PROJECT_ID = "00000000-0000-0000-0000-0000000000aa" as ProjectId;
const SUMMARY = "GET /api/example";

function makeDetail(
  id: ExchangeId,
  bodyText: string,
): ExchangeDetail {
  return {
    meta: {
      id,
      project_id: PROJECT_ID,
      timestamp: "2026-07-20T12:00:00.000Z",
      duration_ns: 1_500_000,
      summary: SUMMARY,
      scope_state: "in_scope",
      notes: "",
      starred: false,
    },
    request: {
      method: "GET",
      url: "https://example.test/api/example",
      version: "HTTP/1.1",
      headers: { host: "example.test" },
      body: { kind: "empty" },
    },
    response: {
      version: "HTTP/1.1",
      status: 200,
      status_text: "OK",
      headers: { "content-type": "text/plain" },
      // Use the legacy number[] form so the test exercises
      // the v0.5 backward-compat path in `decodeBodyToBytes`.
      body: {
        kind: "complete",
        data: Array.from(new TextEncoder().encode(bodyText)),
      },
    },
    blocked_reason: null,
  };
}

const mockGetExchange = vi.mocked(getExchange);

beforeEach(() => {
  projectStore.setState({
    projects: [
      {
        id: PROJECT_ID,
        name: "acme",
        target_host: "example.test",
        db_filename: "acme.db",
      },
    ],
    activeProjectId: PROJECT_ID,
  });
  // The store's `previous` lookup walks the `exchanges`
  // array to find a row with the same `summary`. Insert
  // the previous row FIRST (the store is reverse-chronological
  // and `find` returns the first match, which is the
  // "next-newer previous").
  const prevSummary = {
    id: PREVIOUS_ID,
    project_id: PROJECT_ID,
    timestamp: "2026-07-20T11:00:00.000Z",
    duration_ns: 1_000_000,
    summary: SUMMARY,
    scope_state: "in_scope" as const,
    starred: false,
    notes: "",
  };
  const curSummary = {
    ...prevSummary,
    id: CURRENT_ID,
    timestamp: "2026-07-20T12:00:00.000Z",
  };
  exchangeStore.setState({
    selectedId: CURRENT_ID,
    // Reverse-chronological: previous first, current second.
    // The DiffPanel's `find` for a different `id` and
    // same `summary` returns the first match (next-newer),
    // which is the previous row.
    exchanges: [curSummary, prevSummary],
    details: new Map(),
    detailsLru: [],
  });
});

afterEach(() => {
  cleanup();
  exchangeStore.setState({
    selectedId: null,
    exchanges: [],
    details: new Map(),
    detailsLru: [],
  });
  projectStore.setState({ projects: [], activeProjectId: null });
  vi.clearAllMocks();
});

describe("DiffPanel", () => {
  it("renders the no-selection state when no row is selected", () => {
    exchangeStore.setState({ selectedId: null, exchanges: [] });
    render(<DiffPanel />);
    expect(
      screen.getByTestId("diff-panel-no-selection"),
    ).toBeInTheDocument();
  });

  it("renders the no-previous state when no matching summary exists", () => {
    // The "no previous" branch returns BEFORE the
    // `getExchange` effect fires (the `previous` memo is
    // null), so the effect never runs. But because the
    // `current` row is selected, the effect's `if` doesn't
    // early-return — it proceeds to call `getExchange`.
    // Set a default mock so the call doesn't throw.
    mockGetExchange.mockResolvedValue(makeDetail(CURRENT_ID, ""));
    exchangeStore.setState({
      selectedId: CURRENT_ID,
      exchanges: [
        {
          id: CURRENT_ID,
          project_id: PROJECT_ID,
          timestamp: "2026-07-20T12:00:00.000Z",
          duration_ns: 1_000_000,
          summary: "GET /api/only-once",
          scope_state: "in_scope",
          starred: false,
          notes: "",
        },
      ],
    });
    render(<DiffPanel />);
    expect(
      screen.getByTestId("diff-panel-no-previous"),
    ).toBeInTheDocument();
  });

  it("renders the loading state while details are in-flight", () => {
    // Never-resolving mock: keeps the component in the
    // "Loading…" branch.
    mockGetExchange.mockReturnValue(new Promise(() => {}));
    render(<DiffPanel />);
    expect(screen.getByTestId("diff-panel-loading")).toBeInTheDocument();
  });

  it("shows a pure-addition diff: all lines are 'added'", async () => {
    // A is empty; B has 3 lines. Expected: 3 added lines.
    mockGetExchange.mockImplementation(async (_p, id) => {
      if (id === PREVIOUS_ID) return makeDetail(PREVIOUS_ID, "");
      return makeDetail(CURRENT_ID, "alpha\nbeta\ngamma");
    });
    render(<DiffPanel />);
    await waitFor(() => {
      expect(screen.getByTestId("diff-panel")).toBeInTheDocument();
    });
    const added = screen.getAllByTestId("diff-panel-line-added");
    expect(added).toHaveLength(3);
    expect(added[0].textContent).toContain("alpha");
    expect(added[1].textContent).toContain("beta");
    expect(added[2].textContent).toContain("gamma");
    // The summary line includes the +/- counts.
    const summary = screen.getByTestId("diff-panel-summary");
    expect(summary.textContent).toContain("+3");
    expect(summary.textContent).toContain("-0");
  });

  it("shows a pure-deletion diff: all lines are 'removed'", async () => {
    // A has 3 lines; B is empty.
    mockGetExchange.mockImplementation(async (_p, id) => {
      if (id === PREVIOUS_ID) return makeDetail(PREVIOUS_ID, "alpha\nbeta\ngamma");
      return makeDetail(CURRENT_ID, "");
    });
    render(<DiffPanel />);
    await waitFor(() => {
      expect(screen.getByTestId("diff-panel")).toBeInTheDocument();
    });
    const removed = screen.getAllByTestId("diff-panel-line-removed");
    expect(removed).toHaveLength(3);
    expect(removed[0].textContent).toContain("alpha");
    expect(removed[1].textContent).toContain("beta");
    expect(removed[2].textContent).toContain("gamma");
    const summary = screen.getByTestId("diff-panel-summary");
    expect(summary.textContent).toContain("+0");
    expect(summary.textContent).toContain("-3");
  });

  it("shows an LCS-based diff: a mid-line insertion is one 'added' line, not a full replacement", async () => {
    // Naïve diff would have rendered 4 lines (1 common,
    // 1 add, 1 remove, 1 common). LCS-based diff should
    // render 4 lines (1 common, 1 added, 2 common)
    // because the changed line is a pure insertion.
    mockGetExchange.mockImplementation(async (_p, id) => {
      if (id === PREVIOUS_ID)
        return makeDetail(PREVIOUS_ID, "line1\nline2\nline4");
      return makeDetail(CURRENT_ID, "line1\nline2\nline3-new\nline4");
    });
    render(<DiffPanel />);
    await waitFor(() => {
      expect(screen.getByTestId("diff-panel")).toBeInTheDocument();
    });
    const added = screen.getAllByTestId("diff-panel-line-added");
    // `queryAllByTestId` returns `[]` instead of throwing
    // when no elements match — easier to assert "zero
    // removals" with `toHaveLength(0)`.
    const removed = screen.queryAllByTestId("diff-panel-line-removed");
    const context = screen.getAllByTestId("diff-panel-line-context");
    // LCS-based: line1 context, line2 context, line3-new added, line4 context.
    expect(added).toHaveLength(1);
    expect(added[0].textContent).toContain("line3-new");
    expect(removed).toHaveLength(0);
    expect(context).toHaveLength(3);
    expect(context[0].textContent).toContain("line1");
    expect(context[1].textContent).toContain("line2");
    expect(context[2].textContent).toContain("line4");
  });

  it("renders identical bodies as all-context (no added/removed)", async () => {
    mockGetExchange.mockImplementation(async (_p, id) => {
      return makeDetail(id, "same\nlines\nhere");
    });
    render(<DiffPanel />);
    await waitFor(() => {
      expect(screen.getByTestId("diff-panel")).toBeInTheDocument();
    });
    const added = screen.queryAllByTestId("diff-panel-line-added");
    const removed = screen.queryAllByTestId("diff-panel-line-removed");
    const context = screen.getAllByTestId("diff-panel-line-context");
    expect(added).toHaveLength(0);
    expect(removed).toHaveLength(0);
    expect(context).toHaveLength(3);
  });

  it("truncates very large diffs and renders the rest on demand", { timeout: 30_000 }, async () => {
    // Build a 1500-line diff. The cap is 1000 rows; with
    // 100% additions we'd have 1500 added rows in the
    // rendered output (one per token because of
    // `oneChangePerToken: true`).
    const previousLines = Array.from({ length: 100 }, (_, i) => `prev${i}`);
    const currentLines = Array.from({ length: 1500 }, (_, i) => `cur${i}`);
    mockGetExchange.mockImplementation(async (_p, id) => {
      if (id === PREVIOUS_ID) return makeDetail(PREVIOUS_ID, previousLines.join("\n"));
      return makeDetail(CURRENT_ID, currentLines.join("\n"));
    });
    render(<DiffPanel />);
    await waitFor(() => {
      expect(screen.getByTestId("diff-panel")).toBeInTheDocument();
    });
    // The truncated banner + button are present.
    expect(screen.getByTestId("diff-panel-truncated")).toBeInTheDocument();
    expect(
      screen.getByTestId("diff-panel-show-full"),
    ).toBeInTheDocument();
    // Click "Show full diff" and the truncation banner goes
    // away (the cap is disabled, so all rows render).
    fireEvent.click(screen.getByTestId("diff-panel-show-full"));
    expect(
      screen.queryByTestId("diff-panel-truncated"),
    ).not.toBeInTheDocument();
  });

  it("shows the binary-body placeholder when either side is not valid UTF-8", async () => {
    // Use a 0xFF byte that is invalid UTF-8.
    const binary = new Uint8Array([0xff, 0xfe, 0xfd]);
    const binaryBody = {
      kind: "complete" as const,
      data: Array.from(binary),
    };
    const prevDetail = makeDetail(PREVIOUS_ID, "plain text");
    mockGetExchange.mockImplementation(async (_p, id) => {
      if (id === PREVIOUS_ID) {
        return {
          ...prevDetail,
          response: {
            version: "HTTP/1.1",
            status: 200,
            status_text: "OK",
            headers: { "content-type": "application/json" },
            body: binaryBody,
          },
        };
      }
      return makeDetail(CURRENT_ID, "plain text");
    });
    render(<DiffPanel />);
    await waitFor(() => {
      expect(screen.getByTestId("diff-panel-binary")).toBeInTheDocument();
    });
    expect(
      screen.getByTestId("diff-panel-binary").textContent,
    ).toContain("binary");
  });

  it("renders the missing-response state when either side has no response", async () => {
    mockGetExchange.mockImplementation(async (_p, id) => {
      const d = makeDetail(id, "text");
      // Strip the response on the previous side.
      if (id === PREVIOUS_ID) {
        return { ...d, response: null };
      }
      return d;
    });
    render(<DiffPanel />);
    await waitFor(() => {
      expect(
        screen.getByTestId("diff-panel-missing-response"),
      ).toBeInTheDocument();
    });
  });
});
