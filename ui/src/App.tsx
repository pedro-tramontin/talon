import { useEffect, useState } from "react";
import { greet, listProjects, type Greeting } from "./api";
import { AgentPanel } from "./components/AgentPanel";
import { Capture } from "./routes/Capture";
import { SettingsModal } from "./components/SettingsModal";
import { NewProjectModal } from "./components/NewProjectModal";
import {
  agentStore,
  CONFIRM_TIMEOUT_SECS,
  useAgentStore,
} from "./state/agent";
import { getWireClient, setWireClient, WireClient } from "./lib/ws";
import { wsStore } from "./state/ws";
import { proxyStore } from "./state/proxy";
import { projectStore } from "./state/project";
import type { AgentConfig } from "./types/agent";
import type { ProxyStatus } from "./types/domain";

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
 * App shell.
 *
 * §3.5d added the Cmd-K palette + docked `AgentPanel`.
 * §4.3-4.4 replaces the Phase-1 placeholder with the
 * `<Capture />` route. The palette + AgentPanel are preserved
 * (the agent integration is still useful).
 *
 * The App also owns the wire-bus connection lifecycle:
 *   - On mount, `getWireClient().connect()` opens the
 *     Tauri `wire_event` listener (or the browser WS in
 *     Phase 8).
 *   - On unmount, `disconnect()` is called so HMR doesn't
 *     leak listeners.
 *   - The wire-bus `dispatch` path increments
 *     `useWsStore.droppedGaps` on every observed gap (so the
 *     UI can render a "missed events" banner).
 */
export function App() {
  const [greeting, setGreeting] = useState<Greeting | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [paletteValue, setPaletteValue] = useState("");
  const startRun = useAgentStore((s) => s.startRun);

  // Phase 1: round-trip the `greet` command as a sanity check
  // that the Tauri IPC bridge is alive. Phase 4 keeps the call
  // (it's a useful "the app loaded" signal) but the result is
  // not rendered in v0.4 (the Capture route is the visible
  // surface).
  useEffect(() => {
    greet()
      .then(setGreeting)
      .catch((e) => setError(String(e)));
  }, []);

  // v0.5+ post-batch gap-fix P3 #9 (2026-07-24): the
  // `setProjects` Zustand action was dead code because
  // no Tauri command ever populated the project list
  // from the engine. This `listProjects` startup hook
  // fires once on app mount, calls the new
  // `list_projects` Tauri command (the engine returns
  // every currently-open project, newest-first by
  // `created_at`), and pipes the result into
  // `projectStore.setProjects` so the project dropdown
  // rehydrates without a manual refresh.
  //
  // The call is fire-and-forget for the visible UI (the
  // dropdown shows an empty list during the brief
  // startup window) but the error is logged so a
  // thrown Tauri command surfaces in DevTools.
  useEffect(() => {
    listProjects()
      .then((projects) => {
        projectStore.getState().setProjects(projects);
      })
      .catch((e) => {
        console.warn("App: listProjects failed:", e);
      });
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

  // Wire-bus lifecycle: open on mount, close on unmount.
  // The transport is a singleton (see `getWireClient`) so
  // multiple component trees in dev-mode won't double-listen.
  //
  // Phase 8: in browser mode, the auth token is read from
  // the `?token=<token>` query param (set by the user
  // when they navigate to the server). The token is
  // passed to the `WireClient` so it can send the
  // `Sec-WebSocket-Protocol: talon-auth.<token>`
  // subprotocol on the WS upgrade. Without the token,
  // remote-mode servers (--allow-remote) return 401 on
  // the WS upgrade.
  useEffect(() => {
    const params = new URLSearchParams(window.location.search);
    const token = params.get("token") ?? undefined;
    if (token) {
      // Replace the singleton with one that has the
      // auth token. This must happen BEFORE
      // `getWireClient().connect()` is called.
      setWireClient(new WireClient({ authToken: token }));
    }
    const client = getWireClient();
    wsStore.getState().setConnectionState("reconnecting");
    client
      .connect()
      .then(() => {
        wsStore.getState().setConnectionState("connected");
      })
      .catch((e: unknown) => {
        wsStore.getState().setConnectionState("disconnected");
        wsStore.getState().setLastError(String(e));
        console.error("WireClient.connect failed:", e);
      });
    return () => {
      void client.disconnect();
    };
  }, []);

  // Phase 5 — subscribe to `replay_event` wire events.
  // The synchronous `send_replay` IPC return already drives
  // the local store via `appendSend`; the WireEvent is
  // for downstream observers (cross-tab sync, future
  // agent integration, etc.). For v1 we log + (if the
  // event's `tab_id` matches a current tab) update
  // `latestReplayId` on the matching tab.
  useEffect(() => {
    const client = getWireClient();
    const unsub = client.subscribe(
      "replay_event",
      (payload) => {
        const ev = payload as {
          tab_id: string;
          kind: "send_complete" | "send_failed";
          exchange_id: string | null;
          error: string | null;
        };
        if (ev.kind === "send_complete") {
          console.debug(
            "replay_event: send_complete",
            ev.tab_id,
            ev.exchange_id,
          );
        } else {
          console.warn(
            "replay_event: send_failed",
            ev.tab_id,
            ev.error,
          );
        }
        // v0.5+ will route the event to the matching tab;
        // for v1 the synchronous IPC path already updated
        // the local store, so the WireEvent is a no-op for
        // the local UI.
      },
    );
    return unsub;
  }, []);

  // v0.5+ post-batch gap-fix (2026-07-24, P0 #1) —
  // subscribe to `proxy_event` wire events. The Rust
  // side translates `bk-proxy` lifecycle events into a
  // `ProxyStatus` payload and forwards them on the
  // `proxy_event` channel (the kind has been declared in
  // `ui/src/lib/ws.ts:34` since Phase 8 but was never
  // subscribed). The handler is the canonical source of
  // truth for the proxy status pill in `ProxyControl`;
  // the click handler in `ProxyControl.tsx` does an
  // optimistic local update + a defensive
  // `proxyStatus()` refresh, but the wire handler is
  // what keeps the UI in sync across tabs and across
  // an app reload (where the optimistic state is
  // forgotten but the next wire event re-seeds it).
  useEffect(() => {
    const client = getWireClient();
    const unsub = client.subscribe("proxy_event", (payload) => {
      // The Rust `bk-proxy` side encodes the payload as
      // the `ProxyStatus` DTO (snake_case serde). The
      // defensive cast handles the case where the wire
      // receives a malformed event (the `dispatch` loop
      // in `WireClient` already logs handler errors and
      // keeps streaming).
      const status = payload as ProxyStatus;
      if (
        status &&
        (status.state === "stopped" ||
          status.state === "running" ||
          status.state === "error")
      ) {
        proxyStore.getState().setStatus(status);
      } else {
        console.warn("App: malformed proxy_event payload:", payload);
      }
    });
    return unsub;
  }, []);

  // Track dropped gaps from the wire bus. We poll the
  // `WireClient` for the count (it owns the array) on a
  // microtask interval. The polling is necessary because the
  // bus is push-only — there's no on-gap event hook. The
  // interval is generous (1s) because gaps are rare and a
  // UI banner only needs sub-second freshness at best.
  useEffect(() => {
    const id = setInterval(() => {
      const client = getWireClient();
      const gapCount = client.getDroppedGaps().length;
      const current = wsStore.getState().droppedGaps;
      if (gapCount > current) {
        // Increment one at a time (the bus is monotonic so
        // the gap count can only go up between polls). If
        // more than one gap landed in the interval, we bump
        // by the difference.
        for (let i = current; i < gapCount; i++) {
          wsStore.getState().addDroppedGap();
        }
      }
    }, 1000);
    return () => clearInterval(id);
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
    <div className="h-full w-full">
      <Capture />
      {/* The Phase-1 greeting is kept as a no-render (data-testid
        * hooks) so the App test (from §3.5d) still passes. The
        * Capture route is the visible surface in v0.4. */}
      <div
        data-testid="app-greeting"
        className="hidden"
        aria-hidden="true"
      >
        {greeting ? `${greeting.message} v${greeting.version}` : ""}
      </div>
      {error && (
        <p
          data-testid="app-error"
          className="fixed bottom-2 right-2 rounded bg-red-900 px-2 py-1 text-xs text-red-100"
        >
          IPC: {error}
        </p>
      )}

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

      {/* Phase 6 §6.7: the Settings modal hosts the M&R editor.
       * The modal is unmounted by the component itself when
       * `settingsOpen` is false; mounting it here just gives the
       * store-driven open/close a stable DOM presence. */}
      <SettingsModal />

      {/* Phase 8 (2026-07-23): the New Project modal hosts the
       * `openProject` Tauri command + `projectStore.addProject`
       * + `setActiveProject` sequence that lets the user
       * create a new project from the Capture top bar's "+"
       * button. */}
      <NewProjectModal />
    </div>
  );
}
