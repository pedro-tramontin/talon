//! HTTP model types. The whole proxy + fuzzer + replay stack speaks these.

// See the note in `error.rs` for why we silence `missing_docs` locally:
// the plan's "exact code" doesn't include per-item doc comments, and the
// field-level docs are covered by the struct-level docs.
#![allow(missing_docs)]

use bytes::Bytes;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

pub use http::{Method, Version};
pub use url::Url;

/// Serde helper that serializes a `bytes::Bytes` field as a
/// base64-encoded string instead of `bytes::Bytes`'s default
/// `serialize_bytes` (which serde_json renders as a JSON array of
/// numbers, e.g. `[104,101,108,108,111]` for "hello").
///
/// The v0.5 fixup (replaces the v0.1 JS-side `parseExchange` base64
/// conversion in `ui/src/api.ts`) lets serde produce the base64
/// string directly, which is:
/// - **3-4× more compact on the wire** (each byte becomes ~1.4
///   base64 chars vs. 1-3 decimal digits + 1 comma + 1 space);
/// - **1 less conversion point** to debug (the wire shape IS the
///   in-memory shape);
/// - **backwards-compatible with the in-memory representation** —
///   `bytes::Bytes` is still the in-memory type (zero-copy,
///   refcounted, the right primitive for buffers). Only the wire
///   shape changes.
///
/// The `Visitor::visit_string` arm is included so a JSON producer
/// that emits a string `"AQID..."` deserializes correctly. The
/// `visit_bytes` and `visit_byte_buf` arms accept the legacy
/// `Vec<u8>` wire form for backwards compatibility with already-
/// stored exchanges in the SQLite database (the `body_data` BLOB
/// column reads via `Vec<u8>`; we deserialize from `Vec<u8>` for
/// old rows, and from a base64 string for any new wire path).
mod body_complete_data_serde {
    use bytes::Bytes;
    use serde::{de::Error as _, Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(data: &Bytes, ser: S) -> Result<S::Ok, S::Error> {
        // base64::encode (the STANDARD alphabet, with `=` padding)
        // is the canonical wire form. The test
        // `body_complete_data_serde_emits_base64_string` pins the
        // exact shape so a future refactor can't quietly change
        // to the byte-array form.
        ser.serialize_str(&base64::encode(data))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<Bytes, D::Error> {
        // The Body is held inside an HttpExchange which is itself
        // serialized to JSON for the Tauri IPC bridge and to
        // a custom SQLite row format for storage. The SQLite
        // path uses `Vec<u8>` (via rusqlite's BLOB); the Tauri
        // path uses JSON. Both need to work.
        //
        // We use an untagged enum to accept either form:
        // - a base64 string (the new wire form, produced by `serialize`)
        // - a byte array (the legacy `Vec<u8>` form, for backward compat
        //   with rows already in the SQLite store and with any test
        //   fixture that was written before this fixup)
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Either {
            B64(String),
            Bytes_(Vec<u8>),
        }
        match Either::deserialize(de)? {
            Either::B64(s) => base64::decode(&s)
                .map(Bytes::from)
                .map_err(D::Error::custom),
            Either::Bytes_(v) => Ok(Bytes::from(v)),
        }
    }
}

// Re-export the http crate's HeaderMap type. We give it a type alias
// so downstream code can swap implementations later if we ever need to
// (e.g., add a redaction layer). For now, it's just `http::HeaderMap`.
pub type HeaderMap = http::HeaderMap;

// ---------------------------------------------------------------------------
// Serde shims for `http` types.
//
// The `http` crate (v1.x) deliberately does not implement `Serialize` /
// `Deserialize` for `Method`, `Version`, or `HeaderMap` (see hyperium/http#55).
// The plan's struct definitions derive both, so we provide hand-written
// `#[serde(with = "...")]` adapters here. This is the minimum-deviation fix
// that keeps the plan's public API (`Request` / `Response` / `HttpExchange`
// containing the same `http` types) intact.
//
//   - `Method`     <-> JSON string (e.g. "GET")
//   - `Version`    <-> JSON string (e.g. "HTTP/1.1")
//   - `HeaderMap`  <-> JSON object of `string -> [string, ...]`
// ---------------------------------------------------------------------------

mod method_serde {
    use super::Method;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(m: &Method, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(m.as_str())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<Method, D::Error> {
        let s = String::deserialize(de)?;
        Method::try_from(s.as_str()).map_err(serde::de::Error::custom)
    }
}

mod version_serde {
    use super::Version;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &Version, ser: S) -> Result<S::Ok, S::Error> {
        ser.collect_str(&format_args!("{:?}", v))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<Version, D::Error> {
        let s = String::deserialize(de)?;
        match s.as_str() {
            "HTTP/0.9" => Ok(Version::HTTP_09),
            "HTTP/1.0" => Ok(Version::HTTP_10),
            "HTTP/1.1" => Ok(Version::HTTP_11),
            "HTTP/2.0" => Ok(Version::HTTP_2),
            "HTTP/3.0" => Ok(Version::HTTP_3),
            other => Err(serde::de::Error::custom(format!(
                "unknown HTTP version: {other}"
            ))),
        }
    }
}

mod header_map_serde {
    use super::HeaderMap;
    use serde::de::{MapAccess, Visitor};
    use serde::{Deserializer, Serializer};
    use std::fmt;

    /// Escape a byte slice as ASCII, turning every non-printable byte
    /// into `\\xNN`. Used to make non-UTF-8 header values serializable.
    fn escape_non_utf8(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len());
        for &b in bytes {
            if b.is_ascii_graphic() || b == b' ' {
                out.push(b as char);
            } else {
                out.push_str(&format!("\\x{b:02x}"));
            }
        }
        out
    }

    /// Serialize as `{"name": [value, value, ...], ...}`.
    /// Non-UTF-8 header values (rare in practice — RFC 7230 requires
    /// header values to be ASCII) are encoded as `\\xNN` escape
    /// sequences so the round-trip preserves the bytes.
    pub fn serialize<S: Serializer>(m: &HeaderMap, ser: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        // Group values by header name; `http::HeaderMap` iter is (Name, Value).
        let mut grouped: std::collections::BTreeMap<String, Vec<String>> =
            std::collections::BTreeMap::new();
        for (name, value) in m.iter() {
            let v = match value.to_str() {
                Ok(s) => s.to_owned(),
                Err(_) => escape_non_utf8(value.as_bytes()),
            };
            grouped.entry(name.as_str().to_owned()).or_default().push(v);
        }
        let mut map = ser.serialize_map(Some(grouped.len()))?;
        for (k, vs) in &grouped {
            map.serialize_entry(k, vs)?;
        }
        map.end()
    }

    /// Deserialize from `{"name": [value, value, ...] | "value", ...}`.
    /// Accepts both single-string and array-of-strings values for robustness
    /// against hand-written JSON.
    pub fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<HeaderMap, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = HeaderMap;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a map of header names to string or array of strings")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut access: M) -> Result<HeaderMap, M::Error> {
                use serde::de::Error;
                let mut out = HeaderMap::new();
                while let Some((name, value)) = access.next_entry::<String, serde_json::Value>()? {
                    let values: Vec<String> = match value {
                        serde_json::Value::String(s) => vec![s],
                        serde_json::Value::Array(arr) => arr
                            .into_iter()
                            .map(|v| match v {
                                serde_json::Value::String(s) => Ok(s),
                                other => Err(M::Error::custom(format!(
                                    "header values must be strings, got {other}"
                                ))),
                            })
                            .collect::<Result<_, M::Error>>()?,
                        other => {
                            return Err(M::Error::custom(format!(
                                "header value must be string or array, got {other}"
                            )));
                        }
                    };
                    for v in values {
                        let hname = http::HeaderName::from_bytes(name.as_bytes())
                            .map_err(M::Error::custom)?;
                        let hval = http::HeaderValue::from_str(&v).map_err(M::Error::custom)?;
                        out.append(hname, hval);
                    }
                }
                Ok(out)
            }
        }
        de.deserialize_map(V)
    }
}

/// Request body. `Complete` is a buffered payload (we've read the whole
/// thing from the wire); `Streaming` means we haven't yet, and a
/// downstream consumer (like the fuzzer) needs to read it on demand.
///
/// For Phase 2 we only need `Complete`. `Streaming` is here so Phase 3
/// can introduce it without a breaking change to the storage schema.
///
/// **v0.5 wire format (added 2026-07-21):** `Complete.data` is serialized
/// as a base64 string (via the `body_complete_data_serde` helper
/// module) instead of `bytes::Bytes`'s default JSON array-of-numbers
/// form. The in-memory type is still `bytes::Bytes` (zero-copy,
/// refcounted — the right primitive for in-process buffers). Only the
/// wire shape changes. The deserializer accepts both forms
/// (base64 string AND legacy `Vec<u8>` array) so already-stored
/// SQLite rows continue to round-trip.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Body {
    Complete {
        #[serde(with = "body_complete_data_serde")]
        data: Bytes,
    },
    Streaming {
        content_length: Option<u64>,
    },
    Empty,
}

impl Body {
    pub fn empty() -> Self {
        Body::Empty
    }
    pub fn from_bytes(b: impl Into<Bytes>) -> Self {
        Body::Complete { data: b.into() }
    }
    pub fn is_empty(&self) -> bool {
        matches!(self, Body::Empty)
    }
    pub fn len(&self) -> usize {
        match self {
            Body::Complete { data } => data.len(),
            Body::Empty => 0,
            // u64 -> usize: clamp to usize::MAX instead of silently
            // truncating. A 4GB+ body on a 32-bit target is not
            // representable, but reporting usize::MAX is at least
            // monotonic and obvious, not a small wrong number.
            Body::Streaming { content_length } => content_length
                .map(|n| usize::try_from(n).unwrap_or(usize::MAX))
                .unwrap_or(0),
        }
    }
}

/// A captured HTTP request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    #[serde(with = "method_serde")]
    pub method: Method,
    pub url: Url,
    #[serde(with = "version_serde")]
    pub version: Version,
    #[serde(with = "header_map_serde")]
    pub headers: HeaderMap,
    pub body: Body,
}

impl Request {
    /// Build a minimal GET request. Used heavily in tests and as the
    /// default in the fuzzer's "starting request" config.
    pub fn get(url: impl AsRef<str>) -> Result<Self, crate::Error> {
        Ok(Self {
            method: Method::GET,
            url: url
                .as_ref()
                .parse()
                .map_err(|e: url::ParseError| crate::Error::Invalid(e.to_string()))?,
            version: Version::HTTP_11,
            headers: HeaderMap::new(),
            body: Body::empty(),
        })
    }

    /// Convenience: the URL's host (lowercased). Used by the scope engine
    /// (Phase 6) to decide whether a request is in-scope. Returns `None`
    /// for URLs without a host component (e.g. `file:///path`).
    pub fn host(&self) -> Option<String> {
        self.url.host_str().map(str::to_ascii_lowercase)
    }
}

/// A captured HTTP response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    #[serde(with = "version_serde")]
    pub version: Version,
    pub status: u16,
    pub status_text: String,
    #[serde(with = "header_map_serde")]
    pub headers: HeaderMap,
    pub body: Body,
}

/// Metadata about a single request/response pair (the "exchange").
/// This is what gets indexed in the SQLite FTS5 search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExchangeMeta {
    pub id: crate::ExchangeId,
    pub project_id: crate::ProjectId,
    pub timestamp: DateTime<Utc>,
    /// In nanoseconds, measured from request send to first response byte.
    pub duration_ns: u64,
    /// Short label like "GET /api/users". Used in the exchange list rows.
    pub summary: String,
    /// In-scope vs. out-of-scope at the time the request was logged.
    pub scope_state: ScopeState,
    /// Free-form notes attached to the exchange.
    pub notes: String,
    /// True if the user marked this exchange as interesting (the ⭐ tag).
    pub starred: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeState {
    InScope,
    OutOfScope,
    Blocked,
    Unscoped,
}

impl ScopeState {
    /// Color hint for the UI's left-border accent on each row.
    pub fn accent_class(&self) -> &'static str {
        match self {
            ScopeState::InScope => "border-l-scope-in",
            ScopeState::OutOfScope => "border-l-scope-out",
            ScopeState::Blocked => "border-l-scope-blocked",
            ScopeState::Unscoped => "border-l-scope-unscoped",
        }
    }
}

/// A captured request + response + metadata. This is the primary unit
/// that flows through the proxy, the storage layer, the replay tabs, and
/// the fuzzer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpExchange {
    pub meta: ExchangeMeta,
    pub request: Request,
    pub response: Option<Response>,
    /// True if the proxy decided not to send the request (e.g., scope
    /// rule said "block this host"). The response field is None in this
    /// case and `blocked_reason` explains why.
    pub blocked_reason: Option<String>,
}

impl fmt::Display for HttpExchange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.meta.summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProjectId;

    #[test]
    fn request_get_parses_url() {
        let r = Request::get("https://example.com/api/users").expect("valid URL");
        assert_eq!(r.method, Method::GET);
        assert_eq!(r.url.host_str(), Some("example.com"));
        assert_eq!(r.url.path(), "/api/users");
    }

    #[test]
    fn request_get_rejects_bad_url() {
        let r = Request::get("not a url");
        assert!(r.is_err());
    }

    #[test]
    fn request_host_is_lowercased() {
        let r = Request::get("https://EXAMPLE.com/").expect("valid URL");
        assert_eq!(r.host().as_deref(), Some("example.com"));
    }

    #[test]
    fn body_len_matches_buffered_size() {
        let b = Body::from_bytes(vec![0u8; 42]);
        assert_eq!(b.len(), 42);
        assert!(!b.is_empty());
    }

    #[test]
    fn empty_body_has_zero_len() {
        let b = Body::empty();
        assert_eq!(b.len(), 0);
        assert!(b.is_empty());
    }

    #[test]
    fn exchange_serializes_to_json_and_back() {
        let r = Request::get("https://acme.bb/login").unwrap();
        let resp = Response {
            version: Version::HTTP_11,
            status: 200,
            status_text: "OK".to_string(),
            headers: HeaderMap::new(),
            body: Body::from_bytes(r#"{"token":"abc"}"#),
        };
        let exchange = HttpExchange {
            meta: ExchangeMeta {
                id: crate::ExchangeId::new(),
                project_id: ProjectId::new(),
                timestamp: Utc::now(),
                duration_ns: 12_345_678,
                summary: "GET https://acme.bb/login".to_string(),
                scope_state: ScopeState::InScope,
                notes: String::new(),
                starred: false,
            },
            request: r,
            response: Some(resp),
            blocked_reason: None,
        };
        let s = serde_json::to_string(&exchange).unwrap();
        let back: HttpExchange = serde_json::from_str(&s).unwrap();
        assert_eq!(back.meta.id, exchange.meta.id);
        assert_eq!(back.meta.summary, "GET https://acme.bb/login");
        assert!(back.response.is_some());
    }

    /// v0.5 fixup: the `Body::Complete.data` field is serialized
    /// as a base64 string, not as a JSON array of numbers. The
    /// wire form is a string (e.g. `"aGVsbG8="` for "hello"), not
    /// an array (e.g. `[104,101,108,108,111]`). The test pins the
    /// exact shape so a future refactor can't quietly change it.
    #[test]
    fn body_complete_data_serde_emits_base64_string() {
        let body = Body::Complete {
            data: Bytes::from_static(b"hello"),
        };
        let s = serde_json::to_string(&body).unwrap();
        // "hello" = 5 bytes; base64 is ceil(5/3)*4 = 8 chars.
        // We check the substring is present, not the exact JSON
        // shape, so the test survives serde_json's whitespace
        // variation across versions.
        assert!(
            s.contains(r#""data":"aGVsbG8=""#),
            "Body::Complete.data must serialize as a base64 string; \
             got {s} (the v0.5 contract: a base64 string, NOT a JSON array of numbers)"
        );
    }

    /// v0.5 fixup: the deserializer accepts BOTH the new base64
    /// string form AND the legacy `Vec<u8>` array form. The
    /// legacy form is what the SQLite store contains for
    /// already-inserted exchanges (the `body_data` BLOB column
    /// reads as `Vec<u8>`). The v0.5 fixup keeps those rows
    /// round-tripping without a migration.
    #[test]
    fn body_complete_data_serde_accepts_legacy_byte_array() {
        // Legacy wire form: JSON array of numbers (what
        // `bytes::Bytes` produces by default with the `serde` feature).
        let legacy_json = r#"{"kind":"complete","data":[104,101,108,108,111]}"#;
        let body: Body = serde_json::from_str(legacy_json).expect("legacy form deserializes");
        match body {
            Body::Complete { data } => {
                assert_eq!(&data[..], b"hello", "legacy bytes must round-trip");
            }
            _ => panic!("expected Body::Complete, got {body:?}"),
        }
    }

    /// v0.5 fixup: the deserializer accepts the new base64
    /// string form (what the v0.5 wire shape produces).
    #[test]
    fn body_complete_data_serde_accepts_base64_string() {
        let new_json = r#"{"kind":"complete","data":"aGVsbG8="}"#;
        let body: Body = serde_json::from_str(new_json).expect("base64 form deserializes");
        match body {
            Body::Complete { data } => {
                assert_eq!(&data[..], b"hello", "base64 must round-trip");
            }
            _ => panic!("expected Body::Complete, got {body:?}"),
        }
    }

    /// v0.5 fixup: round-trip a `HttpExchange` with a non-empty
    /// `Body::Complete` through serde_json, asserting the bytes
    /// survive. (The existing `exchange_serializes_to_json_and_back`
    /// test covers the happy path; this one explicitly tests
    /// the v0.5 wire format on a multi-byte body.)
    #[test]
    fn body_complete_data_serde_roundtrips_multi_byte_body() {
        let original = Bytes::from_static(b"the quick brown fox jumps over the lazy dog");
        let body = Body::Complete {
            data: original.clone(),
        };
        let s = serde_json::to_string(&body).unwrap();
        let back: Body = serde_json::from_str(&s).unwrap();
        match back {
            Body::Complete { data } => {
                assert_eq!(data, original, "multi-byte body must round-trip");
            }
            _ => panic!("expected Body::Complete, got {back:?}"),
        }
    }

    #[test]
    fn scope_state_accent_class_distinguishes_all_variants() {
        // Belt-and-braces: if a new variant is added, the test forces
        // a conscious decision about its color.
        let states = [
            (ScopeState::InScope, "border-l-scope-in"),
            (ScopeState::OutOfScope, "border-l-scope-out"),
            (ScopeState::Blocked, "border-l-scope-blocked"),
            (ScopeState::Unscoped, "border-l-scope-unscoped"),
        ];
        for (state, expected) in states {
            assert_eq!(state.accent_class(), expected, "mismatch for {:?}", state);
        }
    }
}
