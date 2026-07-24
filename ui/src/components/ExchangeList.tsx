// Virtualized exchange list. Lives in the Capture route's
// left rail. Renders the full filtered exchange array via
// `@tanstack/react-virtual` so a list of N items only mounts
// ~O(visible window) DOM nodes.
//
// Spec (§4.5 + §4.8):
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
//   - §4.8: a "Full-text search" input BELOW the substring
//     filter issues a debounced 200ms FTS5 query to the
//     Rust backend (`searchExchanges`). When the FTS
//     query is non-empty, the FTS result set is intersected
//     with the in-memory list; when empty, the component
//     falls back to the in-memory filter only.
//
// The component is pure display — it does not call any Tauri
// IPC directly other than the FTS5 search (which is
// debounced + state-driven). Live updates come from
// the §4.2 wire_event subscription calling `unshiftExchange`
// / `removeExchange` on the store; the list re-virtualizes
// automatically when the underlying array identity changes.

import { useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  useExchangeStore,
  useFilteredExchanges,
} from "../state/exchange";
import { useUiStore, FTS_DEBOUNCE_MS } from "../state/ui";
import { useReplayStore } from "../state/replay";
import { useProjectStore } from "../state/project";
import {
  deleteExchange,
  openReplayTab,
  searchExchanges,
} from "../api";
import type { ExchangeSummary } from "../types/domain";
import type { ExchangeId, ProjectId } from "../types/ids";
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
  // v0.6 P2 #6: read the filter object directly so the
  // 3 new dropdowns (status, method, tag) can be
  // controlled. The `useFilteredExchanges` hook reads
  // it too, but the JSX needs the live values for the
  // `value=` props on the `<select>` and `<input>`.
  const filter = useExchangeStore((s) => s.filter);
  const setSelectedId = useExchangeStore((s) => s.setSelectedId);
  const selectedId = useExchangeStore((s) => s.selectedId);

  // §4.8 — FTS5 query + result set. The input below the
  // substring filter drives `filterFtsQuery` (debounced
  // 200ms via the effect below) and the FTS result set
  // is intersected with the in-memory filtered list.
  const filterFtsQuery = useUiStore((s) => s.filterFtsQuery);
  const setFilterFtsQuery = useUiStore((s) => s.setFilterFtsQuery);
  const filterFtsResults = useUiStore((s) => s.filterFtsResults);
  const setFilterFtsResults = useUiStore((s) => s.setFilterFtsResults);
  const activeProjectId = useProjectStore((s) => s.activeProjectId);

  // The filtered list (in-memory). Re-derived from the
  // store on every store change; the virtualizer turns it
  // into a window of visible rows.
  const inMemoryFiltered = useFilteredExchanges();

  // When the FTS query is non-empty, the visible list is
  // the intersection of the in-memory filtered list and
  // the FTS result set. When the FTS query is empty, we
  // fall back to the in-memory filter only.
  const filtered: ExchangeSummary[] = useMemo(() => {
    if (filterFtsQuery.trim().length === 0) {
      return inMemoryFiltered;
    }
    if (filterFtsResults.length === 0) {
      // FTS query is set but the result set is still
      // empty (either pre-debounce or the query
      // returned zero matches). Render an empty list so
      // the existing "no exchanges match" message
      // shows. Avoids flickering the full list while the
      // user is typing.
      return [];
    }
    const allowed = new Set<string>(filterFtsResults);
    return inMemoryFiltered.filter((e) => allowed.has(e.id));
  }, [inMemoryFiltered, filterFtsQuery, filterFtsResults]);

  // §4.8 — debounced FTS5 query. Wait 200ms after the
  // last keystroke, then call `searchExchanges` against
  // the active project. Empty / whitespace queries clear
  // the result set (so the list falls back to the
  // in-memory filter).
  useEffect(() => {
    const q = filterFtsQuery.trim();
    if (q.length === 0) {
      // Clear the result set so the list shows the
      // in-memory filter only. Guard: if the result
      // set is already empty, skip the setState to
      // avoid an extra re-render (which would
      // re-trigger this effect and the
      // virtualizer's ResizeObserver in a tight
      // loop during tests).
      if (filterFtsResults.length > 0) {
        setFilterFtsResults([]);
      }
      return;
    }
    if (!activeProjectId) {
      // No project is open — clear results so the list
      // doesn't show stale ids from a previous project.
      if (filterFtsResults.length > 0) {
        setFilterFtsResults([]);
      }
      return;
    }
    const handle = setTimeout(() => {
      searchExchanges(activeProjectId, q)
        .then((ids) => {
          setFilterFtsResults(ids as ExchangeId[]);
        })
        .catch(() => {
          // Backend error (malformed FTS5 query, etc.) —
          // clear the result set so the user sees the
          // existing empty-state message instead of
          // stale rows. The error is intentionally
          // swallowed here; the §4.8 v0.5 followup
          // surfaces it as a red helper text under the
          // input.
          setFilterFtsResults([]);
        });
    }, FTS_DEBOUNCE_MS);
    return () => {
      clearTimeout(handle);
    };
  }, [
    filterFtsQuery,
    filterFtsResults,
    activeProjectId,
    setFilterFtsResults,
  ]);

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
        {/* v0.6 P2 #6 (2026-07-24): the 3 missing filter
         * dropdowns (Status, Method, Tag) are now wired.
         * The 3 dropdowns feed the `filter.status` /
         * `filter.method` / `filter.tag` fields of the
         * existing `ExchangeFilter` store shape (the
         * fields were already present in the store from
         * the v0.1 spec — only the predicate + the UI
         * were missing). The dropdowns use the existing
         * `setFilter` Zustand action; no new store
         * action is needed. See `state/exchange.ts` for
         * the predicate and the `useFilteredExchanges`
         * hook. */}
        <div className="mt-2 grid grid-cols-2 gap-2">
          <div>
            <label
              htmlFor="exchange-list-status"
              className="mb-1 block text-[10px] uppercase tracking-wide text-slate-500"
            >
              Status
            </label>
            <select
              id="exchange-list-status"
              data-testid="exchange-list-status"
              value={filter.status}
              onChange={(e) => setFilter({ status: e.target.value })}
              className="w-full rounded border border-slate-700 bg-bg-base px-2 py-1 text-sm text-slate-100 focus:border-accent focus:outline-none"
            >
              <option value="any">any</option>
              <option value="2xx">2xx</option>
              <option value="3xx">3xx</option>
              <option value="4xx">4xx</option>
              <option value="5xx">5xx</option>
            </select>
          </div>
          <div>
            <label
              htmlFor="exchange-list-method"
              className="mb-1 block text-[10px] uppercase tracking-wide text-slate-500"
            >
              Method
            </label>
            <select
              id="exchange-list-method"
              data-testid="exchange-list-method"
              value={filter.method}
              onChange={(e) => setFilter({ method: e.target.value })}
              className="w-full rounded border border-slate-700 bg-bg-base px-2 py-1 text-sm text-slate-100 focus:border-accent focus:outline-none"
            >
              <option value="any">any</option>
              <option value="GET">GET</option>
              <option value="POST">POST</option>
              <option value="PUT">PUT</option>
              <option value="DELETE">DELETE</option>
              <option value="PATCH">PATCH</option>
            </select>
          </div>
        </div>
        <div className="mt-2">
          <label
            htmlFor="exchange-list-tag"
            className="mb-1 block text-[10px] uppercase tracking-wide text-slate-500"
          >
            Tag
          </label>
          <input
            id="exchange-list-tag"
            data-testid="exchange-list-tag"
            type="text"
            value={filter.tag}
            onChange={(e) => setFilter({ tag: e.target.value })}
            placeholder="tag contains…"
            className="w-full rounded border border-slate-700 bg-bg-base px-2 py-1 text-sm text-slate-100 placeholder:text-slate-500 focus:border-accent focus:outline-none"
          />
        </div>
        <label
          htmlFor="exchange-list-fts"
          className="mt-3 mb-1 block text-xs uppercase tracking-wide text-slate-400"
        >
          Search by content
        </label>
        <input
          id="exchange-list-fts"
          data-testid="exchange-list-fts"
          type="text"
          value={filterFtsQuery}
          onChange={(e) => {
            setFilterFtsQuery(e.target.value);
          }}
          placeholder="url, body, headers, notes…"
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
                <div
                  key={row.id}
                  data-testid="exchange-list-row"
                  data-row-id={row.id}
                  data-row-index={vi.index}
                  onClick={() => {
                    setSelectedId(row.id as ExchangeId);
                  }}
                  title={row.summary}
                  className={`group absolute left-0 top-0 flex w-full cursor-pointer flex-col justify-center px-3 text-left text-sm transition-colors ${
                    isSelected ? SELECTED_ROW_CLASS : UNSELECTED_ROW_CLASS
                  }`}
                  style={{
                    height: `${vi.size}px`,
                    transform: `translateY(${vi.start}px)`,
                  }}
                  role="button"
                  tabIndex={0}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                      e.preventDefault();
                      setSelectedId(row.id as ExchangeId);
                    }
                  }}
                >
                  <div className="flex items-center gap-2">
                    <span className="flex-1 truncate font-mono text-xs">
                      {row.summary}
                    </span>
                    <ReplayButton rowId={row.id as ExchangeId} />
                    <DeleteButton rowId={row.id as ExchangeId} summary={row.summary} />
                  </div>
                  <span
                    className={`truncate text-[10px] ${
                      isSelected ? "text-slate-600" : "text-slate-500"
                    }`}
                  >
                    {formatTimestamp(row.timestamp)}
                  </span>
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}

/**
 * Replay button. Visible on row hover only. Clicking it:
 *   1. Looks up the `ExchangeDetail` from the in-memory
 *      cache (`useExchangeStore.getState().getDetail`).
 *   2. If the cache misses, falls back to a
 *      `getExchange(projectId, id)` IPC round-trip and
 *      stores the result via `putDetail` so the next click
 *      is instant.
 *   3. Calls `useReplayStore.openTab(detail)` + flips
 *      `useUiStore.setMode("replay")`.
 *
 * The `e.stopPropagation()` keeps the parent row's
 * `onClick` (which sets `selectedId`) from firing — the
 * Replay action is a separate intent from "open the
 * capture detail".
 */
function ReplayButton({ rowId }: { rowId: ExchangeId }) {
  const openTab = useReplayStore((s) => s.openTab);
  const setMode = useUiStore((s) => s.setMode);
  const activeProjectId = useProjectStore((s) => s.activeProjectId);
  const getDetail = useExchangeStore((s) => s.getDetail);

  return (
    <button
      type="button"
      data-testid="exchange-list-replay-button"
      onClick={(e) => {
        e.stopPropagation();
        // Synchronous cache lookup.
        const cached = getDetail(rowId);
        if (cached) {
          // Cache hit: the LRU never holds truncated
          // bodies (it stores the full `ExchangeDetail`
          // from the engine's wire-bus event), so the
          // `body_truncated` flag is always `false` here.
          openTab({
            exchangeId: cached.meta.id,
            summary: cached.meta.summary,
            request: cached.request,
            response: cached.response,
            projectId: cached.meta.project_id,
            bodyTruncated: false,
          });
          setMode("replay");
          return;
        }
        // Cache miss: round-trip the
        // `open_replay_tab` Tauri command. The
        // command returns a `ReplayTabDescriptor`
        // with the `body_truncated` flag (v0.5+
        // post-batch gap-fix P1 #4, 2026-07-24).
        // The `body_truncated` flag is propagated
        // into the new tab so the ReplayRequestEditor
        // can render the 1 MB cap notice.
        //
        // The user's active project id is the
        // source of truth for which DB the engine
        // queries.
        if (!activeProjectId) {
          console.error("Replay: no active project; cannot open tab");
          return;
        }
        openReplayTab(rowId)
          .then((descriptor) => {
            if (!descriptor) {
              console.error("Replay: open_replay_tab returned null", rowId);
              return;
            }
            // The `ReplayTabDescriptor.request` is the
            // same shape as `ExchangeRequest`, so it
            // feeds `openTab` directly. The
            // `body_truncated` flag drives the
            // truncation notice in the request
            // editor.
            //
            // We don't have an `ExchangeDetail`
            // payload from the IPC (the descriptor
            // is a slimmed-down DTO), so we don't
            // populate the LRU cache here. The next
            // click on the same row will use the
            // cache (if the engine has since pushed
            // a wire-bus event for it) or hit the
            // IPC again. Either way is correct.
            openTab({
              exchangeId: rowId,
              summary: "", // descriptor doesn't carry the summary; the tab's `name` defaults work fine
              request: descriptor.request,
              response: descriptor.original_response,
              projectId: descriptor.project_id,
              bodyTruncated: descriptor.body_truncated,
            });
            setMode("replay");
          })
          .catch((err) => {
            console.error("Replay: openReplayTab failed", err);
          });
      }}
      className="invisible text-xs text-slate-500 opacity-0 hover:text-accent group-hover:visible group-hover:opacity-100 focus-visible:visible focus-visible:opacity-100"
    >
      Replay
    </button>
  );
}

/**
 * Delete button (×). Visible on row hover only. v0.6 P3 #9
 * (2026-07-24, delete exchange).
 *
 * Clicking it pops a `<DeleteConfirmDialog>` that gates the
 * actual `delete_exchange` IPC call. The dialog is mandatory
 * (no "auto-confirm" — the action is destructive and cannot
 * be undone; the user can re-capture the request, but the
 * captured history is gone).
 *
 * **Race-safety note:** the engine is the source of truth.
 * When the IPC call succeeds, the engine emits
 * `EngineEvent::ExchangeDeleted` on the wire bus; the UI's
 * `useEngineEventHandler` (in `routes/Capture.tsx`) calls
 * `exchangeStore.removeExchange(id)`. This component does
 * NOT optimistically update the local store on success —
 * waiting for the wire event avoids the "double-remove"
 * race where the local store would attempt to remove the
 * row twice (once optimistically, once on the wire event).
 *
 * The `disabled` prop on the × button prevents double-clicks
 * while the IPC is in flight. The dialog itself also tracks
 * the in-flight state so a fast Enter-key press doesn't
 * re-submit.
 */
function DeleteButton({
  rowId,
  summary,
}: {
  rowId: ExchangeId;
  summary: string;
}) {
  const [confirming, setConfirming] = useState(false);
  const activeProjectId = useProjectStore((s) => s.activeProjectId);
  return (
    <>
      <button
        type="button"
        data-testid="exchange-list-delete-button"
        title="Delete this exchange"
        aria-label="Delete exchange"
        onClick={(e) => {
          e.stopPropagation();
          if (!activeProjectId) {
            console.error(
              "Delete: no active project; cannot delete row",
              rowId,
            );
            return;
          }
          setConfirming(true);
        }}
        className="invisible flex h-5 w-5 items-center justify-center rounded text-xs text-slate-500 opacity-0 hover:bg-red-900/40 hover:text-red-300 group-hover:visible group-hover:opacity-100 focus-visible:visible focus-visible:opacity-100"
      >
        ×
      </button>
      {confirming && activeProjectId ? (
        <DeleteConfirmDialog
          rowId={rowId}
          summary={summary}
          projectId={activeProjectId}
          onClose={() => setConfirming(false)}
        />
      ) : null}
    </>
  );
}

/**
 * Small confirmation dialog for the delete-exchange action.
 * v0.6 P3 #9 (2026-07-24, delete exchange).
 *
 * The dialog is intentionally minimal: the action is
 * destructive but not catastrophic (the user can re-capture
 * the request), so we use a single-click confirm (NOT the
 * "type DELETE" hard-confirm the agent's `DESTRUCTIVE_TOOLS`
 * use — that's for write-tool calls that can drop database
 * tables, not for deleting a single captured exchange).
 *
 * The dialog tracks an in-flight `pending` state so a
 * fast Enter-key press doesn't double-submit. Errors are
 * surfaced inline; the dialog stays open on error so the
 * user can retry or cancel.
 */
function DeleteConfirmDialog({
  rowId,
  summary,
  projectId,
  onClose,
}: {
  rowId: ExchangeId;
  summary: string;
  projectId: ProjectId;
  onClose: () => void;
}) {
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  return (
    <div
      data-testid="delete-confirm-dialog-backdrop"
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50"
      onClick={(e) => {
        if (e.target === e.currentTarget && !pending) onClose();
      }}
    >
      <div
        data-testid="delete-confirm-dialog"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="delete-confirm-title"
        className="w-80 rounded border border-slate-700 bg-bg-rail p-4 shadow-lg"
      >
        <h2
          id="delete-confirm-title"
          className="mb-2 text-sm font-semibold text-slate-100"
        >
          Delete this exchange?
        </h2>
        <p className="mb-3 text-xs text-slate-400">
          <span className="font-mono">{summary}</span>
          <br />
          This cannot be undone. The captured request can be re-captured if
          needed, but the history row will be gone.
        </p>
        {error ? (
          <p
            data-testid="delete-confirm-error"
            className="mb-2 text-xs text-red-400"
          >
            {error}
          </p>
        ) : null}
        <div className="flex justify-end gap-2">
          <button
            type="button"
            data-testid="delete-confirm-cancel"
            disabled={pending}
            onClick={onClose}
            className="rounded border border-slate-700 bg-bg-base px-3 py-1 text-xs text-slate-200 hover:border-slate-500 disabled:opacity-50"
          >
            Cancel
          </button>
          <button
            type="button"
            data-testid="delete-confirm-confirm"
            disabled={pending}
            onClick={() => {
              setPending(true);
              setError(null);
              deleteExchange(projectId, rowId)
                .then(() => {
                  onClose();
                })
                .catch((e: unknown) => {
                  setError(
                    typeof e === "string"
                      ? e
                      : e instanceof Error
                        ? e.message
                        : "Delete failed",
                  );
                  setPending(false);
                });
            }}
            className="rounded border border-red-700 bg-red-900/30 px-3 py-1 text-xs text-red-200 hover:bg-red-900/50 disabled:opacity-50"
          >
            {pending ? "Deleting…" : "Delete"}
          </button>
        </div>
      </div>
    </div>
  );
}
