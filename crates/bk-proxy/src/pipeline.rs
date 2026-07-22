//! Proxy pipeline integration for scope + match & replace (Phase 6 §6.4 + §6.5).
//!
//! **Drift from the spec:** the spec's §6.4 / §6.5 reference
//! `bk_proxy::pipeline::build_exchange` and
//! `bk_proxy::pipeline::forward_to_upstream`; neither exists on
//! `main` (the proxy's MITM-forwarding core lives in
//! `bk_proxy::mitm`, the engine that builds `HttpExchange`s
//! lives in `bk_engine`). The "pipeline" as a single module
//! that owns scope evaluation + M&R application + exchange
//! construction is a v0.5+ follow-up (the proxy ↔ engine
//! capture loop hasn't landed yet).
//!
//! **What this module DOES ship** (the v1 contract per the
//! spec's D3 deferral): two thin wrapper functions that take
//! a `bk_core::Url` and `bk_core::Request` and return the
//! scope-evaluated + M&R-rewritten form. Whoever wires the
//! future capture loop calls these — the engines in
//! `bk_proxy::scope` and `bk_proxy::match_replace` stay
//! decoupled from the pipeline's call site.
//!
//! The `start_proxy` shim in `app/src/commands/core.rs` always
//! passes empty `Vec`s for both (matches the v0.5 "no rules
//! yet" behavior). Wiring the actual rules into proxy startup
//! is the same v0.5+ follow-up.

#![allow(missing_docs)]

use bk_core::scope::{MatchReplaceRule, ScopeRule};
use bk_core::{Request, ScopeState, Url};

use crate::match_replace::MatchReplace;
use crate::scope::Scope;

/// Evaluate the scope rules against a URL and return the resulting
/// `ScopeState`. This is the v1 `build_exchange` shim's
/// scope-classification step: the future pipeline passes the
/// request URL + the project's `scope_rules` and writes the
/// returned `ScopeState` into the `ExchangeMeta.scope_state`
/// field of the `HttpExchange`.
///
/// `rules` is `&[ScopeRule]` — typically `Project::settings::scope_rules`,
/// passed by value at the call site. An empty slice returns
/// `ScopeState::Unscoped` (the spec's v1 default).
pub fn classify(url: &Url, rules: &[ScopeRule]) -> ScopeState {
    Scope::evaluate(url, rules)
}

/// Apply the M&R rules to a request and return the rewritten
/// request. This is the v1 `forward_to_upstream` shim's
/// rewrite step: the future pipeline passes the
/// `bk_core::Request` (after the `http::Request` →
/// `bk_core::Request` round-trip documented in the spec's §6.5
/// "Heads up") and the project's `match_replace_rules`, and
/// uses the returned request as the upstream call's input.
///
/// `rules` is `&[MatchReplaceRule]` — typically
/// `Project::settings::match_replace_rules`, passed by value.
/// An empty slice returns the request unchanged.
pub fn rewrite(request: Request, rules: &[MatchReplaceRule]) -> Request {
    MatchReplace::apply(request, rules)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bk_core::scope::MatchReplaceTarget;
    use bk_core::{Body, HeaderMap, MatchAction, Method, ScopeRuleKind, Version};

    #[test]
    fn classify_empty_rules_returns_unscoped() {
        let url: Url = "https://acme.bb/".parse().unwrap();
        assert_eq!(classify(&url, &[]), ScopeState::Unscoped);
    }

    #[test]
    fn classify_with_one_rule() {
        let url: Url = "https://acme.bb/api".parse().unwrap();
        let rules = vec![ScopeRule {
            kind: ScopeRuleKind::Host,
            pattern: "acme.bb".to_string(),
            action: MatchAction::InScope,
            label: "acme in-scope".to_string(),
            priority: 0,
        }];
        assert_eq!(classify(&url, &rules), ScopeState::InScope);
    }

    #[test]
    fn rewrite_empty_rules_is_identity() {
        let req = Request {
            method: Method::GET,
            url: "https://acme.bb/api/v1/foo".parse().unwrap(),
            version: Version::HTTP_11,
            headers: HeaderMap::new(),
            body: Body::empty(),
        };
        let out = rewrite(req.clone(), &[]);
        assert_eq!(out.url.path(), req.url.path());
    }

    #[test]
    fn rewrite_applies_url_rule() {
        let req = Request {
            method: Method::GET,
            url: "https://acme.bb/api/v1/foo".parse().unwrap(),
            version: Version::HTTP_11,
            headers: HeaderMap::new(),
            body: Body::empty(),
        };
        let rules = vec![MatchReplaceRule {
            target: MatchReplaceTarget::RequestUrl,
            case_insensitive: false,
            is_regex: false,
            pattern: "/api/v1/".to_string(),
            replace: "/api/v2/".to_string(),
            enabled: true,
            priority: 0,
        }];
        let out = rewrite(req, &rules);
        assert_eq!(out.url.path(), "/api/v2/foo");
    }
}
