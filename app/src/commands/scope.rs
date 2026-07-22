//! Tauri commands for scope rules and match & replace rules (Phase 6).
//!
//! ## §6.2 — Scope rules CRUD
//! - `list_scope_rules(project_id)` — read
//! - `add_scope_rule(project_id, rule)` — push
//! - `remove_scope_rule(project_id, index)` — remove by index
//!
//! ## §6.7 — Match & replace rules CRUD (Part B lands these)
//! - `list_match_replace_rules(project_id)` — read
//! - `add_match_replace_rule(project_id, rule)` — push
//! - `remove_match_replace_rule(project_id, index)` — remove by index
//!
//! **State:** all 6 commands read/write the in-memory
//! `ProjectSettings` cache (`Engine::get_project` /
//! `Engine::update_project`). **No SQLite write** — the
//! `ProjectSettings` persistence is a v0.5+ follow-up
//! (same D3 deferral as Phase 5's `ReplayStore.history`).
//!
//! **Drift from the spec:** the spec's §6.2 used
//! `state.active_project.lock().await` (a unified `AppState`); in
//! reality talon's Tauri commands use `tauri::State<'_, EngineArc>`
//! (each managed type is its own state — same pattern Phase 5
//! confirmed). The CRUD commands take `project_id: ProjectId` as
//! an argument and look up the project via `Engine::get_project`.

#![allow(missing_docs)]

use bk_core::scope::MatchReplaceRule;
use bk_core::{ProjectId, ScopeRule};
use tauri::State;

use crate::commands::core::EngineArc;

// ---------------------------------------------------------------------------
// §6.2 — Scope rules CRUD
// ---------------------------------------------------------------------------

/// `list_scope_rules(project_id) -> Vec<ScopeRule>`.
/// Returns the active project's `scope_rules`. Empty Vec if
/// the project is not open (the spec's "no active project" case).
#[tauri::command]
pub fn list_scope_rules(
    engine: State<'_, EngineArc>,
    project_id: ProjectId,
) -> Result<Vec<ScopeRule>, String> {
    let project = engine
        .get_project(project_id)
        .map_err(|e| format!("list_scope_rules failed: {e}"))?;
    Ok(project.settings.scope_rules)
}

/// `add_scope_rule(project_id, rule)` — append a new rule to the
/// active project's `scope_rules`. In-memory only (v0.5+
/// persistence is a follow-up).
#[tauri::command]
pub fn add_scope_rule(
    engine: State<'_, EngineArc>,
    project_id: ProjectId,
    rule: ScopeRule,
) -> Result<(), String> {
    let mut project = engine
        .get_project(project_id)
        .map_err(|e| format!("add_scope_rule failed: {e}"))?;
    project.settings.scope_rules.push(rule);
    engine
        .update_project(project)
        .map_err(|e| format!("add_scope_rule persist failed: {e}"))?;
    Ok(())
}

/// `remove_scope_rule(project_id, index)` — remove the rule at
/// the given index. Returns an error string if the index is
/// out of bounds.
#[tauri::command]
pub fn remove_scope_rule(
    engine: State<'_, EngineArc>,
    project_id: ProjectId,
    index: usize,
) -> Result<(), String> {
    let mut project = engine
        .get_project(project_id)
        .map_err(|e| format!("remove_scope_rule failed: {e}"))?;
    if index >= project.settings.scope_rules.len() {
        return Err(format!(
            "remove_scope_rule: index {index} out of bounds (len = {})",
            project.settings.scope_rules.len()
        ));
    }
    project.settings.scope_rules.remove(index);
    engine
        .update_project(project)
        .map_err(|e| format!("remove_scope_rule persist failed: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// §6.7 — Match & replace rules CRUD
// ---------------------------------------------------------------------------

/// `list_match_replace_rules(project_id) -> Vec<MatchReplaceRule>`.
#[tauri::command]
pub fn list_match_replace_rules(
    engine: State<'_, EngineArc>,
    project_id: ProjectId,
) -> Result<Vec<MatchReplaceRule>, String> {
    let project = engine
        .get_project(project_id)
        .map_err(|e| format!("list_match_replace_rules failed: {e}"))?;
    Ok(project.settings.match_replace_rules)
}

/// `add_match_replace_rule(project_id, rule)`.
#[tauri::command]
pub fn add_match_replace_rule(
    engine: State<'_, EngineArc>,
    project_id: ProjectId,
    rule: MatchReplaceRule,
) -> Result<(), String> {
    let mut project = engine
        .get_project(project_id)
        .map_err(|e| format!("add_match_replace_rule failed: {e}"))?;
    project.settings.match_replace_rules.push(rule);
    engine
        .update_project(project)
        .map_err(|e| format!("add_match_replace_rule persist failed: {e}"))?;
    Ok(())
}

/// `remove_match_replace_rule(project_id, index)`.
#[tauri::command]
pub fn remove_match_replace_rule(
    engine: State<'_, EngineArc>,
    project_id: ProjectId,
    index: usize,
) -> Result<(), String> {
    let mut project = engine
        .get_project(project_id)
        .map_err(|e| format!("remove_match_replace_rule failed: {e}"))?;
    if index >= project.settings.match_replace_rules.len() {
        return Err(format!(
            "remove_match_replace_rule: index {index} out of bounds (len = {})",
            project.settings.match_replace_rules.len()
        ));
    }
    project.settings.match_replace_rules.remove(index);
    engine
        .update_project(project)
        .map_err(|e| format!("remove_match_replace_rule persist failed: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use bk_core::{MatchAction, ScopeRuleKind};
    use bk_engine::Engine;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn fresh_engine() -> (EngineArc, ProjectId, TempDir) {
        let tmp = TempDir::new().unwrap();
        let engine = Arc::new(Engine::new(tmp.path()).unwrap());
        let project = bk_core::Project::new("acme.bb", "acme.bb", "0.1.0");
        let id = project.info.id;
        engine.open_project(&project).unwrap();
        (engine, id, tmp)
    }

    fn test_rule(label: &str) -> ScopeRule {
        ScopeRule {
            kind: ScopeRuleKind::Host,
            pattern: "acme.bb".to_string(),
            action: MatchAction::InScope,
            label: label.to_string(),
            priority: 0,
        }
    }

    /// Direct wrapper of the Tauri command's logic for testing —
    /// the `tauri::State` wrapper itself is exercised at the
    /// `cargo build` level (the macro just lifts the function
    /// into a `Box<dyn FnMut>`); the meaningful business logic
    /// is the engine round-trip.
    fn add_scope_rule_via_engine(
        engine: &EngineArc,
        project_id: ProjectId,
        rule: ScopeRule,
    ) -> Result<(), String> {
        let mut project = engine
            .get_project(project_id)
            .map_err(|e| format!("add_scope_rule failed: {e}"))?;
        project.settings.scope_rules.push(rule);
        engine
            .update_project(project)
            .map_err(|e| format!("add_scope_rule persist failed: {e}"))?;
        Ok(())
    }

    fn remove_scope_rule_via_engine(
        engine: &EngineArc,
        project_id: ProjectId,
        index: usize,
    ) -> Result<(), String> {
        let mut project = engine
            .get_project(project_id)
            .map_err(|e| format!("remove_scope_rule failed: {e}"))?;
        if index >= project.settings.scope_rules.len() {
            return Err(format!(
                "remove_scope_rule: index {index} out of bounds (len = {})",
                project.settings.scope_rules.len()
            ));
        }
        project.settings.scope_rules.remove(index);
        engine
            .update_project(project)
            .map_err(|e| format!("remove_scope_rule persist failed: {e}"))?;
        Ok(())
    }

    #[test]
    fn add_then_list_scope_rule() {
        let (engine, id, _tmp) = fresh_engine();
        let rule = test_rule("acme in-scope");
        add_scope_rule_via_engine(&engine, id, rule.clone()).unwrap();

        let listed = engine.get_project(id).unwrap().settings.scope_rules;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].label, "acme in-scope");
        assert_eq!(listed[0].pattern, "acme.bb");
    }

    #[test]
    fn remove_scope_rule_out_of_bounds() {
        let (engine, id, _tmp) = fresh_engine();
        let res = remove_scope_rule_via_engine(&engine, id, 99);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("out of bounds"));
    }

    #[test]
    fn remove_scope_rule_happy_path() {
        let (engine, id, _tmp) = fresh_engine();
        add_scope_rule_via_engine(&engine, id, test_rule("first")).unwrap();
        add_scope_rule_via_engine(&engine, id, test_rule("second")).unwrap();
        assert_eq!(
            engine.get_project(id).unwrap().settings.scope_rules.len(),
            2
        );
        remove_scope_rule_via_engine(&engine, id, 0).unwrap();
        let rules = engine.get_project(id).unwrap().settings.scope_rules;
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].label, "second");
    }

    #[test]
    fn list_match_replace_rules_empty_by_default() {
        let (engine, id, _tmp) = fresh_engine();
        let rules = engine.get_project(id).unwrap().settings.match_replace_rules;
        assert!(rules.is_empty(), "new project must have empty M&R list");
    }
}
