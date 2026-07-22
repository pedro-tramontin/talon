// Zustand store for the Replay feature (§5.3).
//
// Per the §4.3-4.4 spec convention, this is a per-feature
// store (not a global app store). The `tabs` list is the
// open replay tabs the user has spawned from the exchange
// list; `activeTabId` is the one currently shown in the main
// panel.
//
// The Replay UI (Part B) wires the Replay button on each
// `ExchangeRow` to `openTab`, and the `ReplayView` component
// reads `tabs` + `activeTabId` to render. The
// `ReplayRequestEditor` calls `sendReplay` (via the Tauri
// bridge), then `setDraft` + `appendSend` to record the
// result. The `ReplayHistoryPanel` reads `tab.history` to
// render the per-tab send history.
//
// ## Spec drift corrections
//
// The spec at
// `/root/.hermes/plans/2026-07-01_phase-05-replay.md`
// uses `JSON.parse(JSON.stringify(exchange.request))` for
// the deep clone; we use `structuredClone` instead (handles
// `undefined` and `Date` correctly; available in all modern
// browsers since 2022; the v0.5 base UI deps support it).
//
// The spec stores `history[].timestamp` as an ISO string;
// we store it as a `Date` object (the
// `ReplayHistoryPanel` formats it via `toLocaleTimeString`
// at render time, so the round-trip through `Date` is
// lossless).

import { createStore, useStore } from "zustand";
import type { StoreApi } from "zustand/vanilla";
import type { ExchangeRequest, ExchangeResponse } from "../types/domain";
import type { ExchangeId, ProjectId } from "../types/ids";

/**
 * One open replay tab. The user has a `draftRequest` they
 * edit, a `latestResponse` from the most recent send, and
 * a `history` of every prior send (newest at the end).
 *
 * `sourceExchangeId` is the captured exchange this tab was
 * opened from. The `openTab` action deduplicates by this
 * id: re-opening a tab for the same source returns the
 * existing tab id (no duplicate tab).
 */
export interface ReplayTab {
  /** UUID v4 generated client-side. */
  readonly id: string;
  /** Display name (defaults to the source exchange's summary). */
  name: string;
  /** The request the user is currently editing. Deep-cloned
   * from the source exchange so the original is untouched. */
  draftRequest: ExchangeRequest;
  /** The latest response (or `null` if not yet sent). */
  latestResponse: ExchangeResponse | null;
  /** The exchange id of the source capture, for "open in
   * capture" navigation. `null` if the tab was created
   * without a source. */
  sourceExchangeId: ExchangeId | null;
  /** The exchange id of the latest replay (for prepending
   * to the exchange list). `null` until the first send. */
  latestReplayId: ExchangeId | null;
  /** History of every send. Newest at the end. */
  history: Array<{
    request: ExchangeRequest;
    response: ExchangeResponse | null;
    timestamp: Date;
    exchangeId: ExchangeId | null;
  }>;
  /** Loading flag for the in-flight send. */
  sending: boolean;
  /** The `ProjectId` the source exchange belongs to. Used
   * by `sendReplay` to persist the new exchange in the
   * right project. */
  projectId: ProjectId | null;
}

/** Top-level store shape. */
export type ReplayStore = {
  /** The open tabs. */
  tabs: ReplayTab[];
  /** The tab the user is currently viewing. `null` if no
   * tab is open. */
  activeTabId: string | null;

  /**
   * Open a new replay tab for the given captured exchange.
   * If a tab for the same `sourceExchangeId` already
   * exists, sets `activeTabId` to that tab and returns
   * its id (no duplicate). Otherwise creates a new tab
   * with a fresh `id` (UUID v4), deep-clones the request
   * via `structuredClone`, and returns the new tab id.
   *
   * `name` is optional; defaults to `source.summary` if
   * not provided. `projectId` is optional; if not
   * provided, the tab's `projectId` is `null` and the
   * `sendReplay` call will fail until the user sets it.
   */
  openTab: (
    source: {
      exchangeId: ExchangeId;
      summary: string;
      request: ExchangeRequest;
      response: ExchangeResponse | null;
      projectId?: ProjectId | null;
    },
  ) => string;
  /** Close a tab. If the closed tab was active, sets
   * `activeTabId` to the first remaining tab (or `null`
   * if no tabs remain). */
  closeTab: (id: string) => void;
  /** Mark a tab as active. */
  setActive: (id: string) => void;
  /** Replace the tab's `draftRequest` (called by the
   * `ReplayRequestEditor` on every keystroke; for
   * performance, consider debouncing in the editor if
   * the typing is heavy). */
  setDraft: (id: string, request: ExchangeRequest) => void;
  /** Append a send result to the tab's `history` (called
   * by the `ReplayRequestEditor` after `sendReplay`
   * returns). Updates `latestResponse` + `latestReplayId`
   * in the same `set` so the `ReplayResponseViewer`
   * re-renders consistently. */
  appendSend: (
    id: string,
    request: ExchangeRequest,
    response: ExchangeResponse | null,
    exchangeId: ExchangeId | null,
  ) => void;
  /** Toggle the per-tab `sending` flag (called by the
   * `ReplayRequestEditor` while `sendReplay` is in
   * flight). */
  setSending: (id: string, sending: boolean) => void;
};

function createReplayStore() {
  return createStore<ReplayStore>((set, get) => ({
    tabs: [],
    activeTabId: null,

    openTab(source) {
      // Dedupe: if a tab for this source already exists,
      // just activate it.
      const existing = get().tabs.find(
        (t) => t.sourceExchangeId !== null && t.sourceExchangeId === source.exchangeId,
      );
      if (existing) {
        set({ activeTabId: existing.id });
        return existing.id;
      }
      // New tab: deep-clone the request via
      // `structuredClone` (replaces the plan's
      // `JSON.parse(JSON.stringify(...))`; handles
      // `undefined` and `Date` correctly).
      const id = crypto.randomUUID();
      const tab: ReplayTab = {
        id,
        name: source.summary,
        draftRequest: structuredClone(source.request),
        latestResponse: source.response,
        sourceExchangeId: source.exchangeId,
        latestReplayId: null,
        history: [],
        sending: false,
        projectId: source.projectId ?? null,
      };
      set((s) => ({ tabs: [...s.tabs, tab], activeTabId: id }));
      return id;
    },

    closeTab(id) {
      set((s) => {
        const tabs = s.tabs.filter((t) => t.id !== id);
        const activeTabId =
          s.activeTabId === id ? (tabs[0]?.id ?? null) : s.activeTabId;
        return { tabs, activeTabId };
      });
    },

    setActive(id) {
      set({ activeTabId: id });
    },

    setDraft(id, request) {
      set((s) => ({
        tabs: s.tabs.map((t) => (t.id === id ? { ...t, draftRequest: request } : t)),
      }));
    },

    appendSend(id, request, response, exchangeId) {
      set((s) => ({
        tabs: s.tabs.map((t) =>
          t.id === id
            ? {
                ...t,
                latestResponse: response,
                latestReplayId: exchangeId,
                history: [
                  ...t.history,
                  { request, response, timestamp: new Date(), exchangeId },
                ],
              }
            : t,
        ),
      }));
    },

    setSending(id, sending) {
      set((s) => ({
        tabs: s.tabs.map((t) => (t.id === id ? { ...t, sending } : t)),
      }));
    },
  }));
}

// Singleton store for app-wide use.
export const replayStore: StoreApi<ReplayStore> = createReplayStore();

/**
 * React hook for the Replay store. Use with a selector to
 * limit re-renders to the slice you care about (e.g.
 * `useReplayStore((s) => s.tabs)`).
 */
export function useReplayStore<T>(selector: (state: ReplayStore) => T): T {
  return useStore(replayStore, selector);
}
