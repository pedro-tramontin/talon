// Zustand store for the exchange list + selection + filter.
//
// Per the §4.3-4.4 spec, this is a per-feature store (not a
// global app store). The `exchanges` list is currently a plain
// array (the v0.1 skeleton); §4.5 will replace this with a
// virtualized view (per-row data + scroll position + filter
// pipeline), but the store's external API stays the same so
// the §4.5 PR is a swap-out, not a refactor.
//
// Filter state lives here (not in URL) so the dropdown
// selections survive a tab re-render. The `scrollPosition`
// field is the saved scroll-offset; the §4.5 PR reads it on
// mount to restore position after a detail-view round-trip.
//
// **v0.5 (added 2026-07-21):** a second internal map,
// `details`, caches the FULL `HttpExchange` per id. The
// v0.5 wire payload (an `engine_event` with the
// `ExchangeInserted.exchange` field) carries the full body
// inline so the right-rail detail view can render from
// the cache without a per-click `get_exchange` round-trip.
// The cache is bounded by an LRU (default: 500 entries) so
// a long-running session doesn't OOM the React renderer.

import { createStore, useStore } from "zustand";
import { useShallow } from "zustand/react/shallow";
import type { StoreApi } from "zustand/vanilla";
import type { ExchangeDetail, ExchangeSummary } from "../types/domain";
import type { ExchangeId } from "../types/ids";

/** Filter state for the exchange list. v0.1 ships all four
 * fields; §4.5+ wires the actual matching logic. */
export type ExchangeFilter = {
  /** Free-text search across summary / notes. */
  text: string;
  /** Status filter: "in_scope" | "out_of_scope" | "blocked" |
   * "unscoped" | "any". */
  status: string;
  /** HTTP method filter: "GET" | "POST" | ... | "any". */
  method: string;
  /** Tag name filter: "" for "any". */
  tag: string;
};

const EMPTY_FILTER: ExchangeFilter = {
  text: "",
  status: "any",
  method: "any",
  tag: "",
};

/** Max number of `ExchangeDetail` payloads the in-memory
 * cache holds. At ~50 KB per detail (the upper end of a
 * typical JSON-encoded exchange), 500 entries is ~25 MB —
 * within the v0.1 desktop app's memory budget. The LRU
 * evicts the oldest-touched entry when the cap is hit.
 * v0.5 follow-up: when the list is paged (the v0.5+ design
 * ships pagination), the cache bounds scale with the
 * visible page size, not the total list size. */
const DETAILS_CACHE_CAP = 500;

/** Top-level store shape. */
export type ExchangeStore = {
  /** The full list (or a window of it). §4.5 replaces with
   * a virtualized view. */
  exchanges: ExchangeSummary[];
  /** The currently-selected exchange (drives the detail view). */
  selectedId: ExchangeId | null;
  /** The active filter pipeline. */
  filter: ExchangeFilter;
  /** Last-known scroll position (px). §4.5 restores on mount. */
  scrollPosition: number;
  /** v0.5: in-memory cache of `ExchangeDetail` payloads
   * (the full request/response bodies). Keyed by id. The
   * right-rail reads from this cache on click; the engine
   * populates it on each `ExchangeInserted` event. */
  details: Map<ExchangeId, ExchangeDetail>;
  /** v0.5: the LRU order of `details` accesses. The most
   * recently accessed id is at the end; the least is at the
   * front (the eviction target). */
  detailsLru: ExchangeId[];

  /** Replace the whole list. */
  setExchanges: (exchanges: ExchangeSummary[]) => void;
  /** Prepend a new exchange (called from the wire-bus handler
   * when a new `engine_event` with kind=insert lands). */
  unshiftExchange: (exchange: ExchangeSummary) => void;
  /** Remove an exchange by id. */
  removeExchange: (id: ExchangeId) => void;
  /** Update the `notes` field on a specific exchange. */
  updateExchangeNotes: (id: ExchangeId, notes: string) => void;
  /** Merge a partial filter (replaces the matching keys). */
  setFilter: (filter: Partial<ExchangeFilter>) => void;
  /** Set the selected id. */
  setSelectedId: (id: ExchangeId | null) => void;
  /** Save the scroll position (called on scroll). */
  setScrollPosition: (pos: number) => void;
  /** v0.5: read the cached full detail for an id. Returns
   * `undefined` if the id is not in the cache (the engine
   * hasn't pushed it yet, or the LRU evicted it). The
   * right-rail uses this as the primary read path; the
   * `getExchange` Tauri command is the fallback when the
   * cache misses. */
  getDetail: (id: ExchangeId) => ExchangeDetail | undefined;
  /** v0.5: insert a full detail into the cache. Called
   * from the wire-bus handler when an `ExchangeInserted`
   * event lands (and from the detail-fetch fallback when
   * the cache misses). Touches the LRU. */
  putDetail: (detail: ExchangeDetail) => void;
};

function createExchangeStore() {
  return createStore<ExchangeStore>((set, get) => ({
    exchanges: [],
    selectedId: null,
    filter: { ...EMPTY_FILTER },
    scrollPosition: 0,
    details: new Map(),
    detailsLru: [],

    setExchanges(exchanges) {
      set({ exchanges });
    },

    unshiftExchange(exchange) {
      set((state) => ({
        // Newest-first: prepend. The Rust side already returns
        // rows in reverse-chronological order; the wire-bus
        // path mirrors that.
        exchanges: [exchange, ...state.exchanges],
      }));
    },

    removeExchange(id) {
      set((state) => {
        const exchanges = state.exchanges.filter((e) => e.id !== id);
        // Clear `selectedId` if the removed exchange was
        // selected, so the detail view doesn't render stale
        // data.
        const selectedId = state.selectedId === id ? null : state.selectedId;
        // v0.5: also drop the cached detail so the LRU
        // doesn't retain a dead id. The next `getDetail(id)`
        // would return `undefined` and the right-rail would
        // fetch fresh from the engine.
        const details = new Map(state.details);
        details.delete(id);
        const detailsLru = state.detailsLru.filter((x) => x !== id);
        return { exchanges, selectedId, details, detailsLru };
      });
    },

    updateExchangeNotes(id, notes) {
      set((state) => {
        const exchanges = state.exchanges.map((e) =>
          e.id === id ? { ...e, notes } : e,
        );
        // v0.5: also update the cached detail's notes so
        // the right-rail sees the new value without a
        // round-trip. The body/headers/etc. are unchanged.
        const details = new Map(state.details);
        const cached = details.get(id);
        if (cached) {
          details.set(id, {
            ...cached,
            meta: { ...cached.meta, notes },
          });
        }
        return { exchanges, details };
      });
    },

    setFilter(filter) {
      set((state) => ({
        filter: { ...state.filter, ...filter },
      }));
    },

    setSelectedId(id) {
      set({ selectedId: id });
    },

    setScrollPosition(pos) {
      set({ scrollPosition: pos });
    },

    getDetail(id) {
      const state = get();
      const detail = state.details.get(id);
      if (!detail) return undefined;
      // Touch the LRU: move the id to the end. We do this
      // on every read (so the most-recently-touched id is
      // always at the tail). The set() call updates the
      // `detailsLru` array; the `details` Map is unchanged
      // (the id was already in it).
      const lru = state.detailsLru.filter((x) => x !== id);
      lru.push(id);
      set({ detailsLru: lru });
      return detail;
    },

    putDetail(detail) {
      set((state) => {
        const id = detail.meta.id;
        const details = new Map(state.details);
        details.set(id, detail);
        // Touch the LRU: move the id to the end. If the id
        // is already in the LRU, remove the older occurrence
        // first.
        const lru = state.detailsLru.filter((x) => x !== id);
        lru.push(id);
        // Evict the oldest entry if the cap is exceeded. The
        // cap is checked AFTER the insert so a single insert
        // can grow the cache to cap+1 before the eviction
        // runs; the next insert then evicts one. (The
        // alternative — checking before the insert — would
        // give a hard cap of cap-1, which is not what we
        // want.)
        if (lru.length > DETAILS_CACHE_CAP) {
          const evicted = lru.shift();
          if (evicted !== undefined) {
            details.delete(evicted);
          }
        }
        return { details, detailsLru: lru };
      });
    },
  }));
}

// Singleton store for app-wide use.
export const exchangeStore: StoreApi<ExchangeStore> = createExchangeStore();

/**
 * React hook for the exchange store. Use with a selector to
 * limit re-renders to the slice you care about (e.g.
 * `useExchangeStore((s) => s.exchanges)`).
 */
export function useExchangeStore<T>(selector: (state: ExchangeStore) => T): T {
  return useStore(exchangeStore, selector);
}

/**
 * Predicate: does this row match the given filter? v0.6
 * (P2 #6 filter dropdowns, 2026-07-24) extends this to
 * honor all 4 fields of `ExchangeFilter`:
 *
 * - `text`: case-insensitive substring on `summary` +
 *   `notes` (existing behavior + the notes enhancement).
 * - `method`: exact match on `row.method` (or "any" →
 *   pass). The spec calls this the "Method" dropdown.
 * - `status`: range match — 2xx = 200-299, 3xx = 300-399,
 *   4xx = 400-499, 5xx = 500-599, "any" → pass. The
 *   `row.status === 0` (blocked / pending) case is
 *   excluded from all the range buckets — it can only
 *   match a "any" filter.
 * - `tag`: case-insensitive substring on `row.tags` (any
 *   tag matches → pass). The free-text input matches the
 *   spec's "Tag" filter.
 *
 * The predicate is exposed so tests can exercise the
 * matching logic without standing up a renderer.
 */
export function matchesExchangeFilter(
  row: ExchangeSummary,
  filter: ExchangeFilter,
): boolean {
  // Text filter: case-insensitive substring on the summary
  // + notes. An empty filter text is the "any text" case
  // (no-op). v0.6: also search `notes` (the previous
  // v0.1 behavior only searched `summary`).
  if (filter.text.trim().length > 0) {
    const needle = filter.text.trim().toLowerCase();
    const haySummary = row.summary.toLowerCase();
    const hayNotes = (row.notes ?? "").toLowerCase();
    if (!haySummary.includes(needle) && !hayNotes.includes(needle)) {
      return false;
    }
  }
  // Method filter: exact match on `row.method`, or "any"
  // → pass. The dropdown's "any" sentinel is the empty
  // string "" or "any" (the UI normalizes both).
  if (filter.method && filter.method !== "any") {
    if (row.method !== filter.method) return false;
  }
  // Status filter: range match. "any" or "" → pass.
  // The row's `status` is 0 for blocked / pending; those
  // rows can only match an "any" filter.
  if (filter.status && filter.status !== "any") {
    const loHi = statusRange(filter.status);
    if (loHi === null) return false;
    const [lo, hi] = loHi;
    if (row.status < lo || row.status > hi) return false;
  }
  // Tag filter: case-insensitive substring on any of the
  // row's tag names. An empty filter tag is the "any"
  // case (no-op).
  if (filter.tag.trim().length > 0) {
    const needle = filter.tag.trim().toLowerCase();
    const matches = (row.tags ?? []).some((t) =>
      t.toLowerCase().includes(needle),
    );
    if (!matches) return false;
  }
  return true;
}

/**
 * Map the `filter.status` dropdown value (e.g. "2xx",
 * "3xx", "4xx", "5xx", or "any") to a `[lo, hi]` range.
 * Returns `null` for unknown values (caller should treat
 * as "passes" — see the predicate's check). Exposed for
 * the test suite.
 */
export function statusRange(
  bucket: string,
): [number, number] | null {
  switch (bucket) {
    case "2xx":
      return [200, 299];
    case "3xx":
      return [300, 399];
    case "4xx":
      return [400, 499];
    case "5xx":
      return [500, 599];
    default:
      return null;
  }
}

/**
 * Selector: the exchanges list filtered by the active filter.
 * Returns a new array (slice) so React re-renders downstream
 * consumers. The slice is recomputed only when the inputs
 * change (the underlying array identity or the filter object).
 *
 * Implementation note: we use `useShallow` on the
 * `[exchanges, filter]` tuple so the selector itself only
 * re-fires when one of those two values actually changes by
 * reference. The `.filter(...)` call inside the hook then
 * only runs when the inputs change, not on every store
 * notification.
 *
 * The §4.5 virtualized list uses this so it can render only
 * the rows that survive the current filter without having to
 * know about the filter shape.
 */
export function useFilteredExchanges(): ExchangeSummary[] {
  // `useShallow` compares the tuple element-by-element so
  // the downstream filter only runs when the inputs
  // change by reference. The `.filter(...)` still returns
  // a new array each call, but only when needed.
  const [exchanges, filter] = useStore(
    exchangeStore,
    useShallow((state) => [state.exchanges, state.filter] as const),
  );
  // v0.6 P2 #6: the predicate is the single source of
  // truth for filtering. The previous v0.1 short-circuit
  // (`if filter.text is empty, return all`) was a
  // micro-optimization that quietly disabled the new
  // method / status / tag dropdowns when the text
  // input was empty (the common case). Now the
  // predicate handles every field; the `text`-only
  // case is just the "all of `matchesExchangeFilter`'s
  // text branches are vacuously true" path.
  if (
    filter.text.trim().length === 0 &&
    (filter.method === "" || filter.method === "any") &&
    (filter.status === "" || filter.status === "any") &&
    filter.tag.trim().length === 0
  ) {
    return exchanges;
  }
  return exchanges.filter((e) => matchesExchangeFilter(e, filter));
}
