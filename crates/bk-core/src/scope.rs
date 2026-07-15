#![allow(missing_docs)]
//! Scope rules and match & replace. A `ScopeRule` decides whether a given
//! URL is in-scope, out-of-scope, or blocked. A `MatchReplaceRule`
//! rewrites a request or response before it reaches the wire.

use serde::{Deserialize, Serialize};

/// What to do with a request that matches a `ScopeRule`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchAction {
    /// Allow the request; mark the resulting exchange as in-scope.
    InScope,
    /// Allow the request; mark the resulting exchange as out-of-scope
    /// (so the UI shows it grayed out and a fuzzer won't auto-target it).
    OutOfScope,
    /// Do not send the request; return a synthetic response to the
    /// browser indicating the request was blocked.
    Block,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeRuleKind {
    /// Matches if the URL's host is exactly this string (case-insensitive).
    /// `*.example.com` syntax is supported: prefix with `*.`.
    Host,
    /// Matches if the URL's path starts with this prefix.
    PathPrefix,
    /// Matches if the URL's path matches this regex (case-sensitive by
    /// default; the `(?i)` flag opts into case-insensitive).
    PathRegex,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScopeRule {
    pub kind: ScopeRuleKind,
    pub pattern: String,
    pub action: MatchAction,
    /// Human-readable label, e.g. "Acme in-scope", "CDN out-of-scope",
    /// "Analytics block". Used in the UI's scope list.
    pub label: String,
    /// When multiple rules match, the one with the highest priority wins.
    /// Default 0; users can bump specific rules to override.
    pub priority: i32,
}

impl ScopeRule {
    /// Decide whether this rule applies to a given URL.
    ///
    /// For `Host`, the pattern is matched against the host with a
    /// case-insensitive equality check; a leading `*.` is a wildcard
    /// that matches the host and any subdomain.
    ///
    /// For `PathPrefix`, an exact case-sensitive starts-with check.
    ///
    /// For `PathRegex`, the pattern is treated as a regex.
    pub fn matches(&self, url: &crate::Url) -> bool {
        match self.kind {
            ScopeRuleKind::Host => {
                let host = url.host_str().unwrap_or("").to_ascii_lowercase();
                let pat = self.pattern.to_ascii_lowercase();
                if let Some(suffix) = pat.strip_prefix("*.") {
                    host == suffix || host.ends_with(&format!(".{}", suffix))
                } else {
                    host == pat
                }
            }
            ScopeRuleKind::PathPrefix => url.path().starts_with(&self.pattern),
            ScopeRuleKind::PathRegex => {
                // Best-effort: if the regex is malformed, it doesn't match.
                // We don't `unwrap()` because a single bad user rule
                // shouldn't crash the whole proxy.
                match regex_shim::Regex::new(&self.pattern) {
                    Ok(re) => re.is_match(url.path()),
                    Err(_) => false,
                }
            }
        }
    }
}

/// A single match & replace rule. Operates on the raw request/response
/// before/after it hits the wire.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchReplaceRule {
    /// What part of the message to match against.
    pub target: MatchReplaceTarget,
    /// Whether the match is case-sensitive. Defaults to false.
    pub case_insensitive: bool,
    /// Whether `pattern` is a literal string or a regex.
    pub is_regex: bool,
    pub pattern: String,
    /// Replacement string. If `is_regex` is true, supports `$1`-style
    /// backreferences (delegated to the regex engine).
    pub replace: String,
    /// Disabled rules are kept around but not evaluated. Lets users
    /// toggle without deleting.
    pub enabled: bool,
    /// Higher priority runs first.
    pub priority: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchReplaceTarget {
    RequestUrl,
    RequestHeader,
    RequestBody,
    ResponseHeader,
    ResponseBody,
}

// Wrapper that exposes only `is_match` and `new` from the `regex` crate.
// This intentionally limits the API surface so callers in `bk-core` can
// only do whole-string matching — no capture groups, no replacement, no
// iteration. The full `regex` crate is used in `bk-store` and `bk-engine`
// for the more demanding match/replace logic; we still depend on it
// here, so the size cost is unchanged. The shim exists to make the
// "bk-core is match-only" constraint a compile-time boundary.
mod regex_shim {
    pub struct Regex(regex::Regex);
    impl Regex {
        pub fn new(pat: &str) -> Result<Self, regex::Error> {
            Ok(Self(regex::Regex::new(pat)?))
        }
        pub fn is_match(&self, s: &str) -> bool {
            self.0.is_match(s)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> crate::Url {
        s.parse().unwrap()
    }

    #[test]
    fn host_rule_matches_exact_host_case_insensitive() {
        let r = ScopeRule {
            kind: ScopeRuleKind::Host,
            pattern: "Acme.BB".to_string(),
            action: MatchAction::InScope,
            label: "test".into(),
            priority: 0,
        };
        assert!(r.matches(&url("https://acme.bb/api")));
        assert!(!r.matches(&url("https://other.com/api")));
    }

    #[test]
    fn host_rule_with_wildcard_matches_subdomains() {
        let r = ScopeRule {
            kind: ScopeRuleKind::Host,
            pattern: "*.acme.bb".to_string(),
            action: MatchAction::InScope,
            label: "test".into(),
            priority: 0,
        };
        assert!(r.matches(&url("https://api.acme.bb/x")));
        assert!(r.matches(&url("https://acme.bb/x"))); // bare apex also matches
        assert!(!r.matches(&url("https://evil.com/x")));
    }

    #[test]
    fn path_prefix_rule_matches_starts_with() {
        let r = ScopeRule {
            kind: ScopeRuleKind::PathPrefix,
            pattern: "/api/".to_string(),
            action: MatchAction::InScope,
            label: "test".into(),
            priority: 0,
        };
        assert!(r.matches(&url("https://acme.bb/api/users")));
        assert!(!r.matches(&url("https://acme.bb/static/app.js")));
    }

    #[test]
    fn path_regex_rule_matches_against_path() {
        let r = ScopeRule {
            kind: ScopeRuleKind::PathRegex,
            pattern: r"^/users/\d+$".to_string(),
            action: MatchAction::InScope,
            label: "test".into(),
            priority: 0,
        };
        assert!(r.matches(&url("https://acme.bb/users/42")));
        assert!(!r.matches(&url("https://acme.bb/users/abc")));
        assert!(!r.matches(&url("https://acme.bb/users/42/extra")));
    }

    #[test]
    fn invalid_regex_does_not_panic() {
        let r = ScopeRule {
            kind: ScopeRuleKind::PathRegex,
            pattern: "[unclosed".to_string(),
            action: MatchAction::InScope,
            label: "test".into(),
            priority: 0,
        };
        // Should not match anything AND not crash.
        assert!(!r.matches(&url("https://acme.bb/")));
    }
}
