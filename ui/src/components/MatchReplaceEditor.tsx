// Match & Replace editor. Phase 6 §6.7.
//
// Embedded inside the `<SettingsModal>` (Phase 6 §6.7).
// Renders the active project's M&R rules in a 7-column
// table: target, pattern, replace, is_regex, priority,
// enabled, remove. Adding a new rule generates a default
// with an empty pattern + enabled; the user can edit
// pattern / replace / toggles / priority / target in
// subsequent UI (the v0.1 form fields are read-only
// placeholders for the future v0.5 "edit a row" UI — the
// spec's lines 813-838 only require the CRUD surface).
//
// The "active project" is read from `useProjectStore` —
// the M&R editor degrades gracefully when no project is
// open (the table renders an empty row, the Add button
// is disabled).

import { useEffect } from "react";
import { useProjectStore } from "../state/project";
import { useUiStore } from "../state/ui";
import {
  addMatchReplaceRule,
  listMatchReplaceRules,
  removeMatchReplaceRule,
} from "../api";
import type { MatchReplaceRule } from "../types/domain";

export function MatchReplaceEditor() {
  const rules = useUiStore((s) => s.matchReplaceRules);
  const setRules = useUiStore((s) => s.setMatchReplaceRules);
  const activeProjectId = useProjectStore((s) => s.activeProjectId);

  // Re-fetch when the active project changes. Same
  // shape as the `ScopeRuleEditor` effect; the rules
  // are reset to `[]` when no project is open so the
  // table doesn't show stale rules from a closed
  // project.
  useEffect(() => {
    if (!activeProjectId) {
      setRules([]);
      return;
    }
    let cancelled = false;
    listMatchReplaceRules(activeProjectId)
      .then((r) => {
        if (!cancelled) setRules(r);
      })
      .catch((e) => {
        if (!cancelled) {
          console.error("listMatchReplaceRules failed:", e);
          setRules([]);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [activeProjectId, setRules]);

  const add = async () => {
    if (!activeProjectId) return;
    const rule: MatchReplaceRule = {
      target: "request_url",
      case_insensitive: false,
      is_regex: false,
      pattern: "",
      replace: "",
      enabled: true,
      priority: 0,
    };
    try {
      await addMatchReplaceRule(activeProjectId, rule);
      setRules([...rules, rule]);
    } catch (e) {
      console.error("addMatchReplaceRule failed:", e);
    }
  };

  return (
    <div data-testid="match-replace-editor">
      <button
        data-testid="match-replace-editor-add"
        onClick={add}
        disabled={!activeProjectId}
        className="mb-2 text-xs text-accent hover:text-cyan-300 disabled:opacity-40"
      >
        + Add rule
      </button>
      <table className="w-full text-xs">
        <thead>
          <tr className="text-left text-slate-400">
            <th className="py-1">Target</th>
            <th>Pattern</th>
            <th>Replace</th>
            <th>Regex</th>
            <th>Pri</th>
            <th>Enabled</th>
            <th></th>
          </tr>
        </thead>
        <tbody data-testid="match-replace-editor-tbody">
          {rules.length === 0 ? (
            <tr>
              <td
                colSpan={7}
                data-testid="match-replace-editor-empty"
                className="py-2 text-center italic text-slate-500"
              >
                {activeProjectId
                  ? "No rules. Add one above."
                  : "No project open."}
              </td>
            </tr>
          ) : (
            rules.map((r, i) => (
              <tr
                key={i}
                data-testid={`match-replace-row-${i}`}
                className="border-t border-slate-700"
              >
                <td
                  data-testid={`match-replace-row-target-${i}`}
                  className="py-1 font-mono"
                >
                  {r.target}
                </td>
                <td
                  data-testid={`match-replace-row-pattern-${i}`}
                  className="font-mono text-slate-300"
                >
                  {r.pattern || "(empty)"}
                </td>
                <td
                  data-testid={`match-replace-row-replace-${i}`}
                  className="font-mono text-slate-300"
                >
                  {r.replace || "(empty)"}
                </td>
                <td>{r.is_regex ? "✓" : ""}</td>
                <td>{r.priority}</td>
                <td>{r.enabled ? "✓" : ""}</td>
                <td>
                  <button
                    data-testid={`match-replace-row-remove-${i}`}
                    onClick={async () => {
                      if (!activeProjectId) return;
                      try {
                        await removeMatchReplaceRule(activeProjectId, i);
                        setRules(rules.filter((_, j) => j !== i));
                      } catch (e) {
                        console.error("removeMatchReplaceRule failed:", e);
                      }
                    }}
                    className="text-slate-500 hover:text-red-400"
                    aria-label="Remove rule"
                  >
                    ×
                  </button>
                </td>
              </tr>
            ))
          )}
        </tbody>
      </table>
    </div>
  );
}
