// Modal dialog for write-tool confirmation. In v0.1 the "type
// DELETE" hard-confirm for `talon_delete_exchange` is hard-coded
// and NOT disableable; future phases may add a "dangerous tools"
// allowlist in settings to skip the second prompt.
//
// The dialog calls `respondConfirm` from the agent store; the
// store clears the pending confirmation optimistically and forwards
// the choice to the Rust side.

import { useState } from "react";
import { useAgentStore } from "../state/agent";

/** Tool names that require the hard-coded "type DELETE" second confirm. */
const DESTRUCTIVE_TOOLS: ReadonlySet<string> = new Set([
  "talon_delete_exchange",
]);

/** The literal string the user has to type for a destructive tool. */
const DESTRUCTIVE_PHRASE = "DELETE";

export interface ConfirmDialogProps {
  runId: string;
  toolName: string;
  args: unknown;
}

/**
 * ConfirmDialog. A fixed overlay (`position: fixed; inset: 0`) above
 * the `AgentPanel`. The body shows the tool name and pretty-printed
 * args; the footer has an "Allow all from this agent run" checkbox
 * and Allow / Deny buttons. For tools in `DESTRUCTIVE_TOOLS`, an
 * extra text input requires the user to type `DELETE` before Allow
 * enables.
 */
export function ConfirmDialog({ runId, toolName, args }: ConfirmDialogProps) {
  const respondConfirm = useAgentStore((s) => s.respondConfirm);
  const [remember, setRemember] = useState(false);
  const [confirmText, setConfirmText] = useState("");

  const isDestructive = DESTRUCTIVE_TOOLS.has(toolName);
  // Allow only when the destructive confirm is satisfied (or not
  // required for this tool). The phrase is case-sensitive in v0.1
  // so a user who copy-pastes "Delete" still has to type the
  // uppercase form.
  const allowEnabled = !isDestructive || confirmText === DESTRUCTIVE_PHRASE;

  return (
    <div
      data-testid="confirm-dialog"
      role="dialog"
      aria-modal="true"
      aria-label="Confirm agent action"
      className="fixed inset-0 z-[60] flex items-center justify-center bg-black/50"
    >
      <div className="w-full max-w-lg rounded border border-slate-700 bg-bg-panel p-4 text-slate-100 shadow-2xl">
        <h2 className="mb-2 text-lg font-semibold text-accent">
          Confirm agent action
        </h2>
        <p className="mb-3 text-sm text-slate-300">
          The agent wants to call{" "}
          <code
            data-testid="confirm-dialog-tool-name"
            className="rounded bg-bg-rail px-1 py-0.5 text-accent"
          >
            {toolName}
          </code>{" "}
          with:
        </p>
        <pre
          data-testid="confirm-dialog-args"
          className="mb-3 max-h-48 overflow-auto rounded bg-bg-rail p-2 text-xs text-slate-200"
        >
          {JSON.stringify(args, null, 2)}
        </pre>

        {isDestructive && (
          <div className="mb-3">
            <label
              htmlFor="confirm-dialog-destructive-input"
              className="mb-1 block text-sm text-scope-blocked"
            >
              This is a destructive action. Type{" "}
              <code className="rounded bg-bg-rail px-1">{DESTRUCTIVE_PHRASE}</code>{" "}
              to confirm:
            </label>
            <input
              id="confirm-dialog-destructive-input"
              data-testid="confirm-dialog-destructive-input"
              type="text"
              value={confirmText}
              onChange={(e) => setConfirmText(e.target.value)}
              className="w-full rounded border border-slate-700 bg-bg-base px-2 py-1 text-sm text-slate-100 focus:border-accent focus:outline-none"
            />
          </div>
        )}

        <label className="mb-4 flex items-center gap-2 text-sm text-slate-300">
          <input
            type="checkbox"
            data-testid="confirm-dialog-remember"
            checked={remember}
            onChange={(e) => setRemember(e.target.checked)}
            className="h-4 w-4 accent-accent"
          />
          Allow all from this agent run
        </label>

        <div className="flex justify-end gap-2">
          <button
            type="button"
            data-testid="confirm-dialog-deny"
            onClick={() => {
              void respondConfirm(runId, false, remember);
            }}
            className="rounded border border-slate-600 bg-transparent px-3 py-1 text-sm text-slate-200 hover:border-slate-400"
          >
            Deny
          </button>
          <button
            type="button"
            data-testid="confirm-dialog-allow"
            disabled={!allowEnabled}
            onClick={() => {
              void respondConfirm(runId, true, remember);
            }}
            className="rounded bg-accent px-3 py-1 text-sm font-semibold text-bg-base hover:bg-accent-muted disabled:cursor-not-allowed disabled:bg-slate-700 disabled:text-slate-500"
          >
            Allow
          </button>
        </div>
      </div>
    </div>
  );
}
