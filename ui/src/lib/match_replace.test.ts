// Vitest cases for the JS-side `match_replace.ts` engine
// (Phase 7 C-B.2 — the "Test" button's engine).
//
// The cases mirror the Rust `MatchReplace::apply` test
// cases in `crates/bk-proxy/src/match_replace.rs` (4 cases:
// literal replace, no match, priority order, disabled
// rules). The JS engine's scope is narrower (URL only), so
// the header/body cases are excluded.

import { describe, expect, it } from "vitest";
import { matchReplaceApplyUrl } from "./match_replace";
import type { MatchReplaceRule } from "../types/domain";

const rule = (
  overrides: Partial<MatchReplaceRule> = {},
): MatchReplaceRule => ({
  target: "request_url",
  case_insensitive: false,
  is_regex: false,
  pattern: "",
  replace: "",
  enabled: true,
  priority: 0,
  ...overrides,
});

describe("matchReplaceApplyUrl", () => {
  it("applies a literal string replace on the URL", () => {
    const rules = [rule({ pattern: "old.example.com", replace: "new.example.com" })];
    expect(matchReplaceApplyUrl("https://old.example.com/path", rules)).toBe(
      "https://new.example.com/path",
    );
  });

  it("returns the input unchanged when no rule matches", () => {
    const rules = [rule({ pattern: "nope", replace: "yes" })];
    expect(matchReplaceApplyUrl("https://example.com/path", rules)).toBe(
      "https://example.com/path",
    );
  });

  it("runs higher-priority rules first", () => {
    // Two rules: a v1->v2 rule at priority 0 (low) and a v2->v3
    // rule at priority 10 (high). The engine sorts by
    // descending priority, so the v2->v3 rule runs first.
    // On the input "/api/v1/foo", neither rule's pattern
    // matches in a way that would chain (v2->v3 doesn't see
    // /v2 in the input). The v1->v2 rule then sees the
    // unchanged input and rewrites to "/api/v2/foo".
    // Final result: "/api/v2/foo" (the high-priority rule
    // had no effect because its pattern didn't match).
    //
    // To exercise priority + chaining, the v1->v2 rule
    // should be high priority (so it produces /api/v2) and
    // the v2->v3 rule should be low priority (so it
    // chains on the high-priority rule's output).
    const rules = [
      rule({ pattern: "/api/v2", replace: "/api/v3", priority: 0 }),
      rule({ pattern: "/api/v1", replace: "/api/v2", priority: 10 }),
    ];
    // Step 1 (high-priority v1->v2): input /api/v1/foo
    //   -> matches /api/v1 -> rewrites to /api/v2/foo
    // Step 2 (low-priority v2->v3): input /api/v2/foo
    //   -> matches /api/v2 -> rewrites to /api/v3/foo
    expect(matchReplaceApplyUrl("https://x.test/api/v1/foo", rules)).toBe(
      "https://x.test/api/v3/foo",
    );
  });

  it("skips disabled rules", () => {
    const rules = [
      rule({ pattern: "/x", replace: "/y", enabled: false }),
      rule({ pattern: "/y", replace: "/z", priority: 1 }),
    ];
    // The disabled rule is filtered out. The enabled rule
    // sees the input and rewrites /y -> /z. (The input
    // doesn't have /y; the rule has no effect; final is the
    // input unchanged.)
    expect(matchReplaceApplyUrl("https://x.test/path", rules)).toBe(
      "https://x.test/path",
    );
  });

  it("respects case_insensitive on literal rules", () => {
    const rules = [
      rule({ pattern: "OLD", replace: "new", case_insensitive: true }),
    ];
    expect(matchReplaceApplyUrl("https://old.example.com/", rules)).toBe(
      "https://new.example.com/",
    );
  });

  it("applies regex rules", () => {
    const rules = [
      rule({
        pattern: "v(\\d+)",
        replace: "v$1-fixed",
        is_regex: true,
      }),
    ];
    expect(matchReplaceApplyUrl("https://x.test/api/v1/foo", rules)).toBe(
      "https://x.test/api/v1-fixed/foo",
    );
  });

  it("skips malformed regex without crashing", () => {
    const rules = [
      rule({ pattern: "[unclosed", replace: "x", is_regex: true }),
      rule({ pattern: "/a", replace: "/b" }),
    ];
    // The malformed regex is caught by the `try/catch`; the
    // engine returns the input unchanged for that rule. The
    // second rule still applies.
    expect(matchReplaceApplyUrl("https://x.test/a/c", rules)).toBe(
      "https://x.test/b/c",
    );
  });

  it("skips non-URL targets (header/body stubs for v1)", () => {
    const rules = [
      rule({ target: "request_header", pattern: "X-Foo: bar", replace: "X-Foo: baz" }),
      rule({ target: "request_url", pattern: "/a", replace: "/b" }),
    ];
    // The header target is skipped; the URL target applies.
    expect(matchReplaceApplyUrl("https://x.test/a/c", rules)).toBe(
      "https://x.test/b/c",
    );
  });
});
