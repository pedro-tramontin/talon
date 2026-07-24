// Replay request editor. Three textareas (request line
// `METHOD url`, headers as `Header-Name: value` lines, body)
// + a "Send" button that calls `sendReplay` from the IPC
// bridge. On success: `setDraft` + `appendSend` + `prepend`
// (the capture list) + the `useExchangesStore.putDetail`
// cache update. On error: `appendSend` with `null`
// response + `console.error`.
//
// The textareas re-sync on tab switch (via a `useEffect`
// keyed on `tab.id` per the v0.5 pattern). The `sending`
// flag disables the button while the IPC is in flight.
//
// Uses the v0.5 `decodeBodyUtf8` helper from
// `ui/src/lib/body-decode.ts` for body decoding (NOT raw
// `atob`) — the latter would silently create a UTF-8 view
// of the base64 chars instead of the decoded bytes.
//
// Phase 5 — §5.4.
//
// ## Phase 7 C-B.5 — Raw / Pretty sub-tabs + "fork from
// history" same-tab re-sync
//
// The body textarea is now wrapped in a Raw/Pretty sub-tab
// strip. Default: "Raw" (the existing textarea).
//   - "Pretty" with a JSON body → a `JsonTreeView` of the
//     parsed object (read-only in v1; a separate v0.5+ item
//     would add a "Pretty edit" mode).
//   - "Pretty" with a form-data body (key=val OR
//     `Content-Type: application/x-www-form-urlencoded`)
//     → a key-value table.
//   - "Pretty" with an unrecognized body → a fallback
//     message ("Pretty view unavailable for this body").
//   - Pretty view is capped at 1 MB (matches the Phase 5
//     + Phase 7 C-A.4 body cap).
//
// A second `useEffect` keyed on `JSON.stringify(tab.draftRequest)`
// re-syncs the textareas when the draft changes in the
// same tab (the original `tab?.id` keyed effect only
// handled tab switches). This is the load-bearing piece
// for the "fork from history" action — the
// `ReplayHistoryPanel`'s "Fork" button mutates
// `tab.draftRequest` via `setDraft`, and the textareas
// need to re-sync to show the new content.

import { useEffect, useState } from "react";
import { decodeBodyUtf8 } from "../lib/body-decode";
import { sendReplay } from "../api";
import { useReplayStore } from "../state/replay";
import { useExchangeStore } from "../state/exchange";
import { useProjectStore } from "../state/project";
import { JsonTreeView } from "./JsonTreeView";
import { parseFormData } from "../lib/form_data";
import type {
  ExchangeBody,
  ExchangeRequest,
  ExchangeResponse,
} from "../types/domain";
import type { ExchangeId } from "../types/ids";

/** 1 MB cap on the Pretty view (matches the Phase 5 +
 *  Phase 7 C-A.4 body caps). The Raw textarea shows the
 *  full body regardless. */
const PRETTY_VIEW_BODY_CAP = 1_000_000;

/** "Raw" or "Pretty" sub-tab state (per-component, not
 *  in the store — the user can switch freely without
 *  round-tripping through the IPC). */
type BodyView = "raw" | "pretty";

interface Props {
  tabId: string;
}

export function ReplayRequestEditor({ tabId }: Props) {
  const tab = useReplayStore((s) => s.tabs.find((t) => t.id === tabId));
  const setDraft = useReplayStore((s) => s.setDraft);
  const appendSend = useReplayStore((s) => s.appendSend);
  const setSending = useReplayStore((s) => s.setSending);
  const prependSummary = useExchangeStore((s) => s.unshiftExchange);
  const putDetail = useExchangeStore((s) => s.putDetail);
  const activeProjectId = useProjectStore((s) => s.activeProjectId);

  const [requestLine, setRequestLine] = useState("");
  const [headersText, setHeadersText] = useState("");
  const [bodyText, setBodyText] = useState("");
  const [error, setError] = useState<string | null>(null);
  // Phase 7 C-B.5: which sub-tab is active. Default
  // "raw" matches the v0.1 behavior (the existing
  // textarea was always shown).
  const [bodyView, setBodyView] = useState<BodyView>("raw");

  // Re-sync the textareas on tab switch (NOT on every
  // keystroke). The v0.5 pattern: the parent passes
  // `tab.id` as a dependency, and the inner state is
  // discarded on switch (the new tab's draft is shown
  // fresh).
  useEffect(() => {
    if (!tab) return;
    const r = tab.draftRequest;
    setRequestLine(`${r.method} ${r.url}`);
    setHeadersText(
      Object.entries(r.headers)
        .map(([k, v]) => `${k}: ${v}`)
        .join("\n"),
    );
    setBodyText(decodeBodyUtf8(r.body) ?? "");
    setError(null);
    // We deliberately depend on `tab?.id` only — the
    // `draftRequest` content is the new state we want to
    // reflect on switch; per-keystroke updates flow through
    // `setDraft` (which mutates the store) but we don't
    // need to re-sync the textareas on every keystroke.
  }, [tab?.id]);

  // Phase 7 C-B.5 (D3 spec drift fix): same-tab re-sync.
  // When `tab.draftRequest` changes WITHOUT a tab switch
  // (e.g. the "fork from history" action calls `setDraft`
  // on the active tab), the textareas need to re-sync.
  // The textarea-to-store path already flows through
  // `setDraft` on every keystroke, so this effect is
  // idempotent — it only fires when the draft *content*
  // changes from outside the editor.
  useEffect(() => {
    if (!tab) return;
    const r = tab.draftRequest;
    const nextLine = `${r.method} ${r.url}`;
    const nextHeaders = Object.entries(r.headers)
      .map(([k, v]) => `${k}: ${v}`)
      .join("\n");
    const nextBody = decodeBodyUtf8(r.body) ?? "";
    // Only re-sync if the content actually changed
    // (the keystroke path is identical; we just don't
    // want to clobber the user's in-flight edits).
    setRequestLine((cur) => (cur === nextLine ? cur : nextLine));
    setHeadersText((cur) => (cur === nextHeaders ? cur : nextHeaders));
    setBodyText((cur) => (cur === nextBody ? cur : nextBody));
    // We depend on the stringified draft so a new
    // reference with the same content is a no-op
    // (per-keystroke `setDraft` calls produce a new
    // reference, but the `cur === next` guard means
    // we don't clobber).
  }, [tab ? JSON.stringify(tab.draftRequest) : null]);

  if (!tab) return null;

  const send = async () => {
    if (!activeProjectId) {
      setError("No active project");
      return;
    }
    // Parse the request line back into method + url.
    // Tolerate the method/url being separated by multiple
    // spaces (Burp-style). The first token is the method;
    // everything after is the url.
    const trimmed = requestLine.trimStart();
    const spaceIdx = trimmed.indexOf(" ");
    const method = spaceIdx > 0 ? trimmed.slice(0, spaceIdx) : trimmed;
    const url = spaceIdx > 0 ? trimmed.slice(spaceIdx + 1).trim() : "";

    // Parse the headers text.
    const headers: Record<string, string> = {};
    for (const line of headersText.split("\n")) {
      const m = line.match(/^([^:]+):\s*(.*)$/);
      if (m) headers[m[1].trim()] = m[2];
    }

    // Build the request DTO. The body is base64-encoded
    // for the v0.5 wire form (`Body::Complete { data: <b64> }`).
    const body: ExchangeBody = {
      kind: "complete",
      data: btoa(unescape(encodeURIComponent(bodyText))),
    };
    const request: ExchangeRequest = {
      method,
      url,
      version: "HTTP/1.1",
      headers,
      body,
    };

    setSending(tabId, true);
    setError(null);
    try {
      const exchange = await sendReplay(activeProjectId, request);
      setDraft(tabId, request);
      const response: ExchangeResponse | null = exchange.response ?? null;
      const exchangeId: ExchangeId | null = exchange.meta
        ? (exchange.meta.id as ExchangeId)
        : null;
      appendSend(tabId, request, response, exchangeId);
      // Update the capture list (ExchangeList) so the
      // new exchange shows up at the top, and the
      // `ExchangeDetail` cache so the right-rail reads
      // fresh data without a Tauri round-trip.
      if (exchangeId) {
        putDetail(exchange);
        prependSummary({
          id: exchangeId,
          project_id: activeProjectId,
          timestamp: exchange.meta.timestamp,
          duration_ns: exchange.meta.duration_ns,
          summary: exchange.meta.summary,
          scope_state: "unscoped",
          starred: exchange.meta.starred,
          notes: exchange.meta.notes,
          // v0.6 P2 #6: pass through the new fields
          // from the in-memory `HttpExchange` (the
          // wire event carries the full body).
          method: exchange.meta.method,
          status: exchange.meta.status,
          tags: exchange.meta.tags,
        });
      }
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      console.error("send_replay failed:", msg);
      setError(msg);
      appendSend(tabId, request, null, null);
    } finally {
      setSending(tabId, false);
    }
  };

  // Phase 7 C-B.5: Pretty view rendering. The body
  // rendering is chosen based on (a) the active tab
  // (`Pretty` vs `Raw`) and (b) the body content +
  // Content-Type header.
  const isOverCap = bodyText.length > PRETTY_VIEW_BODY_CAP;
  const contentType = (() => {
    for (const line of headersText.split("\n")) {
      const m = line.match(/^[^:]+:\s*(.*)$/);
      if (!m) continue;
      const name = line.split(":")[0].trim().toLowerCase();
      if (name === "content-type") return m[1].trim().toLowerCase();
    }
    return "";
  })();
  const trimmedBody = bodyText.trim();
  const isFormDataByContentType = contentType.includes(
    "application/x-www-form-urlencoded",
  );
  const isFormDataByShape = /^[^\s=]+=/.test(trimmedBody);
  const isFormData = isFormDataByContentType || isFormDataByShape;
  const isJson = trimmedBody.startsWith("{") || trimmedBody.startsWith("[");
  const prettyRender = (() => {
    if (isOverCap) {
      return (
        <p
          data-testid="replay-request-editor-pretty-cap"
          className="text-xs italic text-slate-500"
        >
          Body too large for Pretty view (&gt; {PRETTY_VIEW_BODY_CAP}{" "}
          bytes). Switch to Raw to see the full body.
        </p>
      );
    }
    if (isJson) {
      try {
        const parsed = JSON.parse(bodyText);
        return (
          <div
            data-testid="replay-request-editor-pretty-json"
            className="rounded border border-slate-700 bg-slate-900 p-2"
          >
            <JsonTreeView value={parsed} />
          </div>
        );
      } catch {
        return (
          <p
            data-testid="replay-request-editor-pretty-fallback"
            className="text-xs italic text-slate-500"
          >
            Pretty view unavailable: invalid JSON.
          </p>
        );
      }
    }
    if (isFormData) {
      const pairs = parseFormData(bodyText);
      return (
        <div
          data-testid="replay-request-editor-pretty-form"
          className="rounded border border-slate-700 bg-slate-900 p-2"
        >
          <table className="w-full text-xs">
            <thead>
              <tr className="text-left text-slate-400">
                <th className="py-1">Key</th>
                <th>Value</th>
              </tr>
            </thead>
            <tbody>
              {pairs.length === 0 ? (
                <tr>
                  <td
                    colSpan={2}
                    data-testid="replay-request-editor-pretty-form-empty"
                    className="py-2 text-center italic text-slate-500"
                  >
                    (empty)
                  </td>
                </tr>
              ) : (
                pairs.map(([k, v], i) => (
                  <tr
                    key={i}
                    data-testid={`replay-request-editor-pretty-form-row-${i}`}
                    className="border-t border-slate-700"
                  >
                    <td className="py-1 font-mono text-slate-300">{k}</td>
                    <td className="font-mono text-slate-300">{v}</td>
                  </tr>
                ))
              )}
            </tbody>
          </table>
        </div>
      );
    }
    return (
      <p
        data-testid="replay-request-editor-pretty-fallback"
        className="text-xs italic text-slate-500"
      >
        Pretty view unavailable for this body.
      </p>
    );
  })();

  return (
    <div
      data-testid="replay-request-editor"
      className="flex h-full flex-col space-y-2 bg-bg-base p-3 text-xs"
    >
      {/* v0.5+ post-batch gap-fix P1 #4 (2026-07-24):
       * render the 1 MB body-cap notice when the
       * source tab was opened from a truncated
       * `openReplayTab` response. Hidden when the
       * flag is `false` (the cache-hit path; the LRU
       * never holds truncated bodies). */}
      {tab?.bodyTruncated && (
        <div
          data-testid="replay-request-editor-body-truncated-notice"
          role="status"
          className="rounded border border-amber-700 bg-amber-900/30 px-3 py-2 text-amber-200"
        >
          Response body truncated to 1 MB by the engine. Re-send the
          request to see the full response body.
        </div>
      )}
      <input
        data-testid="replay-request-editor-line"
        value={requestLine}
        onChange={(e) => setRequestLine(e.target.value)}
        className="rounded border border-slate-600 bg-bg-panel px-2 py-1 font-mono text-slate-100 focus:border-accent focus:outline-none"
        placeholder="METHOD https://example.com/path"
      />
      <textarea
        data-testid="replay-request-editor-headers"
        value={headersText}
        onChange={(e) => setHeadersText(e.target.value)}
        placeholder="Header-Name: value"
        className="h-24 resize-y rounded border border-slate-600 bg-bg-panel px-2 py-1 font-mono text-slate-100 focus:border-accent focus:outline-none"
      />
      <div className="flex gap-1">
        <button
          type="button"
          data-testid="replay-request-editor-tab-raw"
          onClick={() => setBodyView("raw")}
          className={`rounded px-2 py-1 text-xs ${
            bodyView === "raw"
              ? "bg-accent text-bg-base"
              : "bg-slate-800 text-slate-300 hover:bg-slate-700"
          }`}
        >
          Raw
        </button>
        <button
          type="button"
          data-testid="replay-request-editor-tab-pretty"
          onClick={() => setBodyView("pretty")}
          className={`rounded px-2 py-1 text-xs ${
            bodyView === "pretty"
              ? "bg-accent text-bg-base"
              : "bg-slate-800 text-slate-300 hover:bg-slate-700"
          }`}
        >
          Pretty
        </button>
      </div>
      {bodyView === "raw" ? (
        <textarea
          data-testid="replay-request-editor-body"
          value={bodyText}
          onChange={(e) => setBodyText(e.target.value)}
          placeholder="Body (text or base64-encoded binary)"
          className="flex-1 resize-none rounded border border-slate-600 bg-bg-panel px-2 py-1 font-mono text-slate-100 focus:border-accent focus:outline-none"
        />
      ) : (
        <div
          data-testid="replay-request-editor-pretty"
          className="flex-1 overflow-auto rounded border border-slate-600 bg-bg-panel p-2"
        >
          {prettyRender}
        </div>
      )}
      {error && (
        <p
          data-testid="replay-request-editor-error"
          className="text-xs text-red-400"
        >
          {error}
        </p>
      )}
      <div className="flex items-center justify-end gap-2">
        {tab.sending && (
          <span
            data-testid="replay-request-editor-sending"
            className="text-slate-500"
          >
            Sending…
          </span>
        )}
        <button
          type="button"
          data-testid="replay-request-editor-send"
          onClick={send}
          disabled={tab.sending}
          className="rounded bg-accent px-3 py-1 font-medium text-bg-base hover:bg-cyan-300 disabled:opacity-50"
        >
          Send
        </button>
      </div>
    </div>
  );
}
