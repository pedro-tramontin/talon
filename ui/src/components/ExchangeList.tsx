// Virtualized exchange list. Lives in the Capture route's
// left rail. Renders the full filtered exchange array via
// `@tanstack/react-virtual` so a list of N items only mounts
// ~O(visible window) DOM nodes.
//
// Spec (§4.5):
//   - 48px row height, fixed for v1.
//   - overscan: 10 — mount 10 extra rows above/below the
//     visible window so fast scrolls don't flash empty rows.
//   - Reads from `useExchangeStore` (full list) and
//     `useFilteredExchanges()` (filtered list).
//   - Each row is a button: summary text + timestamp; click
//     sets `selectedId` (the detail view in §4.6 reads it).
//   - Selected row highlighted (`bg-blue-100`).
//   - Filter input above the list: free-text substring on
//     `summary`, debounced 150ms so a 10k-row list isn't
//     filtered on every keystroke.
//
// The component is pure display — it does not call any Tauri
// IPC or execute any request bodies. Live updates come from
// the §4.2 wire_event subscription calling `unshiftExchange`
// / `removeExchange` on the store; the list re-virtualizes
// automatically when the underlying array identity changes.

import { useEffect, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  useExchangeStore,
  useFilteredExchanges,
} from "../state/exchange";
import type { ExchangeId } from "../types/ids";
import { LEFT_RAIL_PX } from "../routes/Capture";

/** Row height in px. Fixed for v1; §4.5+ may add dynamic
 * sizing for multi-line summaries. */
const ROW_HEIGHT_PX = 48;

/** Number of extra rows mounted above and below the visible
 * window. 10 is the spec's chosen default — large enough to
 * cover a fast flick on a 60Hz display without flashing
 * empty rows, small enough to keep the DOM small. */
const OVERSCAN = 10;

/** Debounce window (ms) for the filter text input. 150ms
 * is the spec's chosen value — fast enough to feel
 * responsive on a typical typing cadence, slow enough that
 * a 10k-row list isn't re-filtered on every keystroke. */
const FILTER_DEBOUNCE_MS = 150;

/** Tailwind class for the highlighted (selected) row. */
const SELECTED_ROW_CLASS = "bg-blue-100 text-slate-900";
/** Tailwind classes for the unselected row. */
const UNSELECTED_ROW_CLASS = "text-slate-200 hover:bg-slate-800";

/**
 * Format an ISO-8601 timestamp as `HH:MM:SS.mmm` for the row
 * subtitle. The full timestamp is shown on hover (via the
 * `title` attribute) so users can see the date without
 * expanding the row.
 */
function formatTimestamp(iso: string): string {
  // Parse the YYYY-MM-DDTHH:MM:SS(.fff)Z form. We slice
  // rather than reach for `Date` to avoid TZ-dependent
  // formatting across the test-runner and the browser.
  const tPart = iso.split("T")[1] ?? "";
  return tPart.replace("Z", "");
}

export interface ExchangeListProps {
  /** Total height of the scrollable area in px. Defaults
   * to "fill the left rail" via flex-1; callers that
   * want a fixed height can pass a number. */
  height?: number;
}

/**
 * ExchangeList. The left rail of the Capture route. Shows
 * a filter input, then a virtualized list of `ExchangeSummary`
 * rows. Each row is a `<button>` for keyboard a11y; the
 * selected row is highlighted.
 */
export function ExchangeList(_props: ExchangeListProps = {}) {
  // The filter text the user is typing into the input. We
  // keep a local copy so the input stays responsive, then
  // push the debounced value into the store via
  // `setFilter`. The store's `filter.text` is what the
  // `useFilteredExchanges` selector reads.
  const [filterInput, setFilterInput] = useState("");
  const setFilter = useExchangeStore((s) => s.setFilter);
  const setSelectedId = useExchangeStore((s) => s.setSelectedId);
  const selectedId = useExchangeStore((s) => s.selectedId);

  // The filtered list. Re-derived from the store on every
  // store change; the virtualizer turns it into a window
  // of visible rows.
  const filtered = useFilteredExchanges();

  // Ref to the scrollable container. The virtualizer
  // attaches its scroll listener here.
  const parentRef = useRef<HTMLDivElement | null>(null);

  // Virtualizer: 1 row per item, fixed height, with the
  // spec's overscan. `count` is `filtered.length` (NOT the
  // full store list) so the visible range indexes into the
  // filtered array.
  //
  // `initialRect` is a non-zero starting size so the
  // virtualizer computes a non-empty visible window on the
  // first render — important for jsdom (no layout engine)
  // and for SSR. Once `useEffect` mounts, the
  // ResizeObserver attached by the virtualizer takes over
  // and re-measures against the real DOM.
  const virtualizer = useVirtualizer({
    count: filtered.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => ROW_HEIGHT_PX,
    overscan: OVERSCAN,
    initialRect: { width: 240, height: 480 },
  });

  // Debounce: push the filter input into the store 150ms
  // after the user stops typing. This keeps the keystroke
  // path snappy (local state updates the input on every
  // character) while the expensive re-filter only runs
  // after the user pauses.
  useEffect(() => {
    const handle = setTimeout(() => {
      setFilter({ text: filterInput });
    }, FILTER_DEBOUNCE_MS);
    return () => {
      clearTimeout(handle);
    };
  }, [filterInput, setFilter]);

  return (
    <div
      data-testid="exchange-list"
      className="flex h-full flex-col border-r border-slate-800 bg-bg-rail"
      style={{ width: `${LEFT_RAIL_PX}px` }}
    >
      <div className="border-b border-slate-800 p-2">
        <label
          htmlFor="exchange-list-filter"
          className="mb-1 block text-xs uppercase tracking-wide text-slate-400"
        >
          Filter
        </label>
        <input
          id="exchange-list-filter"
          data-testid="exchange-list-filter"
          type="text"
          value={filterInput}
          onChange={(e) => {
            setFilterInput(e.target.value);
          }}
          placeholder="summary…"
          className="w-full rounded border border-slate-700 bg-bg-base px-2 py-1 text-sm text-slate-100 placeholder:text-slate-500 focus:border-accent focus:outline-none"
        />
        <p
          data-testid="exchange-list-count"
          className="mt-1 text-[10px] uppercase tracking-wide text-slate-500"
        >
          {filtered.length} row{filtered.length === 1 ? "" : "s"}
        </p>
      </div>
      <div
        ref={parentRef}
        data-testid="exchange-list-scroll"
        className="flex-1 overflow-auto"
      >
        {filtered.length === 0 ? (
          <p
            data-testid="exchange-list-empty"
            className="p-3 text-xs text-slate-500"
          >
            No exchanges match the current filter.
          </p>
        ) : (
          <div
            data-testid="exchange-list-virtual"
            style={{
              height: `${virtualizer.getTotalSize()}px`,
              position: "relative",
              width: "100%",
            }}
          >
            {virtualizer.getVirtualItems().map((vi) => {
              const row = filtered[vi.index];
              if (!row) return null;
              const isSelected = row.id === selectedId;
              return (
                <button
                  type="button"
                  key={row.id}
                  data-testid="exchange-list-row"
                  data-row-id={row.id}
                  data-row-index={vi.index}
                  onClick={() => {
                    setSelectedId(row.id as ExchangeId);
                  }}
                  title={row.summary}
                  className={`absolute left-0 top-0 flex w-full flex-col justify-center px-3 text-left text-sm transition-colors ${
                    isSelected ? SELECTED_ROW_CLASS : UNSELECTED_ROW_CLASS
                  }`}
                  style={{
                    height: `${vi.size}px`,
                    transform: `translateY(${vi.start}px)`,
                  }}
                >
                  <span className="truncate font-mono text-xs">
                    {row.summary}
                  </span>
                  <span
                    className={`truncate text-[10px] ${
                      isSelected ? "text-slate-600" : "text-slate-500"
                    }`}
                  >
                    {formatTimestamp(row.timestamp)}
                  </span>
                </button>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}
