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
 * Predicate: does this row match the given filter? v0.1 only
 * wires the `text` field (free-text substring on `summary`);
 * the status / method / tag fields stay present on the
 * filter state for v0.2 to fill in. We expose the helper so
 * tests can exercise the matching logic without standing up
 * a renderer.
 */
export function matchesExchangeFilter(
  row: ExchangeSummary,
  filter: ExchangeFilter,
): boolean {
  // Text filter: case-insensitive substring on the summary.
  // An empty filter text is the "any text" case (no-op).
  if (filter.text.trim().length > 0) {
    const needle = filter.text.trim().toLowerCase();
    const hay = row.summary.toLowerCase();
    if (!hay.includes(needle)) return false;
  }
  return true;
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
  if (filter.text.trim().length === 0) {
    return exchanges;
  }
  return exchanges.filter((e) => matchesExchangeFilter(e, filter));
}
