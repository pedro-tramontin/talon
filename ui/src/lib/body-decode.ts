//! Body-decoding helpers shared across the inspector components.
//!
//! **v0.5 wire form:** `Body::Complete.data` is a base64 string
//! (e.g. `"aGVsbG8="` for "hello"). **v0.1 wire form (still
//! accepted for backwards compat with already-stored SQLite
//! rows):** a JSON array of byte values, e.g.
//! `[104, 101, 108, 108, 111]`. The Rust deserializer
//! (`body_complete_data_serde` in
//! `crates/bk-core/src/model.rs`) accepts both; this module
//! mirrors that on the TS side.
//!
//! The detection is by `typeof data === "string"` (new) vs.
//! `Array.isArray(data)` (legacy). All call sites in the UI
//! should go through the helpers in this module rather than
//! calling `new Uint8Array(body.data)` directly — the v0.5
//! form would silently create a UTF-8 view of the base64
//! chars instead of the decoded bytes.

import type { ExchangeBody } from "../types/domain";

/**
 * Decode a `Body::Complete` payload to a `Uint8Array`.
 *
 * Returns `null` if:
 * - the body is `Empty` or `Streaming`;
 * - the `data` field is not a string (new form) or a number
 *   array (legacy form) — i.e. malformed wire data;
 * - the v0.5 form's string is not valid base64.
 *
 * The byte length of an empty `Complete` body is 0 (the
 * `data` field is the empty string `""` or the empty array
 * `[]`); this helper returns an empty `Uint8Array` in that
 * case, NOT `null`, so callers can use the byte length to
 * distinguish "no body" from "binary body".
 */
export function decodeBodyToBytes(
  body: ExchangeBody | null | undefined,
): Uint8Array | null {
  if (!body || body.kind !== "complete") return null;
  const data = body.data;
  if (typeof data === "string") {
    // New v0.5 form: base64 string.
    if (data.length === 0) return new Uint8Array(0);
    try {
      const binary = atob(data);
      const out = new Uint8Array(binary.length);
      for (let i = 0; i < binary.length; i++) {
        out[i] = binary.charCodeAt(i);
      }
      return out;
    } catch {
      // The Rust base64 alphabet is standard (`A-Z a-z 0-9 + / =`).
      // A `atob` failure means the wire data is malformed (e.g.
      // a non-base64 string slipped through). Surface as
      // "binary" to the UI rather than crashing.
      return null;
    }
  }
  // Legacy v0.1 form: `number[]` (each element is a byte 0..=255).
  if (!Array.isArray(data)) return null;
  return new Uint8Array(data);
}

/**
 * Decode a `Body::Complete` payload to a UTF-8 string.
 *
 * Returns `null` if:
 * - the body is `Empty` or `Streaming`;
 * - the bytes are not valid UTF-8.
 *
 * Callers use the `null` signal to swap in the binary
 * placeholder. The empty-body case returns `""` (NOT `null`)
 * so the UI's "No body" branch fires correctly.
 */
export function decodeBodyUtf8(
  body: ExchangeBody | null | undefined,
): string | null {
  const bytes = decodeBodyToBytes(body);
  if (bytes === null) return null;
  if (bytes.length === 0) return "";
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    return null;
  }
}

/**
 * Format a body size (in bytes) as a short human-readable
 * string. Used in the binary placeholder so the user has
 * at least a sense of the payload size.
 */
export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}
