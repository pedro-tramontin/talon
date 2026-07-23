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
import type { MatchReplaceRule, ScopeRule } from "../types/domain";
import {
  addMatchReplaceRule,
  removeMatchReplaceRule,
} from "../api";
import type { ProjectId } from "../types/ids";

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

  /**
   * The main-panel mode. `"capture"` shows the §4.3cid;
  `"replay"` shows the Phase 5 `ReplayView` (tab bar + request
  editor + response viewer + history panel). The
  `ExchangeList` row's Replay button (added in Phase 5)
  flips this to `"replay"` after `useReplayStore.openTab`.
  */
  mode: "capture" | "replay";

  /** Switch the main-panel mode. */
  setMode: (m: "capture" | "replay") => void;

  // -------------------------------------------------------------------------
  // Phase 6 (§6.6 + §6.7) — scope rules + match & replace rules
  // -------------------------------------------------------------------------

  /**
   * The active project's scope rules (Phase 6 §6.6). The
   * `ScopeRuleEditor` component (bottom of the left rail)
   * reads + writes this; the Tauri command round-trips
   * through `Engine::get_project` / `update_project`.
   *
   * Empty array when no project is open or the project has
   * no rules. The list is the source of truth in the UI;
   * the backend has the same data in `Project::settings::scope_rules`.
   */
  scopeRules: ScopeRule[];

  /** Replace the scope rules list. */
  setScopeRules: (rules: ScopeRule[]) => void;

  /**
   * The active project's match & replace rules (Phase 6
   * §6.7). The `MatchReplaceEditor` component (inside the
   * Settings modal) reads + writes this.
   */
  matchReplaceRules: MatchReplaceRule[];

  /** Replace the M&R rules list. */
  setMatchReplaceRules: (rules: MatchReplaceRule[]) => void;

  /**
   * Update a single M&R rule by index (Phase 7 C-B.3). The
   * `addMatchReplaceRule` Tauri command is push-only; an
   * edit is therefore a remove + add round-trip. The
   * action reads the current rules from the store,
   * removes the rule at `idx` on the backend, adds the
   * patched rule, and optimistically replaces the local
   * `matchReplaceRules[idx]` with the new rule.
   *
   * The action is a thin wrapper around the existing
   * IPC commands; there is no new Tauri command for the
   * edit case. If either IPC call fails, the local
   * store is left unchanged (the error is logged to
   * `console.error`; the UI degrades silently — the
   * user can re-try by editing again).
   */
  updateMatchReplaceRule: (
    projectId: ProjectId,
    idx: number,
    patch: Partial<MatchReplaceRule>,
  ) => Promise<void>;

  /**
   * Whether the Settings modal is open. Flipped by the
   * "Settings" button in the top bar; closes on overlay
   * click or on the explicit close button.
   */
  settingsOpen: boolean;

  /** Open / close the Settings modal. */
  setSettingsOpen: (open: boolean) => void;

  /**
   * Whether the New Project modal is open. Flipped by the
   * "+" button next to the project dropdown in the Capture
   * top bar. Phase 8 (2026-07-23) — the modal hosts the
   * `openProject` + `addProject` + `setActiveProject`
   * sequence that wires the Tauri command to the UI.
   */
  newProjectModalOpen: boolean;

  /** Open / close the New Project modal. */
  setNewProjectModalOpen: (open: boolean) => void;
};

function createUiStore() {
  return createStore<UiStore>((set, get) => ({
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

    mode: "capture",

    setMode(m) {
      set({ mode: m });
    },

    scopeRules: [],

    setScopeRules(rules) {
      set({ scopeRules: rules });
    },

    matchReplaceRules: [],

    setMatchReplaceRules(rules) {
      set({ matchReplaceRules: rules });
    },

    async updateMatchReplaceRule(projectId, idx, patch) {
      const current = get().matchReplaceRules;
      const existing = current[idx];
      if (!existing) return;
      const next: MatchReplaceRule = { ...existing, ...patch };
      try {
        // Round-trip: remove the old rule, add the new one.
        // The Rust side does not have a dedicated
        // "update" command (the Tauri IPC surface is
        // push-only by design — the rules are an
        // append-mostly log).
        await removeMatchReplaceRule(projectId, idx);
        await addMatchReplaceRule(projectId, next);
        // Optimistic local update: replace the rule at
        // `idx` with the patched one. If either IPC call
        // failed, we throw before this line, leaving the
        // store unchanged (the user can retry).
        set((s) => ({
          matchReplaceRules: s.matchReplaceRules.map((r, i) =>
            i === idx ? next : r,
          ),
        }));
      } catch (e) {
        console.error("updateMatchReplaceRule failed:", e);
        throw e;
      }
    },

    settingsOpen: false,

    setSettingsOpen(open) {
      set({ settingsOpen: open });
    },

    newProjectModalOpen: false,

    setNewProjectModalOpen(open) {
      set({ newProjectModalOpen: open });
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
