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
//
// ## Phase 7 C-B.1 — replay history persistence (UI side)
//
// The per-tab `history` is now persisted to SQLite via the
// `Engine::append_replay_history` / `Engine::list_replay_history`
// Tauri commands (added in Phase 7 C-A.4, PR #73). The store
// rehydrates `history` from SQLite on `openTab` (the
// same pattern as `listMatchReplaceRules` in the scope/M&R
// store). The `appendSend` action persists each new entry
// after the in-memory update. **This is the minimum-viable
// persistence** — on app restart, tabs are empty (the
// in-memory `tabs: ReplayTab[]` is not persisted); the
// user re-opens a tab from the exchange list, and the
// history is reloaded from SQLite. The v0.5+ "re-open tabs
// on app restart" follow-up is out of scope for Phase 7.

import { createStore, useStore } from "zustand";
import type { StoreApi } from "zustand/vanilla";
import type { ExchangeRequest, ExchangeResponse } from "../types/domain";
import { asExchangeId, type ExchangeId, type ProjectId } from "../types/ids";
import {
  appendReplayHistory,
  listReplayHistory,
  type ReplayHistoryEntry,
} from "../api";

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
  /** v0.5+ post-batch gap-fix P1 #4 (2026-07-24): the
   * `body_truncated` flag from `open_replay_tab` /
   * `openReplayTab`. The 1 MB response body cap
   * (`app/src/commands/replay.rs:78-89`) sets this to
   * `true` when the engine returned a truncated body.
   * The `ReplayRequestEditor` reads this and renders
   * a "Response body truncated to 1 MB" notice in the
   * response viewer. The flag is `false` (the default)
   * for tabs that came from the in-memory LRU cache
   * (which never holds truncated bodies — the cache
   * stores the full `ExchangeDetail` from the engine
   * event, not the 1 MB-cap'd replay descriptor). */
  bodyTruncated: boolean;
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
      /** v0.5+ post-batch gap-fix P1 #4 (2026-07-24):
       * the `body_truncated` flag from
       * `open_replay_tab` / `openReplayTab`. Defaults
       * to `false` if not provided (the cache-hit path
       * doesn't have the flag, since the LRU never
       * holds truncated bodies). */
      bodyTruncated?: boolean;
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
        // v0.5+ post-batch gap-fix P1 #4 (2026-07-24):
        // the `body_truncated` flag from
        // `openReplayTab`. The cache-hit path (the
        // existing v0.1 code) didn't have a way to
        // surface this; the new `open_replay_tab` IPC
        // round-trip populates the flag explicitly. The
        // default is `false` (cache-hit path, full body).
        bodyTruncated: source.bodyTruncated ?? false,
      };
      set((s) => ({ tabs: [...s.tabs, tab], activeTabId: id }));

      // Phase 7 C-B.1: rehydrate the tab's `history` from
      // SQLite. This is async (a Tauri IPC call) and runs
      // in the background — the tab is already open +
      // active before the history populates. If the
      // project_id is null, we skip (the entry shape
      // requires it). If the IPC errors, the tab stays
      // empty (the per-send `appendSend` will still
      // persist future entries once the project is set).
      if (tab.projectId) {
        listReplayHistory(tab.projectId, id)
          .then((entries) => {
            if (get().tabs.find((t) => t.id === id)) {
              // Tab still exists; rehydrate.
              set((s) => ({
                tabs: s.tabs.map((t) =>
                  t.id === id
                    ? {
                        ...t,
                        history: entries.map(replayEntryToHistoryItem),
                      }
                    : t,
                ),
              }));
            }
          })
          .catch((e) => {
            console.error("listReplayHistory failed in openTab:", e);
          });
      }

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
      const tab = get().tabs.find((t) => t.id === id);
      if (!tab) return;
      const sequenceWithiNtab = tab.history.length;
      const newHistoryItem = { request, response, timestamp: new Date(), exchangeId };
      set((s) => ({
        tabs: s.tabs.map((t) =>
          t.id === id
            ? {
                ...t,
                latestResponse: response,
                latestReplayId: exchangeId,
                history: [...t.history, newHistoryItem],
              }
            : t,
        ),
      }));

      // Phase 7 C-B.1: persist the new history entry to
      // SQLite. The Tauri command requires a project_id;
      // if the tab doesn't have one, we skip (the entry
      // exists in-memory only; the user's view is still
      // consistent for the current session). The exchangeId
      // round-trip is best-effort: we use the response's
      // `id` if available, else the request's `id`, else
      // generate a fresh UUID. The Rust side has its own
      // request_exchange_id + response_exchange_id fields
      // — for the v1, both point at the same `exchangeId`
      // (the new replay's exchange id, which is what the
      // `ExchangeStore.putDetail` call writes).
      if (tab.projectId) {
        const entryId = asExchangeId(exchangeId ?? crypto.randomUUID());
        const entry: ReplayHistoryEntry = {
          id: entryId,
          project_id: tab.projectId,
          tab_id: id,
          request_exchange_id: entryId,
          response_exchange_id: response && exchangeId ? entryId : null,
          timestamp: newHistoryItem.timestamp.toISOString(),
          sequence_within_tab: sequenceWithiNtab,
        };
        appendReplayHistory(tab.projectId, entry).catch((e) => {
          console.error("appendReplayHistory failed in appendSend:", e);
        });
      }
    },

    setSending(id, sending) {
      set((s) => ({
        tabs: s.tabs.map((t) => (t.id === id ? { ...t, sending } : t)),
      }));
    },
  }));
}

/**
 * Convert a `ReplayHistoryEntry` (the Rust wire shape) to
 * the in-memory `history[0]` shape used by the `ReplayTab`.
 * The Rust side stores `timestamp` as an ISO string; we
 * rehydrate it as a `Date`. The `request` and `response`
 * fields aren't persisted in the `replay_history` table
 * (only the `request_exchange_id` and `response_exchange_id`
 * references are) — the full request/response is still in
 * the `exchanges` table, rehydrated on tab open via the
 * `exchangeStore.get(id)` lookup. For Phase 7, the v1
 * minimum-viable persistence is: the `history[].exchangeId`
 * is set, and the `ReplayHistoryPanel` can navigate to the
 * exchange detail via the id. The full request/response
 * re-hydration is a v0.5+ follow-up.
 */
function replayEntryToHistoryItem(entry: ReplayHistoryEntry): {
  request: ExchangeRequest;
  response: ExchangeResponse | null;
  timestamp: Date;
  exchangeId: ExchangeId | null;
} {
  return {
    request: {
      method: "GET",
      url: "",
      version: "HTTP/1.1",
      headers: {},
      body: { kind: "complete", data: "" },
    },
    response: null,
    timestamp: new Date(entry.timestamp),
    exchangeId: asExchangeId(
      entry.response_exchange_id ?? entry.request_exchange_id,
    ),
  };
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
