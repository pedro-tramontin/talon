// §v0.5 HexViewer. Tests the row format (offset, hex,
// ASCII) and the size cap behavior.

import { render, screen, fireEvent } from "@testing-library/react";
import { HexViewer } from "../lib/hex-viewer";

describe("HexViewer", () => {
  it("renders the standard xxd-style row format", () => {
    // 'hello world!\n\nhe' = the 15 bytes after the "00000000" + "  " + 16-byte row.
    const bytes = new TextEncoder().encode("hello world!\n\nhe");
    const { container } = render(<HexViewer bytes={bytes} />);
    // The 8-char offset column.
    expect(
      container.querySelector('[data-testid="hex-viewer-row"][data-offset="00000000"]'),
    ).toBeTruthy();
    // The hex column (the 'h' byte is 0x68, 'e' is 0x65, etc.).
    // We assert the full hex substring for the first 4 bytes
    // to confirm the format is `68 65 6c 6c` (with spaces).
    const firstRow = container.querySelector(
      '[data-testid="hex-viewer-row"][data-offset="00000000"]',
    );
    expect(firstRow?.textContent).toContain("68 65 6c 6c");
    // The ASCII column shows printable bytes verbatim.
    expect(firstRow?.textContent).toContain("|hello world!..he|");
  });

  it("renders multiple rows with sequential offsets", () => {
    // 32 bytes: 2 full 16-byte rows.
    const bytes = new Uint8Array(32);
    for (let i = 0; i < 32; i++) bytes[i] = i;
    const { container } = render(<HexViewer bytes={bytes} />);
    const rows = container.querySelectorAll('[data-testid="hex-viewer-row"]');
    expect(rows.length).toBe(2);
    expect(rows[0]?.getAttribute("data-offset")).toBe("00000000");
    expect(rows[1]?.getAttribute("data-offset")).toBe("00000010");
  });

  it("shows non-printable bytes as '.' in the ASCII column", () => {
    // First byte 0x00 (NUL, non-printable), then "abc" (0x61 0x62 0x63).
    const bytes = new Uint8Array([0x00, 0x61, 0x62, 0x63]);
    const { container } = render(<HexViewer bytes={bytes} />);
    const firstRow = container.querySelector(
      '[data-testid="hex-viewer-row"][data-offset="00000000"]',
    );
    expect(firstRow?.textContent).toContain("00 61 62 63");
    expect(firstRow?.textContent).toContain("|.abc|");
  });

  it("truncates the last partial row correctly (no padding in the ASCII column)", () => {
    // 19 bytes: row 0 is 16 full bytes, row 1 is 3 partial bytes.
    // The last (partial) row's hex column should have 3 hex
    // pairs + padding '  ' for the missing 13 bytes.
    // The ASCII column should have exactly 3 chars, no padding.
    const bytes = new Uint8Array(19);
    for (let i = 0; i < 19; i++) bytes[i] = 0x41; // all 'A'
    const { container } = render(<HexViewer bytes={bytes} />);
    const lastRow = container.querySelector(
      '[data-testid="hex-viewer-row"][data-offset="00000010"]',
    );
    expect(lastRow).toBeTruthy();
    // The ASCII column for a partial row is just 3 chars.
    const asciiMatch = lastRow?.textContent?.match(/\|([^|]*)\|/);
    expect(asciiMatch?.[1].length).toBe(3);
    expect(asciiMatch?.[1]).toBe("AAA");
  });

  it("respects the rowCap and shows the truncated message + copy button", () => {
    // 100 KiB of bytes: ceil(100*1024 / 16) = 6400 rows.
    const bytes = new Uint8Array(100 * 1024);
    const { container } = render(<HexViewer bytes={bytes} rowCap={100} />);
    const rows = container.querySelectorAll('[data-testid="hex-viewer-row"]');
    expect(rows.length).toBe(100);
    // The truncated message + the copy button.
    expect(container.textContent).toContain("truncated");
    expect(container.textContent).toContain("100.0 KB total");
    const copyButton = container.querySelector(
      '[data-testid="hex-viewer-copy-full"]',
    );
    expect(copyButton).toBeTruthy();
  });

  it("does NOT show the truncated message when rowCap is not set", () => {
    const bytes = new Uint8Array(100);
    const { container } = render(<HexViewer bytes={bytes} />);
    expect(container.textContent).not.toContain("truncated");
  });

  it("handles an empty body (0 bytes) without crashing", () => {
    const bytes = new Uint8Array(0);
    const { container } = render(<HexViewer bytes={bytes} />);
    const rows = container.querySelectorAll('[data-testid="hex-viewer-row"]');
    expect(rows.length).toBe(0);
    // The hex viewer is present in the DOM but empty.
    expect(container.querySelector('[data-testid="hex-viewer"]')).toBeTruthy();
  });

  it("the copy button click triggers navigator.clipboard.writeText", () => {
    // Mock the clipboard API for this test. jsdom doesn't
    // provide navigator.clipboard by default; we stub it.
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText },
      configurable: true,
      writable: true,
    });
    // Use enough bytes that totalRows > rowCap (1) so the
    // truncated UI + copy button render. 200 bytes = 13
    // rows, rowCap=1, so 12 rows are truncated.
    const bytes = new Uint8Array(200);
    for (let i = 0; i < bytes.length; i++) bytes[i] = i;
    const { container } = render(<HexViewer bytes={bytes} rowCap={1} />);
    const copyButton = container.querySelector(
      '[data-testid="hex-viewer-copy-full"]',
    ) as HTMLButtonElement;
    expect(copyButton).toBeTruthy();
    fireEvent.click(copyButton);
    // The writeText is called with the full hex dump (not
    // just the first row, despite the rowCap). The promise
    // resolves on the next microtask.
    expect(writeText).toHaveBeenCalledTimes(1);
    // The argument is a string with the offset + hex + ascii
    // for the first row.
    const callArg = writeText.mock.calls[0]?.[0] as string;
    expect(callArg).toContain("00000000");
    expect(callArg).toContain("00 01 02 03");
    // The full dump should have ALL the rows (13 of them),
    // not just the truncated 1. The first 0x00 byte shows
    // up as 0x00 in the dump at the last row's first byte.
    const lines = callArg.split("\n");
    expect(lines.length).toBe(13);
    expect(lines[12]).toContain("000000c0"); // 12 * 16 = 192 = 0xc0
  });
});
