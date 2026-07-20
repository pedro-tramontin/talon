// Tests for `useExchangeStore` (ui/src/state/exchange.ts).
//
// The store is a global module-level singleton. Each test
// resets it via `exchangeStore.setState` so tests are
// independent.

import { beforeEach, describe, expect, it } from "vitest";
import { exchangeStore } from "./exchange";
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

function resetStore() {
  exchangeStore.setState({
    exchanges: [],
    selectedId: null,
    filter: { text: "", status: "any", method: "any", tag: "" },
    scrollPosition: 0,
  });
}

beforeEach(() => {
  resetStore();
});

describe("useExchangeStore", () => {
  it("starts with an empty list, no selection, and a default filter", () => {
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
