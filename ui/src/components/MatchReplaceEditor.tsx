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
//
// ## Phase 7 C-B.2 — "Test" button (URL preview)
//
// A new "Test" section above the table: the user types a
// sample URL + clicks "Test" + sees the URL after the
// active rules have been applied (via the JS-side
// `match_replace.ts` engine). The test runs client-side —
// no Tauri round-trip for what is a UI affordance. The
// engine is a JS mirror of the Rust `MatchReplace::apply`,
// not the source of truth; the wire behavior is the
// Rust engine's. The "Test" button is a best-effort UI
// preview.

import { useEffect, useState } from "react";
import { useProjectStore } from "../state/project";
import { useUiStore } from "../state/ui";
import {
  addMatchReplaceRule,
  listMatchReplaceRules,
  removeMatchReplaceRule,
} from "../api";
import { matchReplaceApplyUrl } from "../lib/match_replace";
import type { MatchReplaceRule } from "../types/domain";

export function MatchReplaceEditor() {
  const rules = useUiStore((s) => s.matchReplaceRules);
  const setRules = useUiStore((s) => s.setMatchReplaceRules);
  const activeProjectId = useProjectStore((s) => s.activeProjectId);

  // Phase 7 C-B.2: "Test" button state. The sample URL
  // and the result of running it through the engine. The
  // result is `null` until the user clicks "Test" (or
  // until the rules change after a click — see the
  // `useEffect` below).
  const [testUrl, setTestUrl] = useState("");
  const [testResult, setTestResult] = useState<string | null>(null);

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

  // Phase 7 C-B.2: when the rules change after a test
  // was clicked, the result is stale. Re-run the engine
  // so the preview tracks the rules. If the user hasn't
  // clicked Test yet, the result stays `null` (no
  // preview until the user opts in).
  //
  // `testResult` is intentionally excluded from the deps:
  // including it would re-run the engine every time the
  // result string is recomputed (infinite setState loop).
  useEffect(() => {
    if (testResult !== null) {
      setTestResult(matchReplaceApplyUrl(testUrl, rules));
    }
  }, [rules, testUrl]);

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
      {/* Phase 7 C-B.2: the "Test" section. The user
          types a sample URL + clicks "Test" + sees the
          URL after the active rules have been applied.
          The button is disabled until the user enters
          a URL. */}
      <div
        data-testid="match-replace-editor-test"
        className="mb-3 flex flex-col gap-1 rounded border border-slate-700 p-2"
      >
        <label className="text-xs text-slate-400">
          Test a URL against the rules above:
        </label>
        <div className="flex gap-1">
          <input
            data-testid="match-replace-editor-test-url"
            type="text"
            value={testUrl}
            onChange={(e) => setTestUrl(e.target.value)}
            placeholder="https://example.com/api/v1/users"
            className="flex-1 rounded border border-slate-700 bg-slate-900 px-2 py-1 font-mono text-xs text-slate-200"
          />
          <button
            data-testid="match-replace-editor-test-button"
            onClick={() => {
              if (testUrl === "") {
                setTestResult(null);
                return;
              }
              setTestResult(matchReplaceApplyUrl(testUrl, rules));
            }}
            disabled={testUrl === ""}
            className="rounded border border-slate-700 bg-slate-800 px-3 py-1 text-xs text-slate-200 hover:bg-slate-700 disabled:opacity-40"
          >
            Test
          </button>
        </div>
        {testResult !== null && (
          <div
            data-testid="match-replace-editor-test-result"
            className="rounded bg-slate-900 px-2 py-1 font-mono text-xs text-slate-300"
          >
            {testResult}
          </div>
        )}
        {testResult === null && testUrl !== "" && (
          <div className="text-xs italic text-slate-500">
            Click Test to preview the result.
          </div>
        )}
      </div>
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
