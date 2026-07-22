//! End-to-end smoke test for the scope + match & replace engines
//! (Phase 6 §6.8).
//!
//! These are integration tests at the `bk-proxy` crate boundary:
//! they exercise the public `bk_proxy::scope` and
//! `bk_proxy::match_replace` APIs with realistic rule sets, the
//! same way the engine / UI would call them.
//!
//! The two tests in the §6.8 spec:
//! 1. `scope_engine_end_to_end` — priority-wins: a generic
//!    out-of-scope rule on `/api/` loses to a more-specific
//!    in-scope rule on `/api/v2/` for `/api/v2/users`, and wins
//!    for `/api/v1/users`.
//! 2. `match_replace_end_to_end` — URL rewrite: a literal
//!    `/api/v1/` → `/api/v2/` swap changes the path from
//!    `/api/v1/foo` to `/api/v2/foo`.

use bk_core::scope::{MatchReplaceRule, MatchReplaceTarget};
use bk_core::{
    Body, HeaderMap, MatchAction, Method, Request, ScopeRule, ScopeRuleKind, ScopeState, Url,
    Version,
};
use bk_proxy::match_replace::MatchReplace;
use bk_proxy::scope::Scope;

#[test]
fn scope_engine_end_to_end() {
    // An out-of-scope rule on a path, plus a more-specific in-scope
    // rule on a deeper path. Higher-priority wins.
    let rules = vec![
        ScopeRule {
            kind: ScopeRuleKind::PathPrefix,
            pattern: "/api/".to_string(),
            action: MatchAction::OutOfScope,
            label: "API is out".into(),
            priority: 0,
        },
        ScopeRule {
            kind: ScopeRuleKind::PathPrefix,
            pattern: "/api/v2/".to_string(),
            action: MatchAction::InScope,
            label: "v2 is in".into(),
            priority: 10,
        },
    ];
    let url: Url = "https://acme.bb/api/v2/users".parse().unwrap();
    let state = Scope::evaluate(&url, &rules);
    assert_eq!(
        state,
        ScopeState::InScope,
        "v2 should win over generic /api/"
    );

    let url: Url = "https://acme.bb/api/v1/users".parse().unwrap();
    let state = Scope::evaluate(&url, &rules);
    assert_eq!(
        state,
        ScopeState::OutOfScope,
        "v1 should fall through to /api/"
    );
}

#[test]
fn match_replace_end_to_end() {
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
    let out = MatchReplace::apply(req, &rules);
    assert_eq!(out.url.path(), "/api/v2/foo");
}
