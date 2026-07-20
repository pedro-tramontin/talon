// Bottom-docked status bar for the active agent run. Renders the
// goal, the latest event message, and (when running) a Cancel
// button. When a `pendingConfirm` is set on the active run, the
// `<ConfirmDialog />` is rendered on top of the bar.

import { useMemo } from "react";
import { agentStore, useAgentStore } from "../state/agent";
import { ConfirmDialog } from "./ConfirmDialog";

/**
 * AgentPanel. Docks to the bottom of the viewport. The component
 * itself is always mounted; it shows a placeholder when no run is
 * active. The data selector is intentionally minimal so the
 * component only re-renders when the slices it actually renders
 * change.
 */
export function AgentPanel() {
  const activeRunId = useAgentStore((s) => s.activeRunId);
  const run = useAgentStore((s) =>
    activeRunId ? (s.runs[activeRunId] ?? null) : null,
  );
  const cancelRun = useAgentStore((s) => s.cancelRun);

  // Derive a short "what's happening" string from the latest event.
  // We `useMemo` so the formatter only runs when the event list
  // actually changes.
  const latestMessage = useMemo(() => {
    if (!run || run.events.length === 0) return "starting…";
    const last = run.events[run.events.length - 1];
    switch (last.event) {
      case "agent_started":
        return `started · model ${last.model}`;
      case "agent_thinking":
        return "thinking…";
      case "agent_tool_call":
        return `tool: ${last.tool_name} → ${last.result_summary}`;
      case "agent_message":
        return last.text;
      case "agent_finished":
        return `finished (${last.iterations} iters)`;
      case "agent_error":
        return `error: ${last.error}`;
    }
  }, [run]);

  if (!activeRunId || !run) {
    return null;
  }

  return (
    <>
      {run.pendingConfirm && activeRunId && (
        <ConfirmDialog
          runId={activeRunId}
          toolName={run.pendingConfirm.toolName}
          args={run.pendingConfirm.args}
        />
      )}
      <div
        data-testid="agent-panel"
        className="fixed bottom-0 left-0 right-0 z-50 border-t border-slate-700 bg-bg-panel px-4 py-2 text-sm text-slate-200 shadow-lg"
      >
        <div className="mx-auto flex max-w-5xl items-center gap-3">
          <span className="shrink-0 rounded bg-bg-rail px-2 py-0.5 text-xs uppercase tracking-wide text-accent">
            {run.status}
          </span>
          <span
            data-testid="agent-panel-goal"
            className="shrink-0 max-w-[40ch] truncate text-slate-400"
            title={run.goal}
          >
            {run.goal}
          </span>
          <span
            data-testid="agent-panel-latest"
            className="flex-1 truncate"
            title={latestMessage}
          >
            {latestMessage}
          </span>
          {run.status === "running" && (
            <button
              data-testid="agent-panel-cancel"
              type="button"
              onClick={() => {
                void cancelRun(activeRunId);
              }}
              className="shrink-0 rounded border border-scope-blocked bg-transparent px-2 py-0.5 text-xs uppercase tracking-wide text-scope-blocked hover:bg-scope-blocked hover:text-white"
            >
              Cancel
            </button>
          )}
        </div>
      </div>
    </>
  );
}

// Re-export the store so tests can poke at it directly without
// importing a separate path.
export { agentStore };
