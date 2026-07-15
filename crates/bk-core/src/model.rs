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

    /// Serialize as `{"name": [value, value, ...], ...}`.
    pub fn serialize<S: Serializer>(m: &HeaderMap, ser: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        // Group values by header name; `http::HeaderMap` iter is (Name, Value).
        let mut grouped: std::collections::BTreeMap<String, Vec<String>> =
            std::collections::BTreeMap::new();
        for (name, value) in m.iter() {
            let v = value
                .to_str()
                .map_err(serde::ser::Error::custom)?
                .to_owned();
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Body {
    Complete { data: Bytes },
    Streaming { content_length: Option<u64> },
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
            Body::Streaming { content_length } => content_length.map(|n| n as usize).unwrap_or(0),
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
    /// (Phase 6) to decide whether a request is in-scope.
    pub fn host(&self) -> String {
        self.url.host_str().unwrap_or("").to_ascii_lowercase()
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
        let r = Request::get("https://example.com/api/users").unwrap();
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
        let r = Request::get("https://EXAMPLE.com/").unwrap();
        assert_eq!(r.host(), "example.com");
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
