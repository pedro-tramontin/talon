// §4.7 InspectorPanel. The "Inspector" tab in the
// right-rail. Renders a structured view of the selected
// exchange's request: query params, request headers,
// cookies, and the JSON body (when parseable).
//
// Spec (§4.7):
//   - Reads `selectedId` from `useExchangeStore` (the
//     same source of truth the main panel uses).
//   - Pulls the full `ExchangeDetail` via the §4.1
//     `get_exchange` Tauri command so the request body is
//     available (the store only carries the thin
//     `ExchangeSummary`).
//   - Empty state when no row is selected.
//   - Empty state when the load is in flight / fails.
//   - Cookies are extracted from the `Cookie` header
//     (the wire format is a `;`-separated `key=value`
//     list). The response's `Set-Cookie` is NOT in scope
//     for v1 — `Set-Cookie` is a single header per
//     set-cookie, and the wire shape collapses it into a
//     single string in v0.1's DTO. v0.5 will add a
//     proper response-cookie view.
//   - Binary bodies (not valid UTF-8) get a
//     `[binary: <mime>, <size>]` placeholder, matching
//     the `RequestInspector` pattern. A hex viewer is a
//     v0.5 followup.
//
// Security: the body is rendered inside `<pre>` (not
// interpreted as HTML). React's default escaping applies.

import { useEffect, useState } from "react";
import { getExchange } from "../api";
import { useExchangeStore } from "../state/exchange";
import { useProjectStore } from "../state/project";
import type { ExchangeDetail, ExchangeBody } from "../types/domain";

/**
 * Decode a `Body::Complete` payload (a base64 string on the
 * wire as of v0.5; the v0.1 form was a JSON byte array) to a
 * `Uint8Array`. The Rust side serializes via
 * `body_complete_data_serde` (see `crates/bk-core/src/model.rs`)
 * which uses standard base64 alphabet with `=` padding.
 *
 * The deserializer ALSO accepts the legacy `Vec<u8>` array
 * form (for backwards compat with already-stored SQLite rows
 * and any test fixture written before the v0.5 fixup). The
 * detection is by type: if `body.data` is a string, base64-
 * decode it; if it's an array, treat it as bytes directly.
 *
 * Returns `null` if the input is not valid base64 (which
 * surfaces as a binary body in the UI — the same as the
 * not-valid-UTF-8 case downstream).
 */
function decodeBodyToBytes(body: ExchangeBody): Uint8Array | null {
  if (body.kind !== "complete") return null;
  const data = body.data;
  if (typeof data === "string") {
    // New v0.5 form: base64 string.
    if (data.length === 0) return new Uint8Array(0);
    try {
      const binary = atob(data);
      const out = new Uint8Array(binary.length);
      for (let i = 0; i < binary.length; i++) {
        out[i] = binary.charCodeAt(i);
      }
      return out;
    } catch {
      return null;
    }
  }
  // Legacy v0.1 form: `number[]` (each element is a byte 0..=255).
  if (!Array.isArray(data)) return null;
  return new Uint8Array(data.slice());
}

/**
 * Decode a `Body::Complete` payload to a UTF-8 string.
 * Returns `null` if the bytes are not valid UTF-8 — callers
 * use the `null` signal to swap in the binary placeholder.
 * Mirrors `RequestInspector.decodeBodyUtf8`.
 */
function decodeBodyUtf8(body: ExchangeBody): string | null {
  const bytes = decodeBodyToBytes(body);
  if (bytes === null) return null;
  if (bytes.length === 0) return "";
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    return null;
  }
}

/**
 * Format a body size (in bytes) as a short human-readable
 * string. Used in the binary placeholder so the user has
 * at least a sense of the payload size.
 */
function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

/** Try to parse the URL's query string into a flat record.
 * Returns an empty object for non-URL inputs (the URL
 * constructor throws). */
function parseQueryString(url: string): Record<string, string> {
  try {
    const u = new URL(url);
    const out: Record<string, string> = {};
    u.searchParams.forEach((v, k) => {
      out[k] = v;
    });
    return out;
  } catch {
    return {};
  }
}

/** Try to parse a string as JSON. Returns `null` on any
 * parse error (the caller swaps in a plain `<pre>`). */
function tryParseJson(s: string): unknown | null {
  if (s.length === 0) return null;
  try {
    return JSON.parse(s);
  } catch {
    return null;
  }
}

/** Parse the `Cookie` request header (a `;`-separated
 * `key=value` list) into `[k, v]` tuples. Whitespace is
 * trimmed; entries without a `=` are dropped. */
function parseCookies(
  headers: Readonly<Record<string, string>>,
): Array<[string, string]> {
  const out: Array<[string, string]> = [];
  for (const [k, v] of Object.entries(headers)) {
    if (k.toLowerCase() !== "cookie") continue;
    for (const part of v.split(";")) {
      const trimmed = part.trim();
      if (trimmed.length === 0) continue;
      const eq = trimmed.indexOf("=");
      if (eq <= 0) continue;
      out.push([
        trimmed.slice(0, eq).trim(),
        trimmed.slice(eq + 1).trim(),
      ]);
    }
  }
  return out;
}

/** Pull the first `content-type` header value (sans
 * `;charset=...`) for the binary placeholder. Mirrors
 * `RequestInspector.getContentType`. */
function getContentType(
  headers: Readonly<Record<string, string>>,
): string | null {
  for (const [k, v] of Object.entries(headers)) {
    if (k.toLowerCase() === "content-type") {
      return v.split(";")[0]?.trim() ?? null;
    }
  }
  return null;
}

interface SectionProps {
  title: string;
  children: React.ReactNode;
  /** Optional `data-testid` for the section container. */
  testId?: string;
}

/** Reusable section header + indented body. */
function Section({ title, children, testId }: SectionProps) {
  return (
    <div data-testid={testId}>
      <h3 className="mb-1 text-xs font-bold uppercase text-slate-400">
        {title}
      </h3>
      <div className="space-y-0.5 pl-2">{children}</div>
    </div>
  );
}

/** Empty state when no row is selected. */
function NoSelection() {
  return (
    <p
      data-testid="inspector-panel-no-selection"
      className="text-sm text-slate-500"
    >
      No exchange selected.
    </p>
  );
}

export function InspectorPanel() {
  const selectedId = useExchangeStore((s) => s.selectedId);
  const activeProjectId = useProjectStore((s) => s.activeProjectId);
  const [detail, setDetail] = useState<ExchangeDetail | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  // Fetch the full detail when the selection (or active
  // project) changes. Cancels the in-flight request when
  // the user clicks a new row before the previous one
  // resolved.
  useEffect(() => {
    if (!selectedId || !activeProjectId) {
      setDetail(null);
      setLoadError(null);
      setLoading(false);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setLoadError(null);
    getExchange(activeProjectId, selectedId)
      .then((d) => {
        if (cancelled) return;
        setDetail(d);
        setLoading(false);
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        setLoadError(String(e));
        setDetail(null);
        setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [selectedId, activeProjectId]);

  if (!selectedId) return <NoSelection />;
  if (loading && !detail) {
    return (
      <p
        data-testid="inspector-panel-loading"
        className="text-sm text-slate-500"
      >
        Loading…
      </p>
    );
  }
  if (loadError) {
    return (
      <div
        data-testid="inspector-panel-error"
        className="rounded border border-red-700 bg-red-900/20 p-3 text-sm text-red-300"
      >
        <strong>Error loading detail:</strong> {loadError}
      </div>
    );
  }
  if (!detail) return <NoSelection />;

  const queryParams = parseQueryString(detail.request.url);
  const queryKeys = Object.keys(queryParams);
  const bodyText = decodeBodyUtf8(detail.request.body);
  const bodyJson = bodyText !== null ? tryParseJson(bodyText) : null;
  const cookies = parseCookies(detail.request.headers);
  const requestHeaderEntries = Object.entries(detail.request.headers);
  const contentType = getContentType(detail.request.headers);
  // The decoded byte length of the body (used by the binary
  // placeholder and the "No body" check). Computed once
  // here so the v0.5 base64 wire form doesn't leak into the
  // UI: callers see bytes, not base64 chars.
  const bodyBytes = decodeBodyToBytes(detail.request.body);
  const bodyByteLen = bodyBytes?.length ?? 0;
  const isBinary =
    detail.request.body.kind === "complete" && bodyText === null;

  return (
    <div data-testid="inspector-panel" className="space-y-3 text-xs">
      <Section title="Query params" testId="inspector-panel-query-params">
        {queryKeys.length === 0 ? (
          <span className="italic text-slate-500">none</span>
        ) : (
          queryKeys.map((k) => (
            <div key={k} className="font-mono">
              <span className="text-accent">{k}</span>
              <span className="text-slate-500"> = </span>
              <span className="text-slate-200">{queryParams[k]}</span>
            </div>
          ))
        )}
      </Section>

      <Section
        title={`Request headers (${requestHeaderEntries.length})`}
        testId="inspector-panel-request-headers"
      >
        {requestHeaderEntries.length === 0 ? (
          <span className="italic text-slate-500">none</span>
        ) : (
          requestHeaderEntries.map(([k, v]) => (
            <div key={k} className="font-mono">
              <span className="text-accent">{k}</span>
              <span className="text-slate-500">: </span>
              <span className="break-all text-slate-300">{v}</span>
            </div>
          ))
        )}
      </Section>

      {cookies.length > 0 && (
        <Section title={`Cookies (${cookies.length})`} testId="inspector-panel-cookies">
          {cookies.map(([k, v]) => (
            <div key={k} className="font-mono">
              <span className="text-accent">{k}</span>
              <span className="text-slate-500"> = </span>
              <span className="text-slate-200">{v}</span>
            </div>
          ))}
        </Section>
      )}

      <Section title="Request body" testId="inspector-panel-request-body">
        {detail.request.body.kind === "empty" && (
          <span className="italic text-slate-500">No body</span>
        )}
        {detail.request.body.kind === "streaming" && (
          <span className="italic text-slate-500">
            Streaming body (length{" "}
            {detail.request.body.content_length ?? "unknown"}); v0.1 does
            not buffer.
          </span>
        )}
        {detail.request.body.kind === "complete" &&
          bodyByteLen === 0 && (
            <span className="italic text-slate-500">No body</span>
          )}
        {isBinary && (
          <span
            data-testid="inspector-panel-binary"
            className="italic text-slate-500"
          >
            [binary: {contentType ?? "application/octet-stream"},{" "}
            {formatSize(bodyByteLen)}] (hex viewer is a v0.5 followup)
          </span>
        )}
        {bodyText !== null && bodyText.length > 0 && bodyJson !== null && (
          <pre
            data-testid="inspector-panel-body-json"
            className="whitespace-pre-wrap break-all font-mono text-slate-200"
          >
            {JSON.stringify(bodyJson, null, 2)}
          </pre>
        )}
        {bodyText !== null && bodyText.length > 0 && bodyJson === null && (
          <pre
            data-testid="inspector-panel-body-text"
            className="whitespace-pre-wrap break-all font-mono text-slate-200"
          >
            {bodyText}
          </pre>
        )}
      </Section>

      {detail.response && (
        <Section
          title={`Response headers (${Object.keys(detail.response.headers).length})`}
          testId="inspector-panel-response-headers"
        >
          {Object.entries(detail.response.headers).map(([k, v]) => (
            <div key={k} className="font-mono">
              <span className="text-accent">{k}</span>
              <span className="text-slate-500">: </span>
              <span className="break-all text-slate-300">{v}</span>
            </div>
          ))}
        </Section>
      )}

      {detail.blocked_reason && (
        <div
          data-testid="inspector-panel-blocked"
          className="rounded border border-scope-blocked bg-red-900/20 p-3 text-sm text-red-300"
        >
          <strong>Blocked:</strong> {detail.blocked_reason}
        </div>
      )}
    </div>
  );
}
