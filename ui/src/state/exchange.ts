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

import { createStore, useStore } from "zustand";
import type { StoreApi } from "zustand/vanilla";
import type { ExchangeSummary } from "../types/domain";
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
};

function createExchangeStore() {
  return createStore<ExchangeStore>((set) => ({
    exchanges: [],
    selectedId: null,
    filter: { ...EMPTY_FILTER },
    scrollPosition: 0,

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
        return { exchanges, selectedId };
      });
    },

    updateExchangeNotes(id, notes) {
      set((state) => {
        const exchanges = state.exchanges.map((e) =>
          e.id === id ? { ...e, notes } : e,
        );
        return { exchanges };
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
