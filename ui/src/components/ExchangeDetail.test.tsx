// Tests for the ExchangeDetail panel (the §4.6 capture-UI
// detail inspector).
//
// Spec (§4.6): three sub-tabs (pretty / headers / raw) on
// the request + response inspectors, empty state when no
// row is selected, and a sub-tab toggle that works for both
// request and response inspectors.
//
// We use the `testDetail` and `testSelectedId` props the
// component exposes to skip the Tauri IPC round-trip — the
// IPC is exercised end-to-end by §4.9's smoke test.

import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { exchangeStore } from "../state/exchange";
import type { ExchangeDetail as ExchangeDetailDto } from "../types/domain";
import type { ExchangeId, ProjectId } from "../types/ids";
import { ExchangeDetail } from "./ExchangeDetail";

// Sample fixture. The shape mirrors `bk_core::HttpExchange`.
// The request body is "hello world" (UTF-8) so the
// `pretty` tab renders the decoded text.
const sampleDetail: ExchangeDetailDto = {
  meta: {
    id: "00000000-0000-0000-0000-000000000001" as ExchangeId,
    project_id: "00000000-0000-0000-0000-0000000000aa" as ProjectId,
    timestamp: "2026-07-20T12:00:00.000Z",
    duration_ns: 1_500_000,
    summary: "GET /api/example",
    scope_state: "in_scope",
    notes: "",
    starred: false,
  },
  request: {
    method: "GET",
    url: "https://example.test/api/example",
    version: "HTTP/1.1",
    headers: {
      host: "example.test",
      accept: "application/json",
    },
    body: { kind: "complete", data: Array.from(new TextEncoder().encode("hello world")) },
  },
  response: {
    version: "HTTP/1.1",
    status: 200,
    status_text: "OK",
    headers: { "content-type": "application/json" },
    body: { kind: "complete", data: Array.from(new TextEncoder().encode('{"ok":true}')) },
  },
  blocked_reason: null,
};

afterEach(() => {
  cleanup();
  // Reset the exchange store between tests so `selectedId`
  // from one test doesn't leak into the next.
  exchangeStore.setState({
    selectedId: null,
    exchanges: [],
  });
});

describe("ExchangeDetail", () => {
  it("renders the empty state when no row is selected", () => {
    render(<ExchangeDetail />);
    expect(screen.getByTestId("exchange-detail-empty")).toBeInTheDocument();
    expect(screen.queryByTestId("exchange-detail")).not.toBeInTheDocument();
  });

  it("renders the detail panel when a row is selected (via testDetail)", () => {
    render(
      <ExchangeDetail
        testDetail={sampleDetail}
        testSelectedId={sampleDetail.meta.id}
      />,
    );
    // The empty state should NOT be present.
    expect(screen.queryByTestId("exchange-detail-empty")).not.toBeInTheDocument();
    // The detail panel itself should be.
    const panel = screen.getByTestId("exchange-detail");
    expect(panel).toBeInTheDocument();
    // The header bar shows method + URL + duration.
    const header = screen.getByTestId("exchange-detail-header");
    expect(header.textContent).toContain("GET");
    expect(header.textContent).toContain("https://example.test/api/example");
    // The request inspector is in pretty mode by default.
    expect(screen.getByTestId("request-inspector-pretty")).toBeInTheDocument();
    // The response inspector is in pretty mode by default.
    expect(screen.getByTestId("response-inspector-pretty")).toBeInTheDocument();
  });

  it("toggles the sub-tabs on the request inspector", () => {
    render(
      <ExchangeDetail
        testDetail={sampleDetail}
        testSelectedId={sampleDetail.meta.id}
      />,
    );
    // Start on pretty.
    expect(screen.getByTestId("request-inspector-pretty")).toBeInTheDocument();
    expect(screen.queryByTestId("request-inspector-headers")).not.toBeInTheDocument();
    expect(screen.queryByTestId("request-inspector-raw")).not.toBeInTheDocument();

    // Switch to headers.
    fireEvent.click(screen.getByTestId("request-inspector-tab-headers"));
    expect(screen.queryByTestId("request-inspector-pretty")).not.toBeInTheDocument();
    expect(screen.getByTestId("request-inspector-headers")).toBeInTheDocument();
    // The headers pane shows the host + accept entries.
    const headers = screen.getByTestId("request-inspector-headers");
    expect(headers.textContent).toContain("host");
    expect(headers.textContent).toContain("accept");

    // Switch to raw.
    fireEvent.click(screen.getByTestId("request-inspector-tab-raw"));
    expect(screen.queryByTestId("request-inspector-headers")).not.toBeInTheDocument();
    const raw = screen.getByTestId("request-inspector-raw");
    expect(raw).toBeInTheDocument();
    // The raw pane shows the start line.
    expect(raw.textContent).toContain("GET https://example.test/api/example HTTP/1.1");
    expect(raw.textContent).toContain("host: example.test");

    // Back to pretty.
    fireEvent.click(screen.getByTestId("request-inspector-tab-pretty"));
    expect(screen.queryByTestId("request-inspector-raw")).not.toBeInTheDocument();
    expect(screen.getByTestId("request-inspector-pretty")).toBeInTheDocument();
  });
});
