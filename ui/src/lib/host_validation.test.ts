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
// Per the per-item `objective:` block, the helper-test
// estimate is 3 cases (the spec lists exactly 3: a
// hostname accepted, an IPv4 accepted, malformed hosts
// rejected). Pre-trimmed to the 3 spec cases; do not add
// more without re-deriving the spec estimate.

import { describe, expect, it } from "vitest";
import { isValidHostShape } from "./host_validation";

describe("isValidHostShape (TS mirror of app/src/commands/core.rs:is_valid_host_shape)", () => {
  it("accepts a valid RFC 1123 hostname", () => {
    // Spec case 7: "RFC 1123 hostname accepted
    // (`api.acme.example.com`)". Mirrors the multi-case
    // `for` loop in
    // `open_project_accepts_valid_hostname` at
    // `app/src/commands/core.rs` line 603-617.
    expect(isValidHostShape("api.acme.example.com")).toBe(true);
    for (const host of ["acme.bb", "localhost", "my-host"]) {
      expect(isValidHostShape(host)).toBe(true);
    }
  });

  it("accepts a valid IPv4 literal", () => {
    // Spec case 8: "IPv4 accepted (`10.0.0.1`)". Mirrors
    // `open_project_accepts_valid_ipv4` at
    // `app/src/commands/core.rs` line 619-627.
    expect(isValidHostShape("10.0.0.1")).toBe(true);
  });

  it("rejects a malformed host shape", () => {
    // Spec case 9: "malformed hosts rejected (URL with
    // port, IPv6, empty, whitespace-only, 254 chars)".
    // Covers the same 5 cases the spec enumerated.
    expect(isValidHostShape("foo:8080")).toBe(false); // URL with port
    expect(isValidHostShape("::1")).toBe(false); // IPv6
    expect(isValidHostShape("")).toBe(false); // empty
    expect(isValidHostShape("   ")).toBe(false); // whitespace
    expect(isValidHostShape(`${"a".repeat(253)}.`)).toBe(false); // 254 chars
  });
});
