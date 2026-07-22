//! Scope engine. Evaluates a URL against a list of rules and returns
//! the matched action. Higher-priority rules win; ties are broken by
//! the order in which rules were added (first wins).
//!
//! Phase 6 (§6.1). The rule types themselves (`ScopeRule`,
//! `MatchAction`, `ScopeRuleKind`, `ScopeState`) live in
//! `bk_core::scope`; this module is the *engine* that evaluates them.
//! The engine is a pure function — no I/O, no state, no async — so
//! the UI's debounced editor can call it on every keystroke to
//! preview the resulting state without standing up a Tauri command.

use bk_core::{MatchAction, ScopeRule, ScopeState, Url};

pub struct Scope;

impl Scope {
    /// Evaluate the rules in order, return the action of the highest-priority
    /// matching rule. If no rule matches, return `Unscoped`.
    ///
    /// **Tie-breaking:** when two rules match with the same priority,
    /// the *first* declared wins (deterministic from the input order).
    /// The implementation walks the rules in order and only replaces
    /// the current best when a strictly higher priority is seen — so
    /// the first rule at a given priority level "wins the tie".
    pub fn evaluate(url: &Url, rules: &[ScopeRule]) -> ScopeState {
        let mut best_priority: Option<i32> = None;
        let mut best_action: Option<MatchAction> = None;

        for rule in rules {
            if !rule.matches(url) {
                continue;
            }
            let is_better = match best_priority {
                None => true,
                Some(p) => rule.priority > p,
            };
            if is_better {
                best_priority = Some(rule.priority);
                best_action = Some(rule.action);
            }
        }

        // `MatchAction` is `#[non_exhaustive]` (per the Phase 10
        // plugin-system design contract) so a wildcard is required
        // for forward-compat. New variants in v2 map to `Unscoped`
        // for v1 callers — the safe default until a v2 explicit
        // match lands.
        match best_action {
            Some(MatchAction::InScope) => ScopeState::InScope,
            Some(MatchAction::OutOfScope) => ScopeState::OutOfScope,
            Some(MatchAction::Block) => ScopeState::Blocked,
            Some(_) | None => ScopeState::Unscoped,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bk_core::ScopeRule;
    use bk_core::ScopeRuleKind;
    use std::str::FromStr;

    fn url(s: &str) -> Url {
        Url::from_str(s).unwrap()
    }

    fn rule(kind: ScopeRuleKind, pattern: &str, action: MatchAction, priority: i32) -> ScopeRule {
        ScopeRule {
            kind,
            pattern: pattern.to_string(),
            action,
            label: "test".into(),
            priority,
        }
    }

    #[test]
    fn no_rules_returns_unscoped() {
        let state = Scope::evaluate(&url("https://acme.bb/api"), &[]);
        assert_eq!(state, ScopeState::Unscoped);
    }

    #[test]
    fn single_in_scope_rule_matches() {
        let rules = vec![rule(
            ScopeRuleKind::Host,
            "acme.bb",
            MatchAction::InScope,
            0,
        )];
        let state = Scope::evaluate(&url("https://acme.bb/api"), &rules);
        assert_eq!(state, ScopeState::InScope);
    }

    #[test]
    fn out_of_scope_takes_priority() {
        let rules = vec![
            rule(ScopeRuleKind::Host, "acme.bb", MatchAction::InScope, 0),
            rule(
                ScopeRuleKind::PathPrefix,
                "/api/admin",
                MatchAction::OutOfScope,
                10,
            ),
        ];
        // The higher-priority rule wins.
        let state = Scope::evaluate(&url("https://acme.bb/api/admin/users"), &rules);
        assert_eq!(state, ScopeState::OutOfScope);
    }

    #[test]
    fn block_action_yields_blocked_state() {
        let rules = vec![rule(
            ScopeRuleKind::Host,
            "*.analytics.com",
            MatchAction::Block,
            0,
        )];
        let state = Scope::evaluate(&url("https://track.analytics.com/pixel"), &rules);
        assert_eq!(state, ScopeState::Blocked);
    }

    #[test]
    fn non_matching_rule_is_ignored() {
        let rules = vec![rule(
            ScopeRuleKind::Host,
            "other.com",
            MatchAction::InScope,
            100,
        )];
        let state = Scope::evaluate(&url("https://acme.bb/api"), &rules);
        assert_eq!(state, ScopeState::Unscoped);
    }

    #[test]
    fn equal_priority_uses_first_declared() {
        let rules = vec![
            rule(ScopeRuleKind::Host, "acme.bb", MatchAction::InScope, 5),
            rule(ScopeRuleKind::Host, "acme.bb", MatchAction::OutOfScope, 5),
        ];
        // Both have priority 5; the first (InScope) wins.
        let state = Scope::evaluate(&url("https://acme.bb/"), &rules);
        assert_eq!(state, ScopeState::InScope);
    }
}
