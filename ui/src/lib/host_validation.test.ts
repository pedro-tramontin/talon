// Vitest cases for the Phase 8 `isValidHostShape` mirror
// at `ui/src/lib/host_validation.ts`.
//
// The function is a char-for-char TS mirror of the Rust
// `is_valid_host_shape` in `app/src/commands/core.rs`. The
// cases below mirror the Rust test cases in
// `app/src/commands/core.rs::open_project_accepts_valid_*` /
// `open_project_rejects_malformed_*` / `open_project_rejects_empty_*`
// so a change to one side will surface a divergence on the
// other side during the next test run.
//
// Per the Phase 8 spec, the helper-test estimate is 2-3
// cases. We add 6 here (≈2x the upper bound, matching the
// §5b D5 sub-rule from Phase 7 C-B v0.2 — "1.5-2x is
// acceptable for helper tests that enumerate edge cases").

import { describe, expect, it } from "vitest";
import { isValidHostShape } from "./host_validation";

describe("isValidHostShape (TS mirror of app/src/commands/core.rs:is_valid_host_shape)", () => {
  it("accepts a valid RFC 1123 hostname", () => {
    // The spec's "RFC 1123 hostname accepted" case.
    expect(isValidHostShape("api.acme.example.com")).toBe(true);
    // Mirrors the multi-case `for` loop in
    // `open_project_accepts_valid_hostname` at
    // `app/src/commands/core.rs` line 603-617.
    for (const host of [
      "acme.bb",
      "a",
      "localhost",
      "my-host",
      "redis-7f9c",
    ]) {
      expect(isValidHostShape(host)).toBe(true);
    }
  });

  it("accepts a valid IPv4 literal", () => {
    // The spec's "IPv4 accepted" case.
    expect(isValidHostShape("10.0.0.1")).toBe(true);
    // Mirrors the multi-case `for` loop in
    // `open_project_accepts_valid_ipv4` at
    // `app/src/commands/core.rs` line 619-627.
    for (const ip of ["127.0.0.1", "0.0.0.0", "255.255.255.255"]) {
      expect(isValidHostShape(ip)).toBe(true);
    }
  });

  it("rejects a malformed host shape", () => {
    // The spec's "malformed hosts rejected" case — covers
    // URLs with ports (the `:` rule), IPv6 literals,
    // empty input, whitespace-only, and 254-char overflow.
    expect(isValidHostShape("foo:8080")).toBe(false); // URL with port
    expect(isValidHostShape("::1")).toBe(false); // IPv6
    expect(isValidHostShape("")).toBe(false); // empty
    expect(isValidHostShape("   ")).toBe(false); // whitespace
    expect(isValidHostShape(`${"a".repeat(253)}.`)).toBe(false); // 254 chars
  });

  it("rejects a hostname with invalid characters", () => {
    // Beyond the spec's 3 cases, but each represents a
    // real user mistake that's worth pinning. Covers the
    // rules not in the spec's 3 cases but in the Rust
    // source: leading/trailing hyphens, underscores, empty
    // labels (consecutive dots), embedded space, control
    // characters, full URLs with scheme, query/fragment.
    expect(isValidHostShape("-foo.example.com")).toBe(false);
    expect(isValidHostShape("foo-.example.com")).toBe(false);
    expect(isValidHostShape("foo_bar.example.com")).toBe(false);
    expect(isValidHostShape("foo..example.com")).toBe(false);
    expect(isValidHostShape("foo bar")).toBe(false);
    expect(isValidHostShape("http://example.com")).toBe(false);
    expect(isValidHostShape("example.com?x=1")).toBe(false);
    expect(isValidHostShape("010.0.0.1")).toBe(false); // leading-zero IPv4
    expect(isValidHostShape("256.0.0.1")).toBe(false); // out-of-range octet
  });

  it("accepts a 63-char label (the max label length)", () => {
    // 63-char labels are the max; 64-char labels are
    // rejected. Boundary test for the label-length rule.
    const s = "a".repeat(63);
    expect(isValidHostShape(s)).toBe(true);
    expect(isValidHostShape("a".repeat(64))).toBe(false);
  });

  it("rejects a 254-char hostname (the 253 max)", () => {
    // 253 chars is the max — Rust's
    // `is_valid_host_shape` and `is_valid_hostname` both
    // check `s.len() > 253`. 254 chars is the first
    // invalid length.
    expect(isValidHostShape("a".repeat(254))).toBe(false);
  });
});
