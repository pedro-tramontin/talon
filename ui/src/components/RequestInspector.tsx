// Request inspector pane. Shows the captured request with
// three sub-tabs: pretty / headers / raw.
//
// Spec (§4.6):
//   - `pretty` (default): method + URL on the first line, an
//     expandable header summary, and the decoded body in a
//     `<pre>`. Bodies that are not valid UTF-8 (e.g. a
//     `multipart/form-data` upload with raw bytes) are
//     flagged as `[binary: <mime>, <size>]` text — the full
//     hex viewer is a v0.5 followup (per the master plan).
//   - `headers`: a flat key-value table.
//   - `raw`: the unparsed HTTP wire format (start line +
//     headers + body, joined by `\r\n`).
//
// Body decoding notes (cross-language wire shape):
//   The Rust side serializes `Body::Complete { data: Bytes }`
//   as a JSON array of byte values (e.g. `[104, 101, ...]`
//   for "he..."). The `decodeBody` helper reconstructs the
//   `Uint8Array` and runs it through `TextDecoder` (UTF-8).
//   We deliberately do NOT use `atob` — that would treat the
//   array as a base64 string and fail (the v0.1 plan's
//   "exact code" was wrong on this point; the §4.6 DTO fix
//   in `ui/src/types/domain.ts` is the matching change on
//   the type side).
//
// Security: the body is rendered inside a `<pre>` element,
// not interpreted as HTML. React's default escaping applies;
// no `dangerouslySetInnerHTML` is used. Headers are shown
// verbatim (also inside `<pre>` / `<div>` text nodes).

import { useState } from "react";
import type { ExchangeBody, ExchangeRequest } from "../types/domain";

/** Sub-tab identifier. */
type View = "pretty" | "headers" | "raw";

/** Sub-tabs in display order. The literal `"binary"` flag
 * is a v0.5 followup marker — v1 only has the three panes
 * below. */
const SUB_TABS: readonly View[] = ["pretty", "headers", "raw"];

interface Props {
  /** The captured request to render. */
  request: ExchangeRequest;
}

/**
 * Decode a `Body::Complete` payload (a JSON byte array on
 * the wire) to a UTF-8 string. Returns `null` if the bytes
 * are not valid UTF-8 — callers use the `null` signal to
 * swap in the binary placeholder.
 */
function decodeBodyUtf8(body: ExchangeBody): string | null {
  if (body.kind !== "complete") return null;
  if (body.data.length === 0) return "";
  const bytes = new Uint8Array(body.data);
  try {
    const text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
    return text;
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

/**
 * Try to derive a content-type from the request headers.
 * Returns `null` if there is no `content-type` header. The
 * binary placeholder uses this to label the payload.
 */
function getContentType(
  headers: Readonly<Record<string, string>>,
): string | null {
  for (const [k, v] of Object.entries(headers)) {
    if (k.toLowerCase() === "content-type") {
      // The Rust serde emits the first value only (single
      // string per header name); strip any `; charset=...`.
      return v.split(";")[0]?.trim() ?? null;
    }
  }
  return null;
}

export function RequestInspector({ request }: Props) {
  const [view, setView] = useState<View>("pretty");
  const text = decodeBodyUtf8(request.body);
  const isBinary = request.body.kind === "complete" && text === null;
  const contentType = getContentType(request.headers);
  const headerEntries = Object.entries(request.headers);

  return (
    <div
      data-testid="request-inspector"
      className="rounded border border-slate-700"
    >
      <div className="flex border-b border-slate-700">
        <span className="bg-bg-panel px-3 py-1 text-xs font-bold uppercase text-slate-300">
          Request
        </span>
        <div className="ml-auto flex">
          {SUB_TABS.map((v) => (
            <button
              key={v}
              type="button"
              data-testid={`request-inspector-tab-${v}`}
              onClick={() => {
                setView(v);
              }}
              className={`px-3 py-1 text-xs ${
                view === v
                  ? "text-accent"
                  : "text-slate-400 hover:text-slate-200"
              }`}
            >
              {v}
            </button>
          ))}
        </div>
      </div>

      <div className="p-3 font-mono text-sm">
        {view === "headers" && (
          <div className="space-y-0.5" data-testid="request-inspector-headers">
            {headerEntries.length === 0 ? (
              <div className="text-slate-500 italic">No headers</div>
            ) : (
              headerEntries.map(([k, v]) => (
                <div key={k}>
                  <span className="text-accent">{k}</span>
                  <span className="text-slate-500">: </span>
                  <span className="text-slate-200">{v}</span>
                </div>
              ))
            )}
          </div>
        )}

        {view === "raw" && (
          <pre
            data-testid="request-inspector-raw"
            className="overflow-x-auto whitespace-pre-wrap break-all text-slate-200"
          >
            {`${request.method} ${request.url} ${request.version}\r\n`}
            {headerEntries.map(([k, v]) => `${k}: ${v}`).join("\r\n")}
            {request.body.kind === "complete" && request.body.data.length > 0
              ? `\r\n\r\n${text ?? `[binary: ${contentType ?? "application/octet-stream"}, ${formatSize(request.body.data.length)}]`}`
              : ""}
          </pre>
        )}

        {view === "pretty" && (
          <div data-testid="request-inspector-pretty">
            <div className="mb-2 text-slate-200">
              <span className="font-bold">{request.method}</span>{" "}
              <span className="text-slate-500">{request.url}</span>
            </div>
            {headerEntries.length > 0 && (
              <details>
                <summary className="cursor-pointer text-xs text-slate-500">
                  {headerEntries.length} header
                  {headerEntries.length === 1 ? "" : "s"}
                </summary>
                <div className="mt-1 space-y-0.5 pl-3">
                  {headerEntries.map(([k, v]) => (
                    <div key={k}>
                      <span className="text-accent">{k}</span>
                      <span className="text-slate-500">: </span>
                      <span className="text-slate-300">{v}</span>
                    </div>
                  ))}
                </div>
              </details>
            )}
            {request.body.kind === "empty" && (
              <div className="mt-2 italic text-slate-500">No body</div>
            )}
            {request.body.kind === "streaming" && (
              <div className="mt-2 italic text-slate-500">
                Streaming body (length{" "}
                {request.body.content_length ?? "unknown"}); v0.1 does
                not buffer.
              </div>
            )}
            {request.body.kind === "complete" &&
              request.body.data.length === 0 && (
                <div className="mt-2 italic text-slate-500">No body</div>
              )}
            {request.body.kind === "complete" &&
              request.body.data.length > 0 &&
              text !== null && (
                <pre className="mt-2 whitespace-pre-wrap break-all text-slate-200">
                  {text}
                </pre>
              )}
            {isBinary && (
              <div
                data-testid="request-inspector-binary"
                className="mt-2 italic text-slate-500"
              >
                [binary: {contentType ?? "application/octet-stream"},{" "}
                {formatSize(request.body.data.length)}] (hex viewer is
                a v0.5 followup)
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
