// Unit tests for the `parseFormData` helper
// (Phase 7 C-B.5).
//
// The helper is a thin wrapper around `URLSearchParams`,
// so the tests focus on the cases the `objective:` block
// enumerates:
//   - empty input → []
//   - single key=val
//   - multiple key=val
//   - URL-encoded chars (spaces, slashes, etc.)

import { describe, expect, it } from "vitest";
import { parseFormData } from "./form_data";

describe("parseFormData", () => {
  it("returns an empty list for empty input", () => {
    expect(parseFormData("")).toEqual([]);
  });

  it("parses a single key=val pair", () => {
    expect(parseFormData("a=1")).toEqual([["a", "1"]]);
  });

  it("parses multiple key=val pairs in input order", () => {
    expect(parseFormData("a=1&b=2&c=3")).toEqual([
      ["a", "1"],
      ["b", "2"],
      ["c", "3"],
    ]);
  });

  it("URL-decodes values (spaces, slashes, etc.)", () => {
    expect(parseFormData("a=hello%20world&b=foo%2Fbar")).toEqual([
      ["a", "hello world"],
      ["b", "foo/bar"],
    ]);
  });
});
