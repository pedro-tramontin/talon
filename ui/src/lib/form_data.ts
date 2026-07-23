// Form-data parser for the `ReplayRequestEditor`'s
// "Pretty" view (Phase 7 C-B.5).
//
// The parser handles `application/x-www-form-urlencoded`
// bodies. It is a thin wrapper around `URLSearchParams`
// (the browser-native URL decoder) — we don't reimplement
// URL decoding in v1; the spec says "URLSearchParams-parses
// the input".
//
// The 1 MB cap is enforced in the component (the editor
// shows up to ~1 MB of text before truncating; the Pretty
// view checks `input.length > 1_000_000` and shows a
// "body too large" message in that case). The parser
// itself does NOT cap input length — the component
// decides when to call the parser.
//
// Returns `Array<[string, string]>` (the v1 shape):
//   - key-value pairs in input order
//   - values are URL-decoded (spaces, `%2F`, etc. all
//     become their decoded form via `URLSearchParams.get`)
//   - duplicate keys are preserved (a key that appears
//     twice shows up as two pairs, in input order)
//
// Pure function, no I/O, no React. Tests in
// `form_data.test.ts`.

/**
 * Parse a form-data body. The input is the raw
 * `application/x-www-form-urlencoded` string (e.g.
 * `"a=1&b=hello%20world&c=foo%2Fbar"`). Returns the
 * key-value pairs in input order with values URL-decoded.
 *
 * Empty input → `[]` (the `URLSearchParams` constructor
 * returns an empty iterator, and we map it to an empty
 * array).
 *
 * Duplicate keys are preserved (a key that appears twice
 * shows up as two pairs, in input order). This matches
 * the URLSearchParams iterator semantics.
 */
export function parseFormData(
  input: string,
): Array<[string, string]> {
  if (input === "") return [];
  const params = new URLSearchParams(input);
  const out: Array<[string, string]> = [];
  // `URLSearchParams` iterates in insertion order, so
  // the output preserves the input order.
  for (const [k, v] of params.entries()) {
    out.push([k, v]);
  }
  return out;
}
