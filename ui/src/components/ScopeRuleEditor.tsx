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
//
// ## Phase 7 C-B.4 — SecLists bulk-import
//
// A "Bulk import" button next to the existing "+ Add"
// button. The user picks a `.txt` / `.lst` file; the
// component reads the file via `FileReader.readAsText`
// (no Tauri command — client-side only), parses with
// `parseSecListsHosts` (the pure helper in
// `../lib/scope_bulk_import.ts`), and adds each parsed
// host as a `ScopeRule { kind: Host, pattern: <host>,
// action: InScope, label: "imported", priority: 0 }`
// via the existing `addScopeRule` Tauri command.
//
// The parser drops comments (`#` and `//`), blank lines,
// wildcard lines (`*.` prefix; the per-row editor
// handles wildcards), and duplicates against the
// existing rules array.

import { useEffect, useRef, useState } from "react";
import { useProjectStore } from "../state/project";
import { useUiStore } from "../state/ui";
import {
  addScopeRule,
  listScopeRules,
  removeScopeRule,
} from "../api";
import { parseSecListsHosts } from "../lib/scope_bulk_import";
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

  // Phase 7 C-B.4: bulk-import state. The `fileInputRef`
  // is a hidden `<input type="file">`; clicking the
  // "Bulk import" button triggers `.click()` on it.
  // The `bulkImportError` is shown below the button on
  // failure (a 0-hosts file, a FileReader error, etc.).
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const [bulkImportError, setBulkImportError] = useState<string | null>(
    null,
  );

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

  // Phase 7 C-B.4: bulk-import handler. Reads the file,
  // parses it, and calls `addScopeRule` for each parsed
  // host. Optimistic local update — the new rules are
  // appended to the store immediately.
  const handleBulkImport = async (
    e: React.ChangeEvent<HTMLInputElement>,
  ) => {
    if (!activeProjectId) return;
    const file = e.target.files?.[0];
    if (!file) return;
    setBulkImportError(null);
    try {
      const text = await file.text();
      const parsed = parseSecListsHosts(text, scopeRules);
      if (parsed.length === 0) {
        setBulkImportError("No hosts found in file.");
        return;
      }
      const newRules: ScopeRule[] = parsed.map(({ host }) => ({
        kind: "host",
        pattern: host,
        action: "in_scope",
        label: "imported",
        priority: 0,
      }));
      // Fire `addScopeRule` for each rule in sequence
      // (avoids hammering the IPC bridge on a 1000-line
      // file; the rules are an append-mostly log).
      const appended: ScopeRule[] = [];
      for (const rule of newRules) {
        try {
          await addScopeRule(activeProjectId, rule);
          appended.push(rule);
        } catch (err) {
          console.error("addScopeRule failed during bulk-import:", err);
        }
      }
      setScopeRules([...scopeRules, ...appended]);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setBulkImportError(`Bulk import failed: ${msg}`);
    } finally {
      // Reset the input so the same file can be re-picked
      // (e.g. after an error) — `change` doesn't fire if
      // the user re-selects the same file otherwise.
      if (fileInputRef.current) fileInputRef.current.value = "";
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
        <div className="flex gap-2">
          <button
            data-testid="scope-rule-editor-add"
            onClick={addNew}
            disabled={!activeProjectId}
            className="text-xs text-accent hover:text-cyan-300 disabled:opacity-40"
          >
            + Add
          </button>
          <button
            data-testid="scope-rule-editor-bulk-import"
            onClick={() => fileInputRef.current?.click()}
            disabled={!activeProjectId}
            className="text-xs text-accent hover:text-cyan-300 disabled:opacity-40"
          >
            Bulk import
          </button>
          <input
            ref={fileInputRef}
            data-testid="scope-rule-editor-bulk-import-file"
            type="file"
            accept=".txt,.lst"
            onChange={handleBulkImport}
            className="hidden"
          />
        </div>
      </div>
      {bulkImportError && (
        <p
          data-testid="scope-rule-editor-bulk-import-error"
          className="mb-2 text-xs text-red-400"
        >
          {bulkImportError}
        </p>
      )}
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
