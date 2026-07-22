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

import { useEffect, useState } from "react";
import { decodeBodyUtf8 } from "../lib/body-decode";
import { sendReplay } from "../api";
import { useReplayStore } from "../state/replay";
import { useExchangeStore } from "../state/exchange";
import { useProjectStore } from "../state/project";
import type {
  ExchangeBody,
  ExchangeRequest,
  ExchangeResponse,
} from "../types/domain";
import type { ExchangeId } from "../types/ids";

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

  return (
    <div
      data-testid="replay-request-editor"
      className="flex h-full flex-col space-y-2 bg-bg-base p-3 text-xs"
    >
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
      <textarea
        data-testid="replay-request-editor-body"
        value={bodyText}
        onChange={(e) => setBodyText(e.target.value)}
        placeholder="Body (text or base64-encoded binary)"
        className="flex-1 resize-none rounded border border-slate-600 bg-bg-panel px-2 py-1 font-mono text-slate-100 focus:border-accent focus:outline-none"
      />
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
