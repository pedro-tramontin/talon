// Tests for `useExchangeStore` (ui/src/state/exchange.ts).
//
// The store is a global module-level singleton. Each test
// resets it via `exchangeStore.setState` so tests are
// independent.

import { beforeEach, describe, expect, it } from "vitest";
import { renderHook } from "@testing-library/react";
import {
  exchangeStore,
  matchesExchangeFilter,
  useFilteredExchanges,
} from "./exchange";
import { asExchangeId, asProjectId } from "../types/ids";
import type { ExchangeSummary } from "../types/domain";

function makeSummary(name: string): ExchangeSummary {
  return {
    id: asExchangeId(`11111111-1111-1111-1111-${name.padStart(12, "0")}`),
    project_id: asProjectId("00000000-0000-0000-0000-000000000000"),
    timestamp: "2026-07-20T12:00:00Z",
    duration_ns: 1234,
    summary: `GET /api/${name}`,
    scope_state: "in_scope",
    starred: false,
    notes: "",
  };
}

function makePostSummary(name: string): ExchangeSummary {
  return {
    id: asExchangeId(`22222222-2222-2222-2222-${name.padStart(12, "0")}`),
    project_id: asProjectId("00000000-0000-0000-0000-000000000000"),
    timestamp: "2026-07-20T12:00:01Z",
    duration_ns: 5678,
    summary: `POST /api/${name}`,
    scope_state: "in_scope",
    starred: false,
    notes: "",
  };
}

function resetStore() {
  exchangeStore.setState({
    exchanges: [],
    selectedId: null,
    filter: { text: "", status: "any", method: "any", tag: "" },
    scrollPosition: 0,
  });
}

/** Build a 1000-row fixture: 999 GET rows and 1 POST row
 * (at index 7, so the "POST" filter narrows to a single
 * row). The POST row is at a stable position so the
 * filter test can assert the slice length is exactly 1. */
function buildFixture(count: number): ExchangeSummary[] {
  const out: ExchangeSummary[] = [];
  for (let i = 0; i < count; i++) {
    if (i === 7) {
      out.push(makePostSummary(`r${i}`));
    } else {
      out.push(makeSummary(`r${i}`));
    }
  }
  return out;
}

beforeEach(() => {
  resetStore();
  // The 1000-row fixture exercises the §4.5 virtualized
  // list's selector path under load. Tests that need a
  // small list (e.g. the basic CRUD assertions above)
  // call `setExchanges` themselves to override the
  // fixture; the reset above only ensures a clean slate.
  exchangeStore.getState().setExchanges(buildFixture(1000));
});

describe("useExchangeStore", () => {
  it("starts with an empty list, no selection, and a default filter", () => {
    // Wipe the beforeEach fixture for this state-shape test
    // so we don't depend on the populated 1000-row default.
    exchangeStore.setState({
      exchanges: [],
      filter: { text: "", status: "any", method: "any", tag: "" },
    });
    expect(exchangeStore.getState().exchanges).toEqual([]);
    expect(exchangeStore.getState().selectedId).toBeNull();
    expect(exchangeStore.getState().filter).toEqual({
      text: "",
      status: "any",
      method: "any",
      tag: "",
    });
    expect(exchangeStore.getState().scrollPosition).toBe(0);
  });

  it("setExchanges replaces the whole list", () => {
    const list = [makeSummary("a"), makeSummary("b")];
    exchangeStore.getState().setExchanges(list);
    expect(exchangeStore.getState().exchanges).toEqual(list);
  });

  it("unshiftExchange prepends to the list (newest-first)", () => {
    exchangeStore.getState().setExchanges([makeSummary("a")]);
    exchangeStore.getState().unshiftExchange(makeSummary("b"));
    const summaries = exchangeStore
      .getState()
      .exchanges.map((e) => e.summary);
    expect(summaries).toEqual(["GET /api/b", "GET /api/a"]);
  });

  it("removeExchange drops an exchange by id", () => {
    exchangeStore.getState().setExchanges([makeSummary("a"), makeSummary("b")]);
    const aId = exchangeStore
      .getState()
      .exchanges.find((e) => e.summary === "GET /api/a")!.id;
    exchangeStore.getState().removeExchange(aId);
    const summaries = exchangeStore
      .getState()
      .exchanges.map((e) => e.summary);
    expect(summaries).toEqual(["GET /api/b"]);
  });

  it("removeExchange clears the selectedId if the removed row was selected", () => {
    const a = makeSummary("a");
    exchangeStore.getState().setExchanges([a]);
    exchangeStore.getState().setSelectedId(a.id);
    expect(exchangeStore.getState().selectedId).toBe(a.id);
    exchangeStore.getState().removeExchange(a.id);
    expect(exchangeStore.getState().selectedId).toBeNull();
  });

  it("updateExchangeNotes replaces the notes on the matching row", () => {
    const a = makeSummary("a");
    exchangeStore.getState().setExchanges([a]);
    exchangeStore.getState().updateExchangeNotes(a.id, "hello world");
    const updated = exchangeStore.getState().exchanges[0];
    expect(updated.notes).toBe("hello world");
  });

  it("setFilter merges into the existing filter (partial update)", () => {
    exchangeStore.getState().setFilter({ text: "users" });
    expect(exchangeStore.getState().filter.text).toBe("users");
    expect(exchangeStore.getState().filter.status).toBe("any");

    exchangeStore.getState().setFilter({ method: "POST" });
    expect(exchangeStore.getState().filter.text).toBe("users");
    expect(exchangeStore.getState().filter.method).toBe("POST");
  });

  it("setSelectedId sets and clears the selection", () => {
    const a = makeSummary("a");
    exchangeStore.getState().setExchanges([a]);
    exchangeStore.getState().setSelectedId(a.id);
    expect(exchangeStore.getState().selectedId).toBe(a.id);
    exchangeStore.getState().setSelectedId(null);
    expect(exchangeStore.getState().selectedId).toBeNull();
  });

  it("setScrollPosition updates the saved scroll position", () => {
    exchangeStore.getState().setScrollPosition(1234);
    expect(exchangeStore.getState().scrollPosition).toBe(1234);
  });
});

describe("matchesExchangeFilter", () => {
  const row: ExchangeSummary = makeSummary("users");

  it("matches when the filter text is empty (no-op)", () => {
    expect(
      matchesExchangeFilter(row, { text: "", status: "any", method: "any", tag: "" }),
    ).toBe(true);
    expect(
      matchesExchangeFilter(row, { text: "   ", status: "any", method: "any", tag: "" }),
    ).toBe(true);
  });

  it("matches a substring in the summary (case-insensitive)", () => {
    expect(
      matchesExchangeFilter(row, { text: "users", status: "any", method: "any", tag: "" }),
    ).toBe(true);
    expect(
      matchesExchangeFilter(row, { text: "USERS", status: "any", method: "any", tag: "" }),
    ).toBe(true);
    expect(
      matchesExchangeFilter(row, { text: "api", status: "any", method: "any", tag: "" }),
    ).toBe(true);
  });

  it("rejects a row whose summary lacks the substring", () => {
    expect(
      matchesExchangeFilter(row, { text: "POST", status: "any", method: "any", tag: "" }),
    ).toBe(false);
  });
});

describe("useFilteredExchanges", () => {
  it("returns the full 1000-row list when the filter is cleared", () => {
    // The beforeEach populated the store with 1000 rows.
    const { result } = renderHook(() => useFilteredExchanges());
    expect(result.current.length).toBe(1000);
  });

  it('narrows the 1000-row list to a single row when filtering "POST"', () => {
    // The fixture has exactly one POST row (at index 7);
    // any other filter text would also need to match
    // "POST" as a substring of a GET summary, which it
    // doesn't, so we expect exactly 1.
    exchangeStore.getState().setFilter({ text: "POST" });
    const { result } = renderHook(() => useFilteredExchanges());
    expect(result.current.length).toBe(1);
    expect(result.current[0]?.summary).toMatch(/^POST/);
  });

  it("returns the full list again after the filter is cleared", () => {
    exchangeStore.getState().setFilter({ text: "POST" });
    {
      const { result } = renderHook(() => useFilteredExchanges());
      expect(result.current.length).toBe(1);
    }
    exchangeStore.getState().setFilter({ text: "" });
    {
      const { result } = renderHook(() => useFilteredExchanges());
      expect(result.current.length).toBe(1000);
    }
  });
});
