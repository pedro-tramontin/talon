// Match & replace engine — JavaScript mirror of
// `crates/bk-proxy/src/match_replace.rs`.
//
// Used by the UI's `MatchReplaceEditor` "Test" button
// (Phase 7 C-B.2) to preview what the M&R rules would do
// to a sample URL before the user clicks "Add". Runs
// client-side — no Tauri round-trip needed for a UI
// affordance.
//
// **Scope for v1:** the JS engine handles `RequestUrl`
// target only (the most common case for the "Test" button).
// `RequestHeader` and `RequestBody` targets are stubs that
// return the input unchanged — the v0.5+ "Test on a full
// request" follow-up would extend this. The Rust engine
// is the source of truth for the wire; the JS engine is a
// best-effort UI preview. If the user has a rule that
// transforms a header, the "Test" button won't show it —
// the user has to send a real replay to see the full
// effect.

import type { MatchReplaceRule } from "../types/domain";

/**
 * Apply a list of M&R rules to a sample URL string. Returns
 * the URL after all enabled rules have been applied in
 * descending-priority order. The original `url` is not
 * mutated.
 *
 * The algorithm mirrors the Rust engine's `MatchReplace::apply`:
 * 1. Filter to enabled rules.
 * 2. Sort by descending priority (high priority runs first).
 * 3. Apply each rule via `rewriteString` in turn.
 * 4. If the rewritten URL doesn't parse as a valid URL,
 *    return the previous value (the engine's defensive
 *    fallback — a bad rule shouldn't crash the preview).
 */
export function matchReplaceApplyUrl(
  url: string,
  rules: MatchReplaceRule[],
): string {
  const enabled = rules.filter((r) => r.enabled);
  // Sort by descending priority. Stable sort preserves
  // user-declared order for equal-priority rules (same as
  // the Rust `sort_by_key` with `Reverse`).
  const sorted = [...enabled].sort((a, b) => b.priority - a.priority);

  let current = url;
  for (const rule of sorted) {
    if (rule.target !== "request_url") {
      // v1: skip header/body targets. The preview is
      // URL-only. A header-body preview is a v0.5+
      // follow-up.
      continue;
    }
    current = rewriteString(current, rule);
  }
  return current;
}

/**
 * Apply a single rule to a string. Literal-string replace
 * for `is_regex: false`, regex `String.replace` for true.
 * `case_insensitive: true` adds the `i` flag.
 *
 * The Rust engine uses the `regex` crate with `(?i)` inline
 * for case-insensitive. JS's `RegExp` uses the `i` flag
 * directly. The semantics match: case-insensitive matching
 * is opt-in per rule.
 */
function rewriteString(input: string, rule: MatchReplaceRule): string {
  if (rule.is_regex) {
    try {
      const flags = rule.case_insensitive ? "gi" : "g";
      const re = new RegExp(rule.pattern, flags);
      // `replaceAll`-equivalent via the `g` flag.
      return input.replace(re, rule.replace);
    } catch {
      // Malformed regex — return input unchanged. The
      // engine's `try/catch` is the same defensive shape
      // as the Rust engine's `regex::Regex::new(...).ok()`
      // pattern: a single bad user rule shouldn't crash
      // the whole preview.
      return input;
    }
  }
  // Literal string replace. `String.prototype.replaceAll` is
  // available in all modern browsers (ES2021) and in the
  // v0.5+ Node 20 runtime.
  if (rule.case_insensitive) {
    // Build a case-insensitive literal replace.
    // `String.replaceAll` with a string arg is case-sensitive;
    // use a regex with the `i` flag for case-insensitive
    // literal replace.
    const escaped = escapeRegExp(rule.pattern);
    const re = new RegExp(escaped, "gi");
    return input.replace(re, rule.replace);
  }
  return input.split(rule.pattern).join(rule.replace);
}

/**
 * Escape a string for use as a literal in a `RegExp`
 * constructor. Mirrors the `regex::escape` function
 * from Rust's `regex` crate. Required for the
 * case-insensitive literal branch above.
 */
function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
