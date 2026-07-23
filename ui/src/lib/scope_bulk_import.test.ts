// Unit tests for the `parseSecListsHosts` parser
// (Phase 7 C-B.4).
//
// The parser is pure (no I/O, no React), so these tests
// exercise the function directly. The vitest cases
// mirror the 8 cases the `objective:` block enumerates.

import { describe, expect, it } from "vitest";
import { parseSecListsHosts } from "./scope_bulk_import";
import type { ScopeRule } from "../types/domain";

const emptyRules: ScopeRule[] = [];

describe("parseSecListsHosts", () => {
  it("returns an empty list for empty input", () => {
    expect(parseSecListsHosts("", emptyRules)).toEqual([]);
  });

  it("returns an empty list for comment-only input", () => {
    expect(parseSecListsHosts("# one\n# two\n# three\n", emptyRules)).toEqual(
      [],
    );
  });

  it("parses a single host", () => {
    expect(parseSecListsHosts("example.com\n", emptyRules)).toEqual([
      { host: "example.com", lineNo: 1 },
    ]);
  });

  it("parses a mixed input (hosts + comments + blanks) preserving line numbers", () => {
    const input =
      "# SecLists hosts file\na.test\n\n# in-scope block\nb.test\n\nc.test\n";
    expect(parseSecListsHosts(input, emptyRules)).toEqual([
      { host: "a.test", lineNo: 2 },
      { host: "b.test", lineNo: 5 },
      { host: "c.test", lineNo: 7 },
    ]);
  });

  it("drops wildcard lines and trims whitespace", () => {
    const input = "  *.example.com  \n\tb.test\t\n";
    expect(parseSecListsHosts(input, emptyRules)).toEqual([
      { host: "b.test", lineNo: 2 },
    ]);
  });

  it("dedups duplicates within the input (first wins)", () => {
    const input = "a.test\nb.test\na.test\nc.test\n";
    expect(parseSecListsHosts(input, emptyRules)).toEqual([
      { host: "a.test", lineNo: 1 },
      { host: "b.test", lineNo: 2 },
      { host: "c.test", lineNo: 4 },
    ]);
  });

  it("dedups against the existing scope rules array", () => {
    const existing: ScopeRule[] = [
      {
        kind: "host",
        pattern: "already.test",
        action: "in_scope",
        label: "existing",
        priority: 0,
      },
    ];
    const input = "already.test\nb.test\n";
    expect(parseSecListsHosts(input, existing)).toEqual([
      { host: "b.test", lineNo: 2 },
    ]);
  });

  it("handles CRLF line endings (Windows files)", () => {
    const input = "a.test\r\nb.test\r\n";
    expect(parseSecListsHosts(input, emptyRules)).toEqual([
      { host: "a.test", lineNo: 1 },
      { host: "b.test", lineNo: 2 },
    ]);
  });
});
