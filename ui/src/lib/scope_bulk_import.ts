// SecLists hosts-format parser for the `ScopeRuleEditor`'s
// "Bulk import" button (Phase 7 C-B.4).
//
// The SecLists hosts format is one host per line, with
// `#` as the standard comment marker. The parser is
// intentionally permissive (tolerates `//` as a comment
// marker; trims whitespace) but drops:
//   - empty lines
//   - comment lines (`#` or `//`)
//   - wildcard lines (`*.` prefix; the per-row editor
//     supports wildcards, the bulk-import path does not)
//   - duplicates against the existing `scopeRules` array
//     (the function takes the current array as a 2nd arg)
//
// Returns the parsed host strings + their 1-indexed line
// numbers (for error reporting). The line number is the
// position in the *original* input (post-trim), not the
// post-dedup position. This lets the UI show the user
// "host at line 42 already exists" if we ever want that
// (the v1 surfaces only the count).
//
// The parser is pure (no I/O, no Tauri, no React). Tests
// live in `scope_bulk_import.test.ts`.

import type { ScopeRule } from "../types/domain";

/**
 * One parsed host from the input. `host` is the trimmed
 * non-comment, non-empty, non-duplicate host string.
 * `lineNo` is the 1-indexed line number in the *original*
 * input (preserved for error reporting).
 */
export interface ParsedHost {
  readonly host: string;
  readonly lineNo: number;
}

/**
 * Parse a SecLists hosts-format input.
 *
 * The parser is forgiving on input: trailing whitespace,
 * CRLF line endings, and tabs are all trimmed. Lines
 * starting with `#` or `//` (after trim) are treated as
 * comments and dropped. Lines starting with `*.` are
 * dropped (the editor's per-row wildcards are not
 * supported in bulk-import). Empty lines are dropped.
 *
 * Duplicates against the *existing* `scopeRules` array
 * are dropped. Duplicates within the input itself are
 * also dropped (the first occurrence wins; later
 * occurrences are silently skipped — this matches the
 * `addScopeRule` behavior of appending non-duplicates).
 *
 * @param input  The raw text from the user's file.
 * @param existing  The current scope rules array (used
 *   for cross-dedup against already-imported rules).
 * @returns The list of parsed hosts + their line numbers.
 */
export function parseSecListsHosts(
  input: string,
  existing: readonly ScopeRule[],
): ParsedHost[] {
  const seen = new Set<string>();
  // Seed the dedup set with the existing rules' patterns
  // (the function takes the current array as a 2nd arg).
  for (const rule of existing) {
    seen.add(rule.pattern);
  }

  const result: ParsedHost[] = [];
  const lines = input.split(/\r?\n/);
  for (let i = 0; i < lines.length; i++) {
    const raw = lines[i];
    const trimmed = raw.trim();
    if (trimmed === "") continue;
    if (trimmed.startsWith("#") || trimmed.startsWith("//")) continue;
    // Wildcards not supported in bulk-import; the per-row
    // editor handles `*.` patterns one at a time.
    if (trimmed.startsWith("*.")) continue;
    if (seen.has(trimmed)) continue;
    seen.add(trimmed);
    result.push({ host: trimmed, lineNo: i + 1 });
  }
  return result;
}
