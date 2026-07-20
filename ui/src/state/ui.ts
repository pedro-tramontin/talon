// Zustand store for cross-cutting UI state. v0.1's first
// slice is the active right-rail tab (§4.7); the
// `filterFtsQuery` field is reserved for the §4.8 FTS
// followup so the store can grow without a rename.
//
// Per the §4.3-4.4 convention, this is a per-feature store
// (not a global app store). The `useUiStore` hook reads
// from a singleton; selectors are kept narrow to avoid
// re-render storms on every state change.

import { createStore, useStore } from "zustand";
import type { StoreApi } from "zustand/vanilla";

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

/** Top-level store shape. */
export type UiStore = {
  /** The right-rail tab the user is currently viewing. */
  activeRightTab: RightTab;

  /** Switch the right-rail tab. */
  setActiveRightTab: (tab: RightTab) => void;
};

function createUiStore() {
  return createStore<UiStore>((set) => ({
    activeRightTab: DEFAULT_RIGHT_TAB,

    setActiveRightTab(tab) {
      set({ activeRightTab: tab });
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
