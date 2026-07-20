import { useEffect, useState } from "react";
import { greet, type Greeting } from "./api";
import { AgentPanel } from "./components/AgentPanel";
import {
  agentStore,
  CONFIRM_TIMEOUT_SECS,
  useAgentStore,
} from "./state/agent";
import type { AgentConfig } from "./types/agent";

/**
 * Default config for the Cmd-K palette. Mirrors
 * `AgentConfig::for_test("http://localhost:11434/v1", "qwen2.5-coder:32b")`
 * on the Rust side. The api_key is `"test"` so the Rust
 * `validate()` check passes; v0.1 has no real auth flow.
 */
const DEFAULT_AGENT_CONFIG: AgentConfig = {
  api_base: "http://localhost:11434/v1",
  api_key: "test",
  model: "qwen2.5-coder:32b",
  max_iterations: 20,
  allowed_tools: [
    "talon_list_recent",
    "talon_search",
    "talon_get_exchange",
    "talon_list_tags",
    "talon_list_tags_for_exchange",
  ],
};

/**
 * Phase 1 placeholder shell. Shows the Tauri IPC bridge is alive
 * (the `greet` call round-trips) and displays the engine version.
 * Real UI (capture list, replay tabs, fuzz view, ...) lands in Phase 4.
 *
 * §3.5d adds:
 *  - A Cmd-K (or Ctrl-K) palette that starts an agent run.
 *  - A docked `AgentPanel` showing the active run's status.
 */
export function App() {
  const [greeting, setGreeting] = useState<Greeting | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [paletteValue, setPaletteValue] = useState("");
  const startRun = useAgentStore((s) => s.startRun);

  useEffect(() => {
    greet()
      .then(setGreeting)
      .catch((e) => setError(String(e)));
  }, []);

  // Global Cmd-K / Ctrl-K listener. We attach once on mount and
  // detach on unmount; the listener doesn't depend on component
  // state beyond the imperative callbacks it needs to open the
  // palette and start a run.
  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && e.key === "k") {
        e.preventDefault();
        setPaletteOpen((open) => !open);
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  // Auto-deny pending confirmations after CONFIRM_TIMEOUT_SECS on
  // the UI side, matching the Rust-side timeout. This is a belt
  // alongside the Rust-side oneshot timeout; the Rust one wakes
  // the LLM, the UI one closes the modal even if the WebView
  // missed the resolution event.
  useEffect(() => {
    const unsub = agentStore.subscribe((state, prev) => {
      for (const [runId, run] of Object.entries(state.runs)) {
        const prevRun = prev.runs[runId];
        const becamePending =
          run.pendingConfirm &&
          (!prevRun || !prevRun.pendingConfirm);
        const noLongerPending =
          prevRun?.pendingConfirm && !run.pendingConfirm;
        const existing = state.confirmTimeouts.get(runId);
        if (becamePending && !existing) {
          const t = setTimeout(() => {
            // Fire-and-forget: respondConfirm clears the slot and
            // calls the Rust side. If the user already responded,
            // the Rust side returns an "no pending confirmation"
            // error which is fine to swallow.
            void state.respondConfirm(runId, false, false);
          }, CONFIRM_TIMEOUT_SECS * 1000);
          state.confirmTimeouts.set(runId, t);
        } else if (noLongerPending && existing) {
          clearTimeout(existing);
          state.confirmTimeouts.delete(runId);
        }
      }
    });
    return unsub;
  }, []);

  function handlePaletteSubmit(e: React.FormEvent) {
    e.preventDefault();
    const goal = paletteValue.trim();
    if (!goal) return;
    void startRun(goal, DEFAULT_AGENT_CONFIG);
    setPaletteValue("");
    setPaletteOpen(false);
  }

  return (
    <div className="h-full w-full flex flex-col items-center justify-center gap-4">
      <h1 className="text-4xl font-bold text-accent">Talon</h1>
      {greeting && (
        <p className="text-slate-300">
          {greeting.message}{" "}
          <span className="text-slate-500">v{greeting.version}</span>
        </p>
      )}
      {error && (
        <p className="text-red-400 text-sm">
          Failed to call Rust: {error}
        </p>
      )}
      <p className="text-slate-500 text-sm mt-8">
        v0.1 skeleton · real UI lands in Phase 4
      </p>
      <p className="text-slate-600 text-xs">
        Press <kbd className="rounded bg-bg-rail px-1">Cmd/Ctrl</kbd>+
        <kbd className="rounded bg-bg-rail px-1">K</kbd> to start an
        agent run.
      </p>

      {paletteOpen && (
        <div
          data-testid="cmd-k-palette"
          className="fixed inset-0 z-[70] flex items-start justify-center bg-black/50 pt-32"
          onClick={(e) => {
            // Click outside the form closes the palette.
            if (e.target === e.currentTarget) {
              setPaletteOpen(false);
            }
          }}
        >
          <form
            onSubmit={handlePaletteSubmit}
            className="w-full max-w-xl rounded border border-slate-700 bg-bg-panel p-3 shadow-2xl"
          >
            <label
              htmlFor="cmd-k-palette-input"
              className="mb-2 block text-xs uppercase tracking-wide text-slate-400"
            >
              Agent goal
            </label>
            <input
              id="cmd-k-palette-input"
              data-testid="cmd-k-palette-input"
              type="text"
              autoFocus
              value={paletteValue}
              onChange={(e) => setPaletteValue(e.target.value)}
              placeholder="What do you want the agent to do?"
              className="w-full rounded border border-slate-700 bg-bg-base px-2 py-1 text-sm text-slate-100 focus:border-accent focus:outline-none"
            />
            <div className="mt-2 flex justify-end gap-2">
              <button
                type="button"
                onClick={() => setPaletteOpen(false)}
                className="rounded border border-slate-600 bg-transparent px-2 py-0.5 text-xs text-slate-300 hover:border-slate-400"
              >
                Cancel
              </button>
              <button
                type="submit"
                data-testid="cmd-k-palette-submit"
                className="rounded bg-accent px-2 py-0.5 text-xs font-semibold text-bg-base hover:bg-accent-muted"
              >
                Start
              </button>
            </div>
          </form>
        </div>
      )}

      <AgentPanel />
    </div>
  );
}
