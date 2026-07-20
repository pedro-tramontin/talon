// Zustand store for cross-cutting UI state. v0.1's first
// slice is the active right-rail tab (§4.7). §4.8 added the
// FTS5 search query + the last-known FTS result set so the
// ExchangeList can swap between the in-memory filter and
// the FTS5 result set without re-deriving on every render.
//
// Per the §4.3-4.4 convention, this is a per-feature store
// (not a global app store). The `useUiStore` hook reads
// from a singleton; selectors are kept narrow to avoid
// re-render storms on every state change.

import { createStore, useStore } from "zustand";
import type { StoreApi } from "zustand/vanilla";
import type { ExchangeId } from "../types/ids";

/**
 * The four tabs in the §4.7 right-rail layout. Order is
 * display order (left-to-right in the tab strip).
 *
 * The string values are also the `data-testid` suffix for
 * the tab buttons (e.g. `capture-right-rail-tab-inspector`),
 * so changing them is a UI + test fixture change in one
 * place.
 */
export type RightTab = "inspector" | "decoder" | "diff" | "notes";

/** All four tabs in display order. */
export const RIGHT_TABS: readonly RightTab[] = [
  "inspector",
  "decoder",
  "diff",
  "notes",
] as const;

/** Default tab shown when the right rail first opens. */
export const DEFAULT_RIGHT_TAB: RightTab = "inspector";

/**
 * Debounce window (ms) for the FTS5 query. The Left
 * Rail's "Full-text search" input sets `filterFtsQuery` on
 * every keystroke; we wait this long after the last
 * keystroke before issuing the IPC call. 200ms is the spec's
 * chosen value — fast enough to feel instant, slow enough
 * to avoid hammering the DB on a fast typist.
 */
export const FTS_DEBOUNCE_MS = 200;

/** Top-level store shape. */
export type UiStore = {
  /** The right-rail tab the user is currently viewing. */
  activeRightTab: RightTab;

  /** Switch the right-rail tab. */
  setActiveRightTab: (tab: RightTab) => void;

  /**
   * The user's FTS5 query string (the live, undebounced
   * text from the Left Rail's search input). Empty string
   * means "no FTS filter active". The debounced effect
   * reads this and fires `searchExchanges` 200ms after
   * the last change.
   */
  filterFtsQuery: string;

  /**
   * Update the FTS query string. The Left Rail input calls
   * this on every keystroke; the debounce lives in the
   * subscriber (the useEffect in the component), NOT in
   * the setter.
   */
  setFilterFtsQuery: (q: string) => void;

  /**
   * The last FTS5 result set, as a list of
   * `ExchangeId`s. Empty array when the query is empty or
   * the IPC call hasn't returned yet. The ExchangeList
   * intersects this with the in-memory list to render the
   * filtered rows.
   */
  filterFtsResults: ExchangeId[];

  /** Replace the FTS result set. Called from the
   * debounced effect after a successful
   * `searchExchanges` call. */
  setFilterFtsResults: (ids: ExchangeId[]) => void;
};

function createUiStore() {
  return createStore<UiStore>((set) => ({
    activeRightTab: DEFAULT_RIGHT_TAB,

    setActiveRightTab(tab) {
      set({ activeRightTab: tab });
    },

    filterFtsQuery: "",

    setFilterFtsQuery(q) {
      set({ filterFtsQuery: q });
    },

    filterFtsResults: [],

    setFilterFtsResults(ids) {
      set({ filterFtsResults: ids });
    },
  }));
}

// Singleton store for app-wide use.
export const uiStore: StoreApi<UiStore> = createUiStore();

/**
 * React hook for the UI store. Use with a selector to limit
 * re-renders to the slice you care about (e.g.
 * `useUiStore((s) => s.activeRightTab)`).
 */
export function useUiStore<T>(selector: (state: UiStore) => T): T {
  return useStore(uiStore, selector);
}
