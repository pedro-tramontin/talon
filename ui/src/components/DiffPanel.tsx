// §4.7 DiffPanel. The "Diff" tab in the right-rail. Shows
// the line-level diff between the currently-selected
// exchange's response body and the previous response body
// for the same `METHOD URL` (the "previous" lookup uses the
// `summary` field on `ExchangeSummary` — the exchange
// store's thin row, which carries the label as
// `"METHOD URL"`, no full body).
//
// Spec (§4.7, v0.5+):
//   - The diff is **LCS-based** via the `diff` package's
//     `diffLines` (Myers O(ND) algorithm, equivalent to
//     LCS for the line-token case). The naïve line-by-line
//     index comparison from v0.1 is replaced — that
//     approach treated line N in A vs line N in B as a pair
//     and rendered the same row count, so it couldn't
//     represent insertions or pure deletions correctly.
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
//     binary case shows a size + content-type summary
//     (the hex view in the request/response inspector
//     tabs is the deeper hex view — the diff panel
//     keeps the lightweight summary).
//   - Large diffs (> `DIFF_MAX_LINES` total rendered
//     rows) are truncated with a "Show full diff" button
//     that renders the untruncated version below the
//     truncated one. Without the cap, a 200k-line JSON
//     response diff would lock the UI thread for ~2s
//     during the LCS pass + another ~5s for render
//     layout.
//
// Security: bodies are rendered inside `<div>` text nodes
// (no `dangerouslySetInnerHTML`). React's default escaping
// applies. The LCS pass operates on UTF-8 strings only —
// no eval, no regex backtracking.

import { useEffect, useMemo, useState } from "react";
import { diffLines } from "diff";
import { getExchange } from "../api";
import { useExchangeStore } from "../state/exchange";
import { useProjectStore } from "../state/project";
import type { ExchangeDetail } from "../types/domain";
import type { ExchangeId } from "../types/ids";
import { decodeBodyToBytes, decodeBodyUtf8 } from "../lib/body-decode";

/** Cap on the number of rendered rows before we collapse
 * the diff behind a "Show full diff" button. Keeps the
 * UI responsive for very large response bodies. */
const DIFF_MAX_LINES = 1000;

/** A single rendered line in the diff output. */
type DiffLine = {
  /** The text content (no trailing newline). */
  text: string;
  /** Line number in A (the previous response). `null`
   * if this line is a pure addition. */
  a: number | null;
  /** Line number in B (the current response). `null`
   * if this line is a pure removal. */
  b: number | null;
  /** The kind of line. */
  kind: "context" | "added" | "removed";
};

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

  // The LCS-based line diff. Re-computed when either
  // detail changes. The result is a flat array of
  // `DiffLine` objects in document order (A's lines
  // followed by B's lines, interleaved with context
  // blocks) — easier to render than the `Change` array
  // from `diffLines`.
  const { diffLines: renderedLines, stats } = useMemo(() => {
    if (!currentDetail || !previousDetail) {
      return { diffLines: [] as DiffLine[], stats: { added: 0, removed: 0, total: 0 } };
    }
    const aText = decodeBodyUtf8(previousDetail.response?.body);
    const bText = decodeBodyUtf8(currentDetail.response?.body);
    if (aText === null || bText === null) {
      return { diffLines: [] as DiffLine[], stats: { added: 0, removed: 0, total: 0 } };
    }
    // `newlineIsToken: true` makes the diff human-friendly:
    // the newlines are part of the tokens, so the
    // "value" of a common block doesn't end with `\n` and
    // get rendered as an empty line.
    // `oneChangePerToken: true` ensures each line is its
    // own `Change` object — required for the
    // line-numbered gutter to be correct.
    const changes = diffLines(aText, bText, {
      newlineIsToken: true,
      oneChangePerToken: true,
    });
    const out: DiffLine[] = [];
    let a = 0;
    let b = 0;
    let added = 0;
    let removed = 0;
    // `newlineIsToken: true` means a single visual line
    // "alpha\n" is split into TWO `Change` objects
    // (value: "alpha", value: "\n"). The "alpha" carries
    // the visible text; the "\n" is just a terminator.
    // We accumulate content + (optionally) a trailing
    // newline change, then emit a single `DiffLine` per
    // visual line.
    for (let i = 0; i < changes.length; i++) {
      const change = changes[i];
      const next = changes[i + 1];
      // If the next change is a same-status `\n`, fold
      // it into the current change's text and skip it.
      let text = change.value;
      if (next && next.value === "\n" && next.added === change.added && next.removed === change.removed) {
        text += "\n";
        i += 1;
      }
      if (change.added) {
        b += 1;
        added += 1;
        out.push({ text, a: null, b, kind: "added" });
      } else if (change.removed) {
        a += 1;
        removed += 1;
        out.push({ text, a, b: null, kind: "removed" });
      } else {
        a += 1;
        b += 1;
        out.push({ text, a, b, kind: "context" });
      }
    }
    return { diffLines: out, stats: { added, removed, total: out.length } };
  }, [currentDetail, previousDetail]);

  // The user can opt out of the line cap to render a
  // very large diff. We default to capped to keep the
  // UI responsive; "Show full diff" sets this to true
  // and the full array is rendered.
  const [showFull, setShowFull] = useState(false);
  const visibleLines = showFull
    ? renderedLines
    : renderedLines.slice(0, DIFF_MAX_LINES);
  const isTruncated = !showFull && renderedLines.length > DIFF_MAX_LINES;

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
    const bodyBytes = body ? decodeBodyToBytes(body) : null;
    if (body?.kind === "complete") {
      const ct = Object.entries(
        currentDetail.response.headers,
      ).find(([k]) => k.toLowerCase() === "content-type")?.[1];
      const size = bodyBytes?.length ?? 0;
      return (
        <div
          data-testid="diff-panel-binary"
          className="space-y-1 text-xs"
        >
          <p className="text-slate-500">
            Response body is binary ({" "}
            {ct?.split(";")[0]?.trim() ?? "application/octet-stream"},{" "}
            {size} B). Binary diff is a v0.5+ followup.
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

  // Use the decoded byte length (not the wire-form data.length,
  // which is the base64 char count in the v0.5 wire form).
  const aLength =
    previousDetail.response.body.kind === "complete"
      ? decodeBodyToBytes(previousDetail.response.body)?.length ?? 0
      : 0;
  const bLength =
    currentDetail.response.body.kind === "complete"
      ? decodeBodyToBytes(currentDetail.response.body)?.length ?? 0
      : 0;

  return (
    <div data-testid="diff-panel" className="font-mono text-xs">
      <h3
        data-testid="diff-panel-header"
        className="mb-2 text-xs font-bold uppercase text-slate-400"
      >
        vs. previous {previous.summary}
      </h3>
      <div
        data-testid="diff-panel-lines"
        className="space-y-0 overflow-x-auto"
      >
        {visibleLines.map((line, i) => {
          if (line.kind === "context") {
            return (
              <div
                key={i}
                data-testid="diff-panel-line-context"
                className="grid grid-cols-[3rem_3rem_1fr] gap-1 text-slate-500"
              >
                <span
                  data-testid="diff-panel-line-number-a"
                  className="select-none text-right text-slate-600"
                >
                  {line.a ?? ""}
                </span>
                <span
                  data-testid="diff-panel-line-number-b"
                  className="select-none text-right text-slate-600"
                >
                  {line.b ?? ""}
                </span>
                <span className="whitespace-pre-wrap break-all">
                  {line.text.length > 0 ? line.text : "\u00A0"}
                </span>
              </div>
            );
          }
          if (line.kind === "removed") {
            return (
              <div
                key={i}
                data-testid="diff-panel-line-removed"
                className="grid grid-cols-[3rem_3rem_1fr] gap-1 bg-red-900/30 text-red-300"
              >
                <span
                  data-testid="diff-panel-line-number-a"
                  className="select-none text-right text-slate-600"
                >
                  {line.a ?? ""}
                </span>
                <span
                  data-testid="diff-panel-line-number-b"
                  className="select-none text-right text-slate-600"
                >
                  {line.b ?? ""}
                </span>
                <span className="whitespace-pre-wrap break-all">
                  - {line.text}
                </span>
              </div>
            );
          }
          // added
          return (
            <div
              key={i}
              data-testid="diff-panel-line-added"
              className="grid grid-cols-[3rem_3rem_1fr] gap-1 bg-green-900/30 text-green-300"
            >
              <span
                data-testid="diff-panel-line-number-a"
                className="select-none text-right text-slate-600"
              >
                {line.a ?? ""}
              </span>
              <span
                data-testid="diff-panel-line-number-b"
                className="select-none text-right text-slate-600"
              >
                {line.b ?? ""}
              </span>
              <span className="whitespace-pre-wrap break-all">
                + {line.text}
              </span>
            </div>
          );
        })}
      </div>
      {isTruncated && (
        <div
          data-testid="diff-panel-truncated"
          className="mt-2 flex items-center gap-2 text-slate-500"
        >
          <p>
            Diff truncated: showing {DIFF_MAX_LINES} of {renderedLines.length} lines.
          </p>
          <button
            type="button"
            data-testid="diff-panel-show-full"
            onClick={() => setShowFull(true)}
            className="rounded border border-slate-600 bg-transparent px-2 py-0.5 text-slate-200 hover:border-slate-400"
          >
            Show full diff
          </button>
        </div>
      )}
      <p
        data-testid="diff-panel-summary"
        className="mt-2 text-slate-500"
      >
        A: {aLength} B, B: {bLength} B
        {stats.total > 0 && (
          <>
            {" · "}+{stats.added} -{stats.removed}
          </>
        )}
      </p>
    </div>
  );
}
