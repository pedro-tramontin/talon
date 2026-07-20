// Tests for the `searchExchanges` Tauri IPC wrapper
// (ui/src/api.ts) and the FTS5 debounce in `useUiStore`
// (ui/src/state/ui.ts).
//
// §4.8 spec: +2 vitest (one for the api wrapper, one for
// the debounced store). The api wrapper test mocks the
// `invoke` from `@tauri-apps/api/core` and asserts the call
// shape (`search_exchanges` command, `projectId` / `query` /
// `limit` arg keys, `ExchangeId[]` return). The debounce
// test asserts that `setFilterFtsQuery` writes to the store
// and that the debounce effect in the consumer is the
// 200ms-debounced IPC driver (not the setter itself).

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useEffect } from "react";
import { act, cleanup, render, screen } from "@testing-library/react";
import { searchExchanges } from "../api";
import { uiStore, useUiStore, FTS_DEBOUNCE_MS } from "../state/ui";
import { asExchangeId, asProjectId } from "../types/ids";
import type { ExchangeId, ProjectId } from "../types/ids";

// Capture the Tauri `invoke` so we can assert on the call
// shape (command name + arg keys + return). The mock is
// installed per-test in `beforeEach`; `vi.restoreAllMocks`
// in `afterEach` rolls it back.
const { invokeMock } = vi.hoisted(() => {
  return { invokeMock: vi.fn() };
});

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

function resetUiStore() {
  uiStore.setState({
    activeRightTab: "inspector",
    filterFtsQuery: "",
    filterFtsResults: [],
  });
}

beforeEach(() => {
  invokeMock.mockReset();
  resetUiStore();
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("searchExchanges (api.ts wrapper)", () => {
  it("calls the Tauri `search_exchanges` command with the right shape", async () => {
    // The Rust command returns `Vec<ExchangeId>` (a list
    // of UUID strings). The mock returns two stub ids;
    // the wrapper must pass them through unchanged.
    const expected: string[] = [
      "00000000-0000-0000-0000-000000000001",
      "00000000-0000-0000-0000-000000000002",
    ];
    invokeMock.mockResolvedValueOnce(expected);

    const projectId = asProjectId("00000000-0000-0000-0000-0000000000aa");
    const result = await searchExchanges(projectId, "POST", 50);

    // The wrapper must call `invoke` exactly once, with
    // the right command name and the right arg keys
    // (camelCase, per Tauri's serde convention).
    expect(invokeMock).toHaveBeenCalledTimes(1);
    expect(invokeMock).toHaveBeenCalledWith("search_exchanges", {
      projectId,
      query: "POST",
      limit: 50,
    });
    // The return is the array as-is (the wrapper is
    // type-cast, not transformed).
    expect(result).toEqual(expected);
  });

  it("defaults `limit` to 1000 when the caller omits it", async () => {
    invokeMock.mockResolvedValueOnce([]);
    await searchExchanges(asProjectId("p"), "GET");
    expect(invokeMock).toHaveBeenCalledWith("search_exchanges", {
      projectId: "p",
      query: "GET",
      limit: 1000,
    });
  });
});

describe("useUiStore FTS5 wiring (§4.8)", () => {
  it("setFilterFtsQuery writes to the store immediately (no debounce in the setter)", () => {
    expect(uiStore.getState().filterFtsQuery).toBe("");
    act(() => {
      uiStore.getState().setFilterFtsQuery("hello");
    });
    expect(uiStore.getState().filterFtsQuery).toBe("hello");
  });

  it("setFilterFtsResults writes to the store and the selector reflects the change", () => {
    // The default value is `[]` — exercise the set/get roundtrip
    // through the React hook (this is what the ExchangeList does
    // in production).
    function Probe() {
      const results = useUiStore((s) => s.filterFtsResults);
      return <span data-testid="probe-count">{results.length}</span>;
    }
    const { rerender } = render(<Probe />);
    expect(screen.getByTestId("probe-count").textContent).toBe("0");

    const ids: ExchangeId[] = [
      asExchangeId("00000000-0000-0000-0000-000000000001"),
      asExchangeId("00000000-0000-0000-0000-000000000002"),
    ];
    act(() => {
      uiStore.getState().setFilterFtsResults(ids);
    });
    rerender(<Probe />);
    expect(screen.getByTestId("probe-count").textContent).toBe("2");
  });

  it("FTS_DEBOUNCE_MS is 200ms (the spec's chosen debounce window)", () => {
    // Pin the constant. The ExchangeList effect uses
    // FTS_DEBOUNCE_MS as the setTimeout delay; changing
    // this value is a coordinated change across the
    // store + the consumer + the tests.
    expect(FTS_DEBOUNCE_MS).toBe(200);
  });

  it("the FTS consumer effect debounces via setTimeout(FTS_DEBOUNCE_MS) and fires searchExchanges once per settled query", async () => {
    // This is the integration test: a tiny consumer that
    // mirrors the ExchangeList's debounce effect (the
    // production shape: setTimeout in useEffect, NOT in
    // the setter). The test verifies the setTimeout fires
    // after exactly FTS_DEBOUNCE_MS and the IPC is called
    // once.
    vi.useFakeTimers();
    invokeMock.mockResolvedValueOnce(["id-1", "id-2"]);

    function FtsConsumer({ projectId }: { projectId: ProjectId | null }) {
      const query = useUiStore((s) => s.filterFtsQuery);
      const setResults = useUiStore((s) => s.setFilterFtsResults);
      useEffect(() => {
        const q = query.trim();
        if (q.length === 0 || !projectId) {
          return;
        }
        const handle = setTimeout(() => {
          searchExchanges(projectId, q)
            .then((ids) => {
              setResults(ids as ExchangeId[]);
            })
            .catch(() => {
              setResults([]);
            });
        }, FTS_DEBOUNCE_MS);
        return () => {
          clearTimeout(handle);
        };
      }, [query, projectId, setResults]);
      return <span data-testid="consumer-query">{query}</span>;
    }

    const projectId = asProjectId("00000000-0000-0000-0000-0000000000aa");
    render(<FtsConsumer projectId={projectId} />);
    expect(screen.getByTestId("consumer-query").textContent).toBe("");

    // Type a query.
    act(() => {
      uiStore.getState().setFilterFtsQuery("hello");
    });
    // Before the debounce elapses, the IPC is NOT called.
    expect(invokeMock).not.toHaveBeenCalled();

    // Advance to just before the debounce — still not called.
    act(() => {
      vi.advanceTimersByTime(FTS_DEBOUNCE_MS - 50);
    });
    expect(invokeMock).not.toHaveBeenCalled();

    // Advance past the debounce — now called exactly once.
    act(() => {
      vi.advanceTimersByTime(100);
    });
    expect(invokeMock).toHaveBeenCalledTimes(1);
    expect(invokeMock).toHaveBeenCalledWith("search_exchanges", {
      projectId,
      query: "hello",
      limit: 1000,
    });

    // No second call when more time elapses.
    act(() => {
      vi.advanceTimersByTime(1000);
    });
    expect(invokeMock).toHaveBeenCalledTimes(1);

    // Empty query → no IPC (the consumer short-circuits).
    act(() => {
      uiStore.getState().setFilterFtsQuery("");
    });
    act(() => {
      vi.advanceTimersByTime(FTS_DEBOUNCE_MS + 100);
    });
    expect(invokeMock).toHaveBeenCalledTimes(1); // unchanged

    vi.useRealTimers();
  });
});
