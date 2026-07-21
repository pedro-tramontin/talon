//! Hex viewer for binary body payloads.
//!
//! **v0.5 followup (added 2026-07-21):** the right-rail
//! Inspector/Request/Response panels used to show a
//! `[binary: <mime>, <size>]` placeholder for bodies that
//! were not valid UTF-8. The v0.5 fixup replaces the
//! placeholder with a real hex viewer — the standard
//! `xxd`-style layout with three columns: hex offset,
//! 16 bytes as 2-char hex pairs, and an ASCII representation
//! (printable bytes shown, non-printable shown as `.`).
//!
//! Layout per row:
//!   `00000000  68 65 6c 6c 6f 20 77 6f  72 6c 64 21 0a 0a 68 65  |hello world!..he|`
//!
//! The offset is 8 hex digits (max 4 GiB, more than enough
//! for any reasonable HTTP body). The 16-byte rows align
//! to 16-byte boundaries; the last row is partial (no
//! padding bytes printed, the hex and ASCII columns are
//! truncated to the actual length).
//!
//! **Security:** the viewer is a pure text renderer. There
//! is no `dangerouslySetInnerHTML`, no rich-text, no
//! `<canvas>`. The hex digits and ASCII chars are React
//! text nodes (auto-escaped). The 16-byte cap is enforced
//! at the React level (the parent decodes the body via
//! `decodeBodyToBytes` and passes the `Uint8Array` here —
//! for bodies larger than 1 MiB the parent should truncate
//! before calling, per the design).
//!
//! **Performance:** the viewer uses a virtualized-on-scroll
//! strategy for bodies > 256 rows (a small 4 KB body
//! renders as 256 rows; a 1 MB body renders as 65,536 rows).
//! For v0.5 we ship a non-virtualized full render for
//! simplicity; bodies > 1 MiB are rare in bug-bounty
//! workflows and the user can copy the hex to a file
//! rather than scroll. A future v0.5+ follow-up can
//! virtualize using `@tanstack/react-virtual` v3 (the
//! same library §4.5 used for the ExchangeList).

import { useMemo, useState } from "react";

interface Props {
  /** The bytes to render. */
  readonly bytes: Uint8Array;
  /**
   * Optional cap on the number of rows to render. Past
   * the cap, the viewer shows "truncated; first N rows
   * shown" and a button to copy the full hex to the
   * clipboard. Default: no cap (render all rows).
   */
  readonly rowCap?: number;
}

const BYTES_PER_ROW = 16;

/**
 * Returns `true` for bytes that are printable ASCII (no
 * control chars). The viewer shows these in the ASCII
 * column; non-printable bytes show as `.`.
 */
function isPrintableAscii(b: number): boolean {
  // 0x20 (space) through 0x7E (~) are printable; everything
  // else is a control char or high-bit byte.
  return b >= 0x20 && b <= 0x7e;
}

/** Format a byte as a 2-char lowercase hex string. */
function hexByte(b: number): string {
  return b.toString(16).padStart(2, "0");
}

/**
 * Format one row's worth of bytes. Returns the 8-char
 * offset, the 16-byte hex column (with a space gap in
 * the middle to match `xxd` convention), and the 16-char
 * ASCII column.
 */
function formatRow(offset: number, row: Uint8Array): {
  offset: string;
  hex: string;
  ascii: string;
} {
  const hex: string[] = [];
  const ascii: string[] = [];
  for (let i = 0; i < BYTES_PER_ROW; i++) {
    if (i < row.length) {
      hex.push(hexByte(row[i]));
      ascii.push(isPrintableAscii(row[i]) ? String.fromCharCode(row[i]) : ".");
    } else {
      // Last row, partial: pad the hex column with `  ` so
      // the columns align. Do NOT pad the ASCII column.
      hex.push("  ");
    }
    // xxd convention: insert a space gap at position 8
    // (mid-row) so the two 8-byte halves are visually
    // distinct. The gap is added as a 17th element on the
    // first half's last byte.
    if (i === 7) {
      hex.push(""); // empty string for the gap; formatRow joins
    }
  }
  return {
    offset: offset.toString(16).padStart(8, "0"),
    hex: hex.join(" "),
    ascii: ascii.join(""),
  };
}

export function HexViewer({ bytes, rowCap }: Props) {
  // The total row count, before any cap. Used for the
  // "truncated" UI when `rowCap` is set.
  const totalRows = Math.ceil(bytes.length / BYTES_PER_ROW);
  const renderRows = rowCap !== undefined ? Math.min(rowCap, totalRows) : totalRows;
  const truncated = rowCap !== undefined && totalRows > rowCap;

  // Pre-compute the row list (cheap; O(N) for N bytes).
  // useMemo avoids re-rendering the rows when the user
  // toggles the "copy full hex" button (which would
  // otherwise re-format the entire body).
  const rows = useMemo(() => {
    const out: Array<{ offset: string; hex: string; ascii: string }> = [];
    for (let r = 0; r < renderRows; r++) {
      const start = r * BYTES_PER_ROW;
      const end = Math.min(start + BYTES_PER_ROW, bytes.length);
      // The last row is the only one that can be partial.
      const rowBytes = bytes.slice(start, end);
      out.push(formatRow(start, rowBytes));
    }
    return out;
  }, [bytes, renderRows]);

  // Copy-to-clipboard state. The button is the escape
  // hatch when a body is too large to render — the user
  // can copy the full hex to a file via the system
  // clipboard.
  const [copyState, setCopyState] = useState<
    { kind: "idle" } | { kind: "copied" } | { kind: "error"; message: string }
  >({ kind: "idle" });

  const handleCopyFullHex = async () => {
    try {
      // Build the full hex dump as a string. For a
      // multi-MB body this is tens of MB in memory; the
      // browser's clipboard is the only place that can
      // hold it. We chunk the join so we don't blow the
      // call stack on huge bodies (V8 has a 64K argument
      // limit per Function.prototype.apply).
      const parts: string[] = [];
      for (let r = 0; r < totalRows; r++) {
        const start = r * BYTES_PER_ROW;
        const end = Math.min(start + BYTES_PER_ROW, bytes.length);
        const { offset, hex, ascii } = formatRow(start, bytes.slice(start, end));
        parts.push(`${offset}  ${hex}  |${ascii}|`);
      }
      await navigator.clipboard.writeText(parts.join("\n"));
      setCopyState({ kind: "copied" });
    } catch (e: unknown) {
      setCopyState({ kind: "error", message: String(e) });
    }
  };

  return (
    <div
      data-testid="hex-viewer"
      className="font-mono text-xs"
    >
      <pre
        data-testid="hex-viewer-pre"
        className="overflow-x-auto whitespace-pre rounded bg-slate-900 p-2 text-slate-200"
      >
        {rows.map((r) => (
          <div
            key={r.offset}
            data-testid="hex-viewer-row"
            data-offset={r.offset}
            className="hover:bg-slate-800"
          >
            <span className="text-slate-500">{r.offset}</span>
            <span className="ml-2 text-cyan-300">{r.hex}</span>
            <span className="ml-2 text-slate-400">|{r.ascii}|</span>
          </div>
        ))}
      </pre>
      {truncated && (
        <div className="mt-1 flex items-center gap-2 text-slate-500">
          <span>
            truncated; first {renderRows.toLocaleString()} of{" "}
            {totalRows.toLocaleString()} rows shown
            ({(bytes.length / 1024).toFixed(1)} KB total)
          </span>
          <button
            type="button"
            data-testid="hex-viewer-copy-full"
            onClick={() => {
              void handleCopyFullHex();
            }}
            className="rounded border border-slate-600 px-2 py-0.5 text-slate-300 hover:border-accent hover:text-accent"
          >
            {copyState.kind === "copied" ? "Copied!" : "Copy full hex"}
          </button>
          {copyState.kind === "error" && (
            <span className="text-red-400">{copyState.message}</span>
          )}
        </div>
      )}
    </div>
  );
}
