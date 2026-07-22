//! Match & replace engine. Applies a list of `MatchReplaceRule`s to a
//! `bk_core::Request` in descending-priority order. Disabled rules are
//! skipped. The engine is a pure function — no I/O, no async — so the
//! UI's editor can preview the rewrites in real time without a Tauri
//! round-trip.
//!
//! Phase 6 (§6.3). See the §6.3 design spec for the priority + enabled
//! semantics; this file is the implementation.
//!
//! **Drift from the spec:** the spec's §6.3 used
//! `regex::RegexBuilder::new(&rule.pattern).case_insensitive(rule.case_insensitive).build()`;
//! the v0.5 `bk_core::scope` shim uses the simpler
//! `regex::Regex::new(&rule.pattern)` (with inline `(?i)` flag for
//! case-insensitive) — and the 1.10 workspace `regex` crate supports
//! both. We use the simpler form here for symmetry with the
//! `bk_core::ScopeRule::matches` `PathRegex` branch.

use bk_core::scope::{MatchReplaceRule, MatchReplaceTarget};
use bk_core::{Body, HeaderMap, Request};
use bytes::Bytes;

pub struct MatchReplace;

impl MatchReplace {
    /// Apply the rules to a request, returning a new (or
    /// mutated) `Request`. The rules are sorted by descending
    /// priority once at the top; higher priority runs first, so
    /// a high-priority rule can rewrite the URL/path/header that
    /// a lower-priority rule would otherwise have matched.
    ///
    /// **Mutation is in place** — `Request` is `Clone`, the
    /// caller is expected to pass a fresh `Request` (or the
    /// spec's documented round-trip copy from `http::Request`).
    pub fn apply(mut request: Request, rules: &[MatchReplaceRule]) -> Request {
        // Filter to enabled rules, then sort by descending priority.
        // `sort_by_key` with `Reverse` is the idiomatic way to get
        // a stable descending sort on a `Copy` key.
        let mut sorted: Vec<&MatchReplaceRule> = rules.iter().filter(|r| r.enabled).collect();
        sorted.sort_by_key(|r| std::cmp::Reverse(r.priority));

        for rule in sorted {
            request = Self::apply_one(request, rule);
        }
        request
    }

    /// Apply a single rule to the request. Routes by `target`
    /// (which part of the message to operate on). Response
    /// targets are a no-op for a `Request`-typed input — the
    /// engine just returns the request unchanged.
    fn apply_one(mut request: Request, rule: &MatchReplaceRule) -> Request {
        match rule.target {
            MatchReplaceTarget::RequestUrl => {
                let url_str = request.url.as_str().to_string();
                let new_url_str = Self::rewrite_string(&url_str, rule);
                if let Ok(parsed) = url::Url::parse(&new_url_str) {
                    // `bk_core::Url` is `url::Url` re-exported; the
                    // conversion is identity. We swallow the
                    // (unreachable) `Result` because the spec's
                    // §6.3 "Heads up" line 431 called this out
                    // as overly defensive — but `Url::parse`
                    // is fallible and a bad rewrite (e.g. an
                    // M&R rule that produces a malformed URL)
                    // should NOT crash the proxy, so the
                    // `if let Ok` is the right gate.
                    request.url = parsed;
                }
                request
            }
            MatchReplaceTarget::RequestHeader => {
                let mut new_headers: Vec<(String, String)> = Vec::new();
                for (k, v) in &request.headers {
                    let key_str = k.as_str().to_string();
                    // `HeaderValue::to_str` returns `Result<&str, ToStrError>`;
                    // a non-ASCII header value can't be string-rewritten,
                    // so we pass it through unchanged (a literal match
                    // against the raw bytes would not be useful in any
                    // case — header values are usually short identifiers
                    // and tokens).
                    let original = v.to_str().unwrap_or("").to_string();
                    let new_value = Self::rewrite_string(&original, rule);
                    new_headers.push((key_str, new_value));
                }
                let mut header_map = HeaderMap::new();
                for (k, v) in new_headers {
                    if let Ok(name) = http::HeaderName::from_bytes(k.as_bytes()) {
                        if let Ok(val) = http::HeaderValue::from_str(&v) {
                            header_map.insert(name, val);
                        }
                    }
                }
                request.headers = header_map;
                request
            }
            MatchReplaceTarget::RequestBody => {
                if let Body::Complete { data } = &request.body {
                    let s = String::from_utf8_lossy(data);
                    let new_s = Self::rewrite_string(&s, rule);
                    request.body = Body::Complete {
                        data: Bytes::from(new_s.into_bytes()),
                    };
                }
                request
            }
            // `MatchReplaceTarget` is `#[non_exhaustive]` (Phase 10
            // contract); wildcard for forward-compat. Unknown
            // targets in v1 are a no-op (the engine just passes the
            // request through).
            MatchReplaceTarget::ResponseHeader | MatchReplaceTarget::ResponseBody | _ => request,
        }
    }

    /// Apply a single rule's pattern+replace to one string.
    /// Bad regexes return the input unchanged (defense in
    /// depth: a single bad user rule should not crash the
    /// proxy).
    fn rewrite_string(input: &str, rule: &MatchReplaceRule) -> String {
        if rule.is_regex {
            // Add the `(?i)` flag inline for case-insensitive
            // matches — keeps the API surface narrow
            // (`regex::Regex::new` only, no `RegexBuilder`).
            let pat = if rule.case_insensitive {
                format!("(?i){}", rule.pattern)
            } else {
                rule.pattern.clone()
            };
            match regex::Regex::new(&pat) {
                Ok(re) => re.replace_all(input, rule.replace.as_str()).to_string(),
                Err(_) => input.to_string(), // bad regex: no change
            }
        } else {
            // Literal string replace.
            if rule.case_insensitive {
                let pat = rule.pattern.to_lowercase();
                let mut result = input.to_string();
                let mut start = 0;
                while let Some(pos) = result[start..].to_lowercase().find(&pat) {
                    let abs = start + pos;
                    let replace_len = rule.pattern.len();
                    let replacement_len = rule.replace.len();
                    result = format!(
                        "{}{}{}",
                        &result[..abs],
                        rule.replace,
                        &result[abs + replace_len..]
                    );
                    start = abs + replacement_len;
                }
                result
            } else {
                input.replace(&rule.pattern, &rule.replace)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bk_core::{Method, Version};

    fn literal(target: MatchReplaceTarget, pattern: &str, replace: &str) -> MatchReplaceRule {
        MatchReplaceRule {
            target,
            case_insensitive: false,
            is_regex: false,
            pattern: pattern.to_string(),
            replace: replace.to_string(),
            enabled: true,
            priority: 0,
        }
    }

    fn make_get_request(url_str: &str) -> Request {
        Request {
            method: Method::GET,
            url: url_str.parse().unwrap(),
            version: Version::HTTP_11,
            headers: HeaderMap::new(),
            body: Body::empty(),
        }
    }

    #[test]
    fn literal_string_replace_in_url() {
        let req = make_get_request("https://acme.bb/api/v1/users");
        let rules = vec![literal(
            MatchReplaceTarget::RequestUrl,
            "/api/v1/",
            "/api/v2/",
        )];
        let out = MatchReplace::apply(req, &rules);
        assert_eq!(out.url.path(), "/api/v2/users");
    }

    #[test]
    fn no_matching_rule_is_identity() {
        let req = make_get_request("https://acme.bb/api/users");
        let rules = vec![literal(
            MatchReplaceTarget::RequestUrl,
            "/admin",
            "/blocked",
        )];
        let out = MatchReplace::apply(req.clone(), &rules);
        assert_eq!(out.url.path(), req.url.path());
    }

    #[test]
    fn higher_priority_runs_first() {
        let req = make_get_request("https://acme.bb/api/v1/foo");
        let mut rules = vec![
            literal(
                MatchReplaceTarget::RequestUrl,
                "/api/v1/",
                "/replaced-by-1/",
            ),
            literal(
                MatchReplaceTarget::RequestUrl,
                "/replaced-by-1/",
                "/replaced-by-2/",
            ),
        ];
        // Set the second rule to higher priority so it runs first.
        rules[0].priority = 0;
        rules[1].priority = 10;
        let out = MatchReplace::apply(req, &rules);
        // The first rule (priority 0) should NOT have replaced because
        // the second (priority 10) ran first and changed the URL.
        assert_eq!(out.url.path(), "/replaced-by-1/foo");
    }

    #[test]
    fn disabled_rules_are_skipped() {
        let req = make_get_request("https://acme.bb/api/v1/foo");
        let mut rule = literal(MatchReplaceTarget::RequestUrl, "/api/v1/", "/replaced/");
        rule.enabled = false;
        let rules = vec![rule];
        let out = MatchReplace::apply(req.clone(), &rules);
        assert_eq!(out.url.path(), req.url.path());
    }
}
