// Exchange detail panel. The main column of the Capture
// route. Renders the selected exchange's request and
// response with three sub-tabs each (pretty / headers /
// raw), plus a header bar (method + URL + duration) and a
// "blocked reason" banner when the proxy decided not to
// forward the request.
//
// Spec (§4.6):
//   - Reads `selectedId` from `useExchangeStore`.
//   - Empty state when no row is selected.
//   - Fetches the full `ExchangeDetail` via the §4.1
//     `get_exchange` Tauri command when a row is selected
//     (the list only carries the thin `ExchangeSummary`
//     row, so the request/response bodies need a second
//     round-trip).
//   - Re-fetches when `selectedId` changes (the lookup is
//     `useEffect` driven — we don't poll).
//   - The summary row is read from the store (it's already
//     in the Zustand state from the §4.5 list fetch) so the
//     header bar can render immediately while the full
//     detail is in flight.
//
// Two-step render (header-bar from summary, body from
// detail) means the user sees a non-empty header the
// moment they click a row. v0.1's detail is loaded
// on-demand; a future v0.5+ optimization could prefetch
// the detail on row hover, but that's out of scope for
// this PR.

import { useEffect, useState } from "react";
import { getExchange } from "../api";
import { useExchangeStore } from "../state/exchange";
import { useProjectStore } from "../state/project";
import type { ExchangeDetail } from "../types/domain";
import type { ExchangeId } from "../types/ids";
import { RequestInspector } from "./RequestInspector";
import { ResponseInspector } from "./ResponseInspector";

export interface ExchangeDetailProps {
  /** Optional test override — skips the `getExchange`
   * round-trip and renders the supplied detail directly.
   * Production callers leave this undefined. */
  testDetail?: ExchangeDetail;
  /** Optional test override for the `selectedId` lookup.
   * Production callers leave this undefined. */
  testSelectedId?: ExchangeId | null;
}

/**
 * ExchangeDetail. Main-column inspector that shows the
 * currently-selected exchange. Re-renders on every store
 * change (the `selectedId` selector is narrow enough that
 * the React subscription is cheap).
 */
export function ExchangeDetail(
  props: ExchangeDetailProps = {},
): React.ReactElement {
  const storeSelectedId = useExchangeStore((s) => s.selectedId);
  const selectedId = props.testSelectedId !== undefined
    ? props.testSelectedId
    : storeSelectedId;
  const summary = useExchangeStore((s) =>
    selectedId ? s.exchanges.find((e) => e.id === selectedId) : undefined,
  );
  const activeProjectId = useProjectStore((s) => s.activeProjectId);

  const [detail, setDetail] = useState<ExchangeDetail | null>(
    props.testDetail ?? null,
  );
  const [loadError, setLoadError] = useState<string | null>(null);

  // Fetch the full detail when the selection changes.
  // The lookup is gated on both `selectedId` and the
  // active project (the engine's `get_exchange` requires
  // a project context — exchanges are scoped to a
  // project).
  useEffect(() => {
    if (props.testDetail) {
      // Tests inject the detail directly — skip the IPC.
      return;
    }
    if (!selectedId || !activeProjectId) {
      setDetail(null);
      setLoadError(null);
      return;
    }
    let cancelled = false;
    setLoadError(null);
    getExchange(activeProjectId, selectedId)
      .then((d) => {
        if (cancelled) return;
        setDetail(d);
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        setLoadError(String(e));
        setDetail(null);
      });
    return () => {
      cancelled = true;
    };
  }, [selectedId, activeProjectId, props.testDetail]);

  if (!selectedId) {
    return (
      <main
        data-testid="exchange-detail-empty"
        className="flex h-full flex-1 items-center justify-center bg-bg-base"
      >
        <p className="text-sm text-slate-500">
          Select an exchange to view its request and response.
        </p>
      </main>
    );
  }

  // The header bar reads from the summary (always
  // available) so the user sees something useful even
  // while the full detail is loading.
  const headerMethod = detail?.request.method ?? summary?.summary.split(" ")[0] ?? "";
  const headerUrl = detail?.request.url ?? summary?.summary ?? "";
  const durationNs = detail?.meta.duration_ns ?? summary?.duration_ns ?? 0;
  const durationMs = Math.round(durationNs / 1_000_000);

  return (
    <main
      data-testid="exchange-detail"
      className="flex h-full flex-1 flex-col overflow-hidden bg-bg-base"
    >
      <div
        data-testid="exchange-detail-header"
        className="flex items-center gap-3 border-b border-slate-700 px-3 py-2"
      >
        <span className="font-mono text-sm font-bold text-slate-200">
          {headerMethod}
        </span>
        <span className="flex-1 truncate font-mono text-sm text-slate-300">
          {headerUrl}
        </span>
        <span className="font-mono text-xs text-slate-500">
          {durationMs}ms
        </span>
      </div>
      <div className="flex-1 space-y-3 overflow-y-auto p-3">
        {loadError && (
          <div
            data-testid="exchange-detail-error"
            className="rounded border border-red-700 bg-red-900/20 p-3 text-sm text-red-300"
          >
            <strong>Error loading detail:</strong> {loadError}
          </div>
        )}
        {!detail && !loadError && (
          <div
            data-testid="exchange-detail-loading"
            className="p-3 text-sm text-slate-500"
          >
            Loading…
          </div>
        )}
        {detail && (
          <>
            <RequestInspector request={detail.request} />
            {detail.response && (
              <ResponseInspector response={detail.response} />
            )}
            {detail.blocked_reason && (
              <div
                data-testid="exchange-detail-blocked"
                className="rounded border border-scope-blocked bg-red-900/20 p-3 text-sm text-red-300"
              >
                <strong>Blocked:</strong> {detail.blocked_reason}
              </div>
            )}
          </>
        )}
      </div>
    </main>
  );
}
