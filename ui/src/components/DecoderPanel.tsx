// §4.7 DecoderPanel. The "Decoder" tab in the right-rail.
// A small CyberChef-lite: a textarea input, four
// single-op buttons (base64 / url / html / hex), and a
// "Smart" button that recursively applies base64 → url →
// html up to 8 layers.
//
// Spec (§4.7):
//   - The `applyOp` function is the single-op transform
//     (one input string → one output string). It returns
//     the literal `"[decode error]"` on failure so the
//     smart-decode loop can recognize that the candidate
//     did NOT change.
//   - The `smartDecode` loop walks the layer chain,
//     stopping when no op makes a difference or the
//     8-layer cap is hit. The 8-layer cap prevents
//     infinite loops on self-decoding inputs (e.g. a
//     string that decodes to itself under URL decoding).
//   - Security: the panel runs only `atob`,
//     `decodeURIComponent`, the HTML entity regex, and
//     `parseInt(..., 16)` (for hex). It does NOT use
//     `eval` or the `Function` constructor. The output
//     is rendered inside a `<pre>` (React escapes by
//     default).

import { useState } from "react";

/** Single-op kind. */
type Op = "base64" | "url" | "html" | "hex";

/** All four ops in display order for the `<select>`. */
const OPS: readonly Op[] = ["base64", "url", "html", "hex"];

/** Cap on smart-decode recursion. Mirrors the master
 * plan's 8-layer choice (enough for triple-nested
 * base64-URL-HTML chains). */
const SMART_MAX_LAYERS = 8;

/** Sentinel returned by `applyOp` on a failed decode.
 * The smart-decode loop uses it to detect "no change"
 * (a layer that errored is treated as no-change so the
 * loop moves on to the next op). */
const DECODE_ERROR = "[decode error]";

/**
 * Apply a single decode op to an input string.
 *
 * Returns `DECODE_ERROR` (`"[decode error]"`) on failure.
 * For `base64` we strip whitespace before decoding so
 * pretty-printed base64 (e.g. JWT segments with `\n`) works
 * — `atob` rejects whitespace. We also enforce a strict
 * base64 alphabet (no special chars), so a non-base64
 * string like "hello world" does NOT silently decode to
 * garbage — it returns DECODE_ERROR. This prevents the
 * smart-decode loop from getting stuck in infinite
 * "changes" on plain-text inputs.
 * For `hex` we ignore the last odd byte (a partial byte
 * is silently dropped; v0.5 will surface a warning).
 */
function applyOp(input: string, op: Op): string {
  try {
    switch (op) {
      case "base64": {
        // `atob` rejects whitespace; strip it first. The
        // strict-alphabet check ensures plain text (e.g.
        // "hello world") returns DECODE_ERROR rather than
        // silently decoding to a prefix's bytes.
        const stripped = input.replace(/\s+/g, "");
        if (!/^[A-Za-z0-9+/]*={0,2}$/.test(stripped)) {
          return DECODE_ERROR;
        }
        if (stripped.length === 0) return DECODE_ERROR;
        return atob(stripped);
      }
      case "url":
        return decodeURIComponent(input);
      case "html":
        return input
          .replace(/&amp;/g, "&")
          .replace(/&lt;/g, "<")
          .replace(/&gt;/g, ">")
          .replace(/&quot;/g, '"')
          .replace(/&#39;/g, "'");
      case "hex":
        return input.replace(/../g, (h) =>
          String.fromCharCode(parseInt(h, 16)),
        );
    }
  } catch {
    return DECODE_ERROR;
  }
}

/**
 * Result of the smart-decode walk. `layers` is the chain
 * of ops that fired (in order); `output` is the final
 * string. If no op changed the input, `layers` is empty
 * and `output` equals the input.
 */
interface SmartResult {
  layers: Op[];
  output: string;
}

/**
 * Recursively try base64 → url → html up to
 * `SMART_MAX_LAYERS` times, stopping when no op makes a
 * difference. The op order is fixed (base64 first because
 * it's the most common wrapper; url next; html last
 * because it's the most lossy).
 */
function smartDecode(input: string): SmartResult {
  const layers: Op[] = [];
  const opOrder: readonly Op[] = ["base64", "url", "html"];
  let current = input;
  let safety = 0;
  let changed = true;
  while (changed && safety < SMART_MAX_LAYERS) {
    changed = false;
    safety += 1;
    for (const op of opOrder) {
      const candidate = applyOp(current, op);
      if (candidate !== DECODE_ERROR && candidate !== current) {
        layers.push(op);
        current = candidate;
        changed = true;
        break;
      }
    }
  }
  return { layers, output: current };
}

/** Subset of `SmartResult` (just the layer chain) for
 * the "Smart" button label. */
function formatLayers(layers: Op[]): string {
  if (layers.length === 0) return "no change";
  return layers.join(" → ");
}

export function DecoderPanel() {
  const [input, setInput] = useState("");
  const [op, setOp] = useState<Op>("base64");
  const [output, setOutput] = useState<string | null>(null);
  const [smart, setSmart] = useState<SmartResult | null>(null);

  /** Run a single-op decode. */
  const run = () => {
    setSmart(null);
    setOutput(applyOp(input, op));
  };

  /** Run the recursive smart-decode. */
  const runSmart = () => {
    setOutput(null);
    setSmart(smartDecode(input));
  };

  return (
    <div data-testid="decoder-panel" className="space-y-3 text-xs">
      <textarea
        data-testid="decoder-panel-input"
        value={input}
        onChange={(e) => {
          setInput(e.target.value);
        }}
        placeholder="Paste base64 / URL-encoded / HTML-encoded / hex text…"
        className="h-24 w-full resize-y rounded border border-slate-600 bg-bg-base px-2 py-1 font-mono text-slate-100 focus:border-accent focus:outline-none"
      />
      <div className="flex items-center gap-2">
        <select
          data-testid="decoder-panel-op"
          value={op}
          onChange={(e) => {
            setOp(e.target.value as Op);
          }}
          className="rounded border border-slate-600 bg-bg-base px-2 py-1 text-slate-100 focus:border-accent focus:outline-none"
        >
          {OPS.map((o) => (
            <option key={o} value={o}>
              {labelForOp(o)}
            </option>
          ))}
        </select>
        <button
          type="button"
          data-testid="decoder-panel-decode"
          onClick={run}
          className="rounded bg-accent px-3 py-1 font-medium text-bg-base hover:bg-cyan-300"
        >
          Decode
        </button>
        <button
          type="button"
          data-testid="decoder-panel-smart"
          onClick={runSmart}
          className="rounded border border-accent px-3 py-1 font-medium text-accent hover:bg-accent hover:text-bg-base"
        >
          Smart
        </button>
      </div>

      {output !== null && (
        <div data-testid="decoder-panel-output">
          <h3 className="mb-1 text-xs font-bold uppercase text-slate-400">
            Output ({labelForOp(op)})
          </h3>
          <pre className="whitespace-pre-wrap break-all rounded border border-slate-700 bg-bg-base p-2 font-mono text-slate-200">
            {output}
          </pre>
        </div>
      )}

      {smart !== null && (
        <div data-testid="decoder-panel-smart-output">
          <h3 className="mb-1 text-xs font-bold uppercase text-slate-400">
            Smart decode ({smart.layers.length} layer
            {smart.layers.length === 1 ? "" : "s"}:{" "}
            <span data-testid="decoder-panel-smart-layers">
              {formatLayers(smart.layers)}
            </span>
            )
          </h3>
          <pre
            data-testid="decoder-panel-smart-result"
            className="whitespace-pre-wrap break-all rounded border border-slate-700 bg-bg-base p-2 font-mono text-slate-200"
          >
            {smart.output}
          </pre>
        </div>
      )}
    </div>
  );
}

/** Human-friendly label for an op (used in the `<select>`
 * and the output headers). */
function labelForOp(op: Op): string {
  switch (op) {
    case "base64":
      return "Base64";
    case "url":
      return "URL";
    case "html":
      return "HTML entities";
    case "hex":
      return "Hex";
  }
}
