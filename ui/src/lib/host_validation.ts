// Client-side mirror of `is_valid_host_shape` in
// `app/src/commands/core.rs` (Rust source of truth for what
// counts as a valid `target_host`). Update both at once if
// the rules change.
//
// **Why the mirror exists:** the v0.5 modal gives the user
// immediate feedback on the `target_host` field before the
// Tauri round-trip. The Rust validator is still the gate —
// if the mirror and the Rust validator ever disagree, the
// Rust validator wins, and the user sees the Rust error
// message in the modal.
//
// **The mirror is char-for-char:** same rules, same edge
// cases (RFC 1123 hostname OR IPv4 literal; no IPv6; no
// URLs/ports; max 253 chars). The vitest cases in
// `host_validation.test.ts` are the same 4-6 cases the
// Rust side tests in
// `app/src/commands/core.rs::open_project_rejects_*` /
// `open_project_accepts_valid_*`.

/**
 * Returns `true` if `s` looks like a valid hostname or IP
 * literal per the Rust `is_valid_host_shape` rules.
 *
 * Accepted shapes:
 * - **IPv4 literal**: four dotted decimal octets, each
 *   0..=255. No leading zeros (e.g. `010.0.0.1` is
 *   rejected).
 * - **Hostname**: 1..=253 chars, each label 1..=63 chars,
 *   labels contain `[A-Za-z0-9-]` and don't start or end
 *   with `-`, labels separated by `.`.
 *
 * Not accepted (intentionally):
 * - IPv6 literal (the Rust validator rejects `:` as a
 *   URL/port separator; IPv6 support is a v0.5+ follow-up)
 * - URLs, ports, paths, queries, fragments (anything
 *   containing `:`, `/`, `?`, `#`)
 * - Whitespace, control characters
 * - Empty string or > 253 chars
 */
export function isValidHostShape(s: string): boolean {
  if (s.length === 0 || s.length > 253) {
    return false;
  }
  // Reject embedded whitespace, control chars, and the URL
  // scheme separators that would indicate the user pasted a
  // full URL into the target_host field.
  for (const c of s) {
    if (
      c === " " ||
      c === "\t" ||
      c === "\n" ||
      c === "\r" ||
      // The Rust `c.is_whitespace()` is broader than just
      // space/tab/newline (it includes U+00A0 non-breaking
      // space, U+200B zero-width space, etc.). The mirror
      // uses a RegExp test to match the Rust
      // `is_whitespace` semantics.
      /\s/.test(c) ||
      // Control characters: 0x00-0x1F, 0x7F. Matches
      // `char::is_control` in Rust.
      (c.charCodeAt(0) >= 0x00 && c.charCodeAt(0) <= 0x1f) ||
      c.charCodeAt(0) === 0x7f ||
      c === ":" ||
      c === "/" ||
      c === "?" ||
      c === "#"
    ) {
      return false;
    }
  }
  // If the input contains only digits and dots, it MUST be
  // a valid IPv4 (anything else in the digits-and-dots
  // space is a typo, e.g. "010.0.0.1" or "256.0.0.1" or
  // "1.2.3"). Falling through to the hostname check would
  // let those through because hostnames are allowed to
  // contain digits and dots. The right rule: if the input
  // looks like an attempted IPv4 (only digits + dots), it
  // must be a valid IPv4 or be rejected; if it has any
  // other valid hostname characters (letters or hyphens),
  // accept it as a hostname.
  let allDigitsAndDots = true;
  for (const c of s) {
    if (!/[0-9.]/.test(c)) {
      allDigitsAndDots = false;
      break;
    }
  }
  if (allDigitsAndDots) {
    return looksLikeIpv4(s);
  }
  // Hostname path.
  return isValidHostname(s);
}

/**
 * Returns `true` if `s` is four dotted decimal octets in
 * `0..=255` with no leading zeros. Matches the Rust
 * `looks_like_ipv4` helper.
 */
function looksLikeIpv4(s: string): boolean {
  const parts = s.split(".");
  if (parts.length !== 4) {
    return false;
  }
  for (const p of parts) {
    if (p.length === 0 || p.length > 3) {
      return false;
    }
    if (p.length > 1 && p.startsWith("0")) {
      return false;
    }
    const n = Number(p);
    if (!Number.isInteger(n) || n < 0 || n > 255) {
      return false;
    }
  }
  return true;
}

/**
 * Returns `true` if `s` is a valid RFC 1123 hostname.
 * Matches the Rust `is_valid_hostname` helper.
 */
function isValidHostname(s: string): boolean {
  if (s.length === 0 || s.length > 253) {
    return false;
  }
  const labels = s.split(".");
  for (const label of labels) {
    if (label.length === 0 || label.length > 63) {
      return false;
    }
    if (label.startsWith("-") || label.endsWith("-")) {
      return false;
    }
    for (const c of label) {
      const code = c.charCodeAt(0);
      const isAlnum =
        (code >= 0x30 && code <= 0x39) || // 0-9
        (code >= 0x41 && code <= 0x5a) || // A-Z
        (code >= 0x61 && code <= 0x7a); // a-z
      if (!isAlnum && c !== "-") {
        return false;
      }
    }
  }
  return true;
}
