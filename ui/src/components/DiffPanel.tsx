// §4.7 DiffPanel. The "Diff" tab in the right-rail. Shows
// the line-level diff between the currently-selected
// exchange's response body and the previous response body
// for the same `METHOD URL` (the "previous" lookup uses the
// `summary` field on `ExchangeSummary` — the exchange
// store's thin row, which carries the label as
// `"METHOD URL"`, no full body).
//
// Spec (§4.7):
//   - The diff is **line-based and naïve** (compare line
//     N in A with line N in B; mark lines that differ).
//     O(n) in lines, no LCS. v0.5 will swap in a real
//     LCS-based diff. Naïve is correct for the v1 use
//     case (typical response bodies are <10k lines).
//   - "Previous" means the next-newer same-`summary`
//     exchange in the store. The store is
//     reverse-chronological (newest first — the §4.5 list
//     prepends on insert), so we skip the current row and
//     take the first match.
//   - The previous exchange's full body is fetched via
//     the §4.1 `get_exchange` Tauri command (the
//     `ExchangeSummary` is a thin row — the response
//     body lives in `ExchangeDetail`). The same command
//     serves the current row.
//   - If there's no previous match, the panel renders
//     an empty state ("no previous response to compare
//     against").
//   - If either response is missing (in-flight request,
//     blocked request, streaming body), the panel
//     renders an explanatory empty state instead of a
//     partial diff.
//   - Bodies are decoded as UTF-8 via `TextDecoder`; the
//     binary case is flagged as a v0.5 followup.
//
// Security: bodies are rendered inside `<div>` text nodes
// (no `dangerouslySetInnerHTML`). React's default escaping
// applies. The diff is a line-by-line comparison — no
// regex backtracking, no script eval.

import { useEffect, useMemo, useState } from "react";
import { getExchange } from "../api";
import { useExchangeStore } from "../state/exchange";
import { useProjectStore } from "../state/project";
import type { ExchangeBody, ExchangeDetail } from "../types/domain";
import type { ExchangeId } from "../types/ids";

/** Decode a `Body::Complete` payload to a UTF-8 string.
 * Returns `null` if the bytes are not valid UTF-8.
 * Mirrors `InspectorPanel.decodeBodyUtf8`. */
function decodeBodyUtf8(body: ExchangeBody): string | null {
  if (body.kind !== "complete") return null;
  if (body.data.length === 0) return "";
  const bytes = new Uint8Array(body.data);
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    return null;
  }
}

/** The "previous" summary we picked. Carried in state so
 * the diff header can show its summary line. */
interface PickedPrevious {
  id: ExchangeId;
  summary: string;
}

/**
 * Compute the diff state for the currently-selected
 * exchange. The "previous" lookup walks the exchange
 * store (the thin `ExchangeSummary` array) and finds
 * the first row whose `summary` matches the current
 * row's `summary` (the next-newer match, since the
 * store is reverse-chronological). The async part —
 * fetching both `ExchangeDetail` payloads — is run
 * inside a `useEffect` and the result is stored in
 * state.
 */
export function DiffPanel() {
  const exchanges = useExchangeStore((s) => s.exchanges);
  const selectedId = useExchangeStore((s) => s.selectedId);
  const activeProjectId = useProjectStore((s) => s.activeProjectId);

  // The current row's summary line — used for both the
  // header and the previous-match lookup. Computed
  // every render (it's a cheap `.find`).
  const current = useMemo(
    () => (selectedId ? exchanges.find((e) => e.id === selectedId) : null),
    [exchanges, selectedId],
  );

  // The picked previous summary. `null` when there's no
  // match OR no selection. `useMemo` so the lookup
  // doesn't re-fire on every store notification.
  const previous = useMemo<PickedPrevious | null>(() => {
    if (!current) return null;
    const match = exchanges.find(
      (e) => e.id !== current.id && e.summary === current.summary,
    );
    if (!match) return null;
    return { id: match.id, summary: match.summary };
  }, [exchanges, current]);

  // The two `ExchangeDetail` payloads. The current one
  // is loaded on `selectedId` change; the previous one
  // is loaded on `previous?.id` change. We keep them in
  // separate state slots so a reload of the current
  // doesn't drop the previous.
  const [currentDetail, setCurrentDetail] = useState<ExchangeDetail | null>(
    null,
  );
  const [previousDetail, setPreviousDetail] = useState<ExchangeDetail | null>(
    null,
  );
  const [loadError, setLoadError] = useState<string | null>(null);

  // Fetch the current detail.
  useEffect(() => {
    if (!selectedId || !activeProjectId) {
      setCurrentDetail(null);
      setLoadError(null);
      return;
    }
    let cancelled = false;
    setLoadError(null);
    getExchange(activeProjectId, selectedId)
      .then((d) => {
        if (cancelled) return;
        setCurrentDetail(d);
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        setLoadError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [selectedId, activeProjectId]);

  // Fetch the previous detail. The id comes from the
  // `previous` summary-match.
  useEffect(() => {
    if (!previous || !activeProjectId) {
      setPreviousDetail(null);
      return;
    }
    let cancelled = false;
    getExchange(activeProjectId, previous.id)
      .then((d) => {
        if (cancelled) return;
        setPreviousDetail(d);
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        setLoadError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [previous, activeProjectId]);

  if (!selectedId) {
    return (
      <p
        data-testid="diff-panel-no-selection"
        className="text-sm text-slate-500"
      >
        No exchange selected.
      </p>
    );
  }
  if (!previous) {
    return (
      <p
        data-testid="diff-panel-no-previous"
        className="text-sm text-slate-500"
      >
        No previous response to compare against.
      </p>
    );
  }
  if (loadError) {
    return (
      <div
        data-testid="diff-panel-error"
        className="rounded border border-red-700 bg-red-900/20 p-3 text-sm text-red-300"
      >
        <strong>Error loading detail:</strong> {loadError}
      </div>
    );
  }
  if (!currentDetail || !previousDetail) {
    return (
      <p
        data-testid="diff-panel-loading"
        className="text-sm text-slate-500"
      >
        Loading…
      </p>
    );
  }
  if (!currentDetail.response || !previousDetail.response) {
    return (
      <p
        data-testid="diff-panel-missing-response"
        className="text-sm text-slate-500"
      >
        Cannot diff: one or both responses are missing.
      </p>
    );
  }
  const aText = decodeBodyUtf8(previousDetail.response.body);
  const bText = decodeBodyUtf8(currentDetail.response.body);
  if (aText === null || bText === null) {
    // Binary. Identify which side tripped the check so
    // the placeholder has the right size + content-type.
    const binarySide = aText === null ? previousDetail : currentDetail;
    const body = binarySide.response?.body;
    if (body?.kind === "complete") {
      const ct = Object.entries(
        currentDetail.response.headers,
      ).find(([k]) => k.toLowerCase() === "content-type")?.[1];
      return (
        <div
          data-testid="diff-panel-binary"
          className="space-y-1 text-xs"
        >
          <p className="text-slate-500">
            Response body is binary ({" "}
            {ct?.split(";")[0]?.trim() ?? "application/octet-stream"},{" "}
            {body.data.length} B). Hex diff is a v0.5 followup.
          </p>
        </div>
      );
    }
    return (
      <p
        data-testid="diff-panel-missing-response"
        className="text-sm text-slate-500"
      >
        Cannot diff: one or both responses are missing.
      </p>
    );
  }

  const aLines = aText.split("\n");
  const bLines = bText.split("\n");
  const max = Math.max(aLines.length, bLines.length);
  const aLength =
    previousDetail.response.body.kind === "complete"
      ? previousDetail.response.body.data.length
      : 0;
  const bLength =
    currentDetail.response.body.kind === "complete"
      ? currentDetail.response.body.data.length
      : 0;

  return (
    <div data-testid="diff-panel" className="font-mono text-xs">
      <h3
        data-testid="diff-panel-header"
        className="mb-2 text-xs font-bold uppercase text-slate-400"
      >
        vs. previous {previous.summary}
      </h3>
      <div data-testid="diff-panel-lines" className="space-y-0">
        {Array.from({ length: max }, (_, i) => {
          const la = aLines[i] ?? "";
          const lb = bLines[i] ?? "";
          if (la === lb) {
            return (
              <div
                key={i}
                data-testid="diff-panel-line-context"
                className="text-slate-500"
              >
                {la.length > 0 ? la : "\u00A0"}
              </div>
            );
          }
          return (
            <div key={i}>
              {la.length > 0 && (
                <div
                  data-testid="diff-panel-line-removed"
                  className="bg-red-900/30 text-red-300"
                >
                  - {la}
                </div>
              )}
              {lb.length > 0 && (
                <div
                  data-testid="diff-panel-line-added"
                  className="bg-green-900/30 text-green-300"
                >
                  + {lb}
                </div>
              )}
            </div>
          );
        })}
      </div>
      <p
        data-testid="diff-panel-summary"
        className="mt-2 text-slate-500"
      >
        A: {aLength} B, B: {bLength} B
      </p>
    </div>
  );
}
