// Scope rule editor. Phase 6 §6.6.
//
// Sits at the bottom of the left rail in the Capture route
// (the same `aside` that wraps `<ExchangeList />`). Lists
// the active project's scope rules, lets the user add a
// new default rule and remove existing rules by index. All
// mutations round-trip through the Tauri commands
// (`listScopeRules`, `addScopeRule`, `removeScopeRule`)
// defined in `app/src/commands/scope.rs`; the local store
// (`useUiStore`) is the optimistic-update source of truth
// that the component reads from.
//
// The "active project" is read from `useProjectStore` —
// if no project is open, the editor renders an empty state
// with the same "No rules" hint (the backend would have
// returned an error on the IPC call; we degrade gracefully).

import { useEffect } from "react";
import { useProjectStore } from "../state/project";
import { useUiStore } from "../state/ui";
import {
  addScopeRule,
  listScopeRules,
  removeScopeRule,
} from "../api";
import type { MatchAction, ScopeRule, ScopeRuleKind } from "../types/domain";

/**
 * The colour class for the action chip on each rule row.
 * Matches the spec's §6.6 requirement:
 *   - in_scope     → green
 *   - out_of_scope → slate
 *   - block        → red
 */
function actionChipClass(action: MatchAction): string {
  switch (action) {
    case "in_scope":
      return "bg-green-900/40 text-green-300";
    case "out_of_scope":
      return "bg-slate-700 text-slate-300";
    case "block":
      return "bg-red-900/40 text-red-300";
    default:
      // `MatchAction` is `#[non_exhaustive]` on the Rust side;
      // unknown v2 actions get a neutral chip rather than
      // crashing the row.
      return "bg-slate-800 text-slate-400";
  }
}

export function ScopeRuleEditor() {
  const scopeRules = useUiStore((s) => s.scopeRules);
  const setScopeRules = useUiStore((s) => s.setScopeRules);
  const activeProjectId = useProjectStore((s) => s.activeProjectId);

  // Pull the active project's rules on mount + when the
  // active project changes. The `useProjectStore` change
  // triggers the re-fetch (the same project reopen also
  // re-fetches because `activeProjectId` is a dep).
  useEffect(() => {
    if (!activeProjectId) {
      setScopeRules([]);
      return;
    }
    let cancelled = false;
    listScopeRules(activeProjectId)
      .then((rules) => {
        if (!cancelled) setScopeRules(rules);
      })
      .catch((e) => {
        // Backend error (e.g. project not open) — degrade
        // silently; the empty state shows the "No rules" hint.
        // Surfacing the error would be noisy for v0.1.
        if (!cancelled) {
          console.error("listScopeRules failed:", e);
          setScopeRules([]);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [activeProjectId, setScopeRules]);

  const addNew = async () => {
    if (!activeProjectId) return;
    const rule: ScopeRule = {
      kind: "host",
      pattern: "",
      action: "in_scope",
      label: "new rule",
      priority: 0,
    };
    try {
      await addScopeRule(activeProjectId, rule);
      setScopeRules([...scopeRules, rule]);
    } catch (e) {
      console.error("addScopeRule failed:", e);
    }
  };

  return (
    <div
      data-testid="scope-rule-editor"
      className="mt-6 border-t border-slate-800 pt-3"
    >
      <div className="mb-2 flex items-center justify-between">
        <h2
          data-testid="scope-rule-editor-title"
          className="text-xs font-bold uppercase text-slate-400"
        >
          Scope rules
        </h2>
        <button
          data-testid="scope-rule-editor-add"
          onClick={addNew}
          disabled={!activeProjectId}
          className="text-xs text-accent hover:text-cyan-300 disabled:opacity-40"
        >
          + Add
        </button>
      </div>
      <div
        data-testid="scope-rule-editor-list"
        className="space-y-1"
      >
        {scopeRules.length === 0 ? (
          <p
            data-testid="scope-rule-editor-empty"
            className="text-xs italic text-slate-500"
          >
            {activeProjectId
              ? "No rules. Add one above."
              : "No project open."}
          </p>
        ) : (
          scopeRules.map((r, i) => (
            <div
              key={i}
              data-testid={`scope-rule-row-${i}`}
              className="flex items-center gap-1 text-xs"
            >
              <span
                data-testid={`scope-rule-row-action-${i}`}
                className={`rounded px-1 font-mono ${actionChipClass(r.action)}`}
              >
                {r.action}
              </span>
              <span className="font-mono text-slate-500">{r.kind}</span>
              <span
                data-testid={`scope-rule-row-pattern-${i}`}
                className="flex-1 truncate font-mono text-slate-300"
              >
                {r.pattern || "(empty)"}
              </span>
              <button
                data-testid={`scope-rule-row-remove-${i}`}
                onClick={async () => {
                  if (!activeProjectId) return;
                  try {
                    await removeScopeRule(activeProjectId, i);
                    setScopeRules(
                      scopeRules.filter((_, j) => j !== i),
                    );
                  } catch (e) {
                    console.error("removeScopeRule failed:", e);
                  }
                }}
                className="text-slate-500 hover:text-red-400"
                aria-label="Remove rule"
              >
                ×
              </button>
            </div>
          ))
        )}
      </div>
    </div>
  );
}

// Re-export the type so the test file can import it from here too.
export type { ScopeRule, ScopeRuleKind };
