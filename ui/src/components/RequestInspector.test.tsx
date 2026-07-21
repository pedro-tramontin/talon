//! Unit tests for the `RequestInspector` component.
//!
//! Lockstep contract (see `ConfirmDialogLockstep.test.tsx`
//! for the analog on the destructive-tool side): the
//! `RequestInspector` is the canonical place where binary
//! bodies get the `HexViewer` rendered. The `HexViewer` is
//! covered by its own test suite
//! (`ui/src/lib/hex-viewer.test.tsx`) for format/copy/
//! truncation behavior. **This file** pins the
//! `RequestInspector` integration so a refactor that drops
//! the `request-inspector-binary` testid or breaks the
//! `complete` + non-UTF-8 → `HexViewer` contract fails here,
//! not just in the `HexViewer`'s own suite.
//!
//! Pins (don't break these without a corresponding fix):
//! - `request-inspector-binary` testid is present when the
//!   body is a `Complete` whose bytes are not valid UTF-8.
//! - `request-inspector-binary` testid is NOT present when
//!   the body is a `Complete` whose bytes are valid UTF-8
//!   (the pretty view shows the `<pre>` text instead).
//! - `request-inspector-binary` testid is NOT present when
//!   the body is `Empty` or `Streaming`.
//! - The first row of the hex view renders the bytes in
//!   xxd-style (`ff fe` for `[0xff, 0xfe, ...]`).

import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { ExchangeRequest } from "../types/domain";
import { RequestInspector } from "./RequestInspector";

/** Build a minimal `ExchangeRequest` for tests. The fields
 * we care about (`body`, `headers`) are overrides. */
function makeRequest(overrides?: Partial<ExchangeRequest>): ExchangeRequest {
  return {
    method: "POST",
    url: "https://example.com/upload",
    version: "HTTP/1.1",
    headers: { "content-type": "application/octet-stream" },
    body: { kind: "empty" },
    ...overrides,
  };
}

describe("RequestInspector", () => {
  it("renders the HexViewer (binary branch) when the body is not valid UTF-8", () => {
    // 0xFF 0xFE 0x00 0x01 0x02 are invalid UTF-8 sequences.
    const request = makeRequest({
      body: {
        kind: "complete",
        data: [0xff, 0xfe, 0x00, 0x01, 0x02],
      },
    });
    render(<RequestInspector request={request} />);
    const hexContainer = screen.getByTestId("request-inspector-binary");
    expect(hexContainer).toBeInTheDocument();
    // The hex viewer's testid is present (the binary branch
    // was taken), and the row format is the standard
    // xxd-style (the first 2 bytes are `ff fe`).
    expect(hexContainer.textContent).toContain("ff fe");
  });

  it("does NOT render the binary branch when the body is valid UTF-8", () => {
    // "héllo" is valid UTF-8.
    const request = makeRequest({
      body: {
        kind: "complete",
        // Legacy v0.1 form: JSON byte array.
        data: [104, 195, 169, 108, 108, 111],
      },
    });
    render(<RequestInspector request={request} />);
    // The pretty view is the default — the body text is in
    // a `<pre>`, not the hex viewer.
    expect(
      screen.queryByTestId("request-inspector-binary"),
    ).not.toBeInTheDocument();
  });

  it("does NOT render the binary branch when the body is empty", () => {
    const request = makeRequest({ body: { kind: "empty" } });
    render(<RequestInspector request={request} />);
    expect(
      screen.queryByTestId("request-inspector-binary"),
    ).not.toBeInTheDocument();
  });

  it("does NOT render the binary branch when the body is streaming", () => {
    const request = makeRequest({
      body: { kind: "streaming", content_length: 1024 },
    });
    render(<RequestInspector request={request} />);
    expect(
      screen.queryByTestId("request-inspector-binary"),
    ).not.toBeInTheDocument();
  });
});
