// Response inspector pane. Mirrors `RequestInspector` with a
// status code prefix and the same three sub-tabs.
//
// Spec (§4.6):
//   - `pretty` (default): the status code (color-coded by
//     class), expandable header summary, and the decoded
//     body in a `<pre>`. Binary bodies flagged with
//     `[binary: <mime>, <size>]` text (full hex viewer is
//     v0.5).
//   - `headers`: a flat key-value table.
//   - `raw`: the unparsed HTTP wire format (status line +
//     headers + body, joined by `\r\n`).
//
// Status color classes match the §4.6 convention:
//   - 2xx → green
//   - 3xx → yellow
//   - 4xx → orange
//   - 5xx → red
//   - anything else (1xx, custom) → red (fail-safe)

import { useState } from "react";
import type { ExchangeBody, ExchangeResponse } from "../types/domain";

type View = "pretty" | "headers" | "raw";

const SUB_TABS: readonly View[] = ["pretty", "headers", "raw"];

interface Props {
  response: ExchangeResponse;
}

/**
 * Decode a `Body::Complete` payload (a base64 string on the
 * wire as of v0.5; the v0.1 form was a JSON byte array) to a
 * `Uint8Array`. Mirrors the helper in
 * `ui/src/lib/body-decode.ts` (duplicated here for the same
 * reason `decodeBodyUtf8` is — see the `RequestInspector`
 * copy for the rationale).
 */
function decodeBodyToBytes(body: ExchangeBody): Uint8Array | null {
  if (body.kind !== "complete") return null;
  const data = body.data;
  if (typeof data === "string") {
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
  if (!Array.isArray(data)) return null;
  // Coerce `readonly number[]` to a plain `number[]` (a copy).
  // `Uint8Array` accepts `ArrayLike<number>`, but the readonly
  // modifier trips the type checker; the slice() produces a
  // mutable copy that satisfies both.
  return new Uint8Array(data.slice());
}

/**
 * Decode a `Body::Complete` payload to a UTF-8 string.
 * Returns `null` if the bytes are not valid UTF-8 — callers
 * use the `null` signal to swap in the binary placeholder.
 */
function decodeBodyUtf8(body: ExchangeBody): string | null {
  const bytes = decodeBodyToBytes(body);
  if (bytes === null) return null;
  if (bytes.length === 0) return "";
  try {
    const text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
    return text;
  } catch {
    return null;
  }
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

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

/**
 * Map an HTTP status code to a Tailwind text color class.
 * The status color is a UI affordance only — the 5xx
 * fail-safe is the "anything weird" case (1xx, custom
 * codes).
 */
function statusColor(status: number): string {
  if (status < 300) return "text-green-400";
  if (status < 400) return "text-yellow-400";
  if (status < 500) return "text-orange-400";
  return "text-red-400";
}

export function ResponseInspector({ response }: Props) {
  const [view, setView] = useState<View>("pretty");
  const text = decodeBodyUtf8(response.body);
  const isBinary = response.body.kind === "complete" && text === null;
  const contentType = getContentType(response.headers);
  const headerEntries = Object.entries(response.headers);
  const colorClass = statusColor(response.status);
  // The decoded byte length of the body. Computed once
  // here so the v0.5 base64 wire form doesn't leak into
  // the UI: callers see bytes, not base64 chars.
  const bodyBytes = decodeBodyToBytes(response.body);
  const bodyByteLen = bodyBytes?.length ?? 0;

  return (
    <div
      data-testid="response-inspector"
      className="rounded border border-slate-700"
    >
      <div className="flex border-b border-slate-700">
        <span className="bg-bg-panel px-3 py-1 text-xs font-bold uppercase text-slate-300">
          Response
        </span>
        <span
          data-testid="response-inspector-status"
          className={`px-3 py-1 font-mono text-sm font-bold ${colorClass}`}
        >
          {response.status} {response.status_text}
        </span>
        <div className="ml-auto flex">
          {SUB_TABS.map((v) => (
            <button
              key={v}
              type="button"
              data-testid={`response-inspector-tab-${v}`}
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
          <div
            className="space-y-0.5"
            data-testid="response-inspector-headers"
          >
            {headerEntries.length === 0 ? (
              <div className="italic text-slate-500">No headers</div>
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
            data-testid="response-inspector-raw"
            className="overflow-x-auto whitespace-pre-wrap break-all text-slate-200"
          >
            {`${response.version} ${response.status} ${response.status_text}\r\n`}
            {headerEntries.map(([k, v]) => `${k}: ${v}`).join("\r\n")}
            {response.body.kind === "complete" && bodyByteLen > 0
              ? `\r\n\r\n${text ?? `[binary: ${contentType ?? "application/octet-stream"}, ${formatSize(bodyByteLen)}]`}`
              : ""}
          </pre>
        )}

        {view === "pretty" && (
          <div data-testid="response-inspector-pretty">
            <div className="mb-2">
              <span className={colorClass}>{response.status}</span>{" "}
              {response.status_text}
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
            {response.body.kind === "empty" && (
              <div className="mt-2 italic text-slate-500">No body</div>
            )}
            {response.body.kind === "streaming" && (
              <div className="mt-2 italic text-slate-500">
                Streaming body (length{" "}
                {response.body.content_length ?? "unknown"}); v0.1 does
                not buffer.
              </div>
            )}
            {response.body.kind === "complete" &&
              bodyByteLen === 0 && (
                <div className="mt-2 italic text-slate-500">No body</div>
              )}
            {response.body.kind === "complete" &&
              bodyByteLen > 0 &&
              text !== null && (
                <pre className="mt-2 whitespace-pre-wrap break-all text-slate-200">
                  {text}
                </pre>
              )}
            {isBinary && (
              <div
                data-testid="response-inspector-binary"
                className="mt-2 italic text-slate-500"
              >
                [binary: {contentType ?? "application/octet-stream"},{" "}
                {formatSize(bodyByteLen)}] (hex viewer is a v0.5 followup)
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
