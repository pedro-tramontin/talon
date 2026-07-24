// Proxy control for the Capture top bar.
//
// This is the §1 (P0) of the 2026-07-24 UI gap audit. The
// `start_proxy` / `stop_proxy` / `proxy_status` Tauri
// commands have been on `main` for several phases
// (`app/src/commands/core.rs:373-434`), the `api.ts`
// wrappers exist (`ui/src/api.ts:229-241`), and the
// `proxyStore` Zustand singleton exists
// (`ui/src/state/proxy.ts`) — but no component imported any
// of them. This widget is the missing UI affordance.
//
// State machine:
//
//   stopped   ── click Start ─►   starting (local)  ── IPC ok ─► running
//                               │                  ── IPC err ─► error
//   error     ── click Retry ──► starting (local)  ── IPC ok ─► running
//                                                   ── IPC err ─► error
//   running   ── click Stop ───► stopped
//
// The Rust `ProxyState` enum has only three variants
// (Stopped / Running / Error — see `app/src/proxy_handle.rs`)
// so the `starting` state is a local-only UI affordance. We
// track it with a `useState<boolean>` so the button stays
// disabled (no race-clicks) until the IPC round-trip
// resolves. The store still reflects the canonical Rust
// state.
//
// Wire-bus sync:
//
//   The `proxy_event` WireEvent kind (already in
//   `ui/src/lib/ws.ts:34`) is subscribed in `App.tsx`; this
//   component is mounted inside the Capture top bar, so the
//   store is updated by the App-level handler whenever the
//   engine pushes a proxy lifecycle event. The local
//   `starting` flag is cleared by the local effect that
//   watches the store's transition back to "running" (or
//   "error"). No double-fires — the App-level handler does
//   the only store mutation; the click handler does its own
//   optimistic mutation to keep the button responsive in
//   single-process dev (where the wire event arrives
//   after the IPC reply, so the click handler's optimistic
//   update is what the user sees first).
//
// Related v0.5+ follow-up (the Phase 6 §6 open item "Wire
// actual scope rules + M&R rules into `start_proxy`"):
// The engine-side wiring is already done
// (`start_with_rules` is the entry point, called with
// `scope_rules` + `match_replace_rules` from
// `app/src/commands/core.rs:400-419`). What's left is the
// UI-side affordance: a `rulesActive: <count>` badge in the
// status pill that reads from `useUiStore.scopeRules` and
// `useUiStore.matchReplaceRules`. The badge is purely
// cosmetic — clicking it does nothing (a v0.5+ follow-up
// can wire a tooltip explaining "active rules" + a "go to
// settings" link).

import { useEffect, useState } from "react";
import { useProxyStore } from "../state/proxy";
import { useUiStore } from "../state/ui";
import { proxyStatus, startProxy, stopProxy } from "../api";
import type { ProxyStatus, ProxyState } from "../types/domain";

/**
 * Truncate a CA fingerprint for the status-pill display.
 * Returns the first `head` and last `tail` characters
 * separated by an ellipsis (e.g. "ab:cd:…:89:01"). The full
 * fingerprint is always available in the pill's `title`
 * attribute (browser tooltip).
 */
function truncateFingerprint(fp: string, head = 4, tail = 4): string {
  if (fp.length <= head + tail + 1) return fp;
  return `${fp.slice(0, head)}…${fp.slice(-tail)}`;
}

/**
 * Format a `SocketAddr` string for the status pill. The Rust
 * side emits strings like `"127.0.0.1:8080"` (the `Display`
 * for `SocketAddr`); the UI mirrors that with a small
 * defensive fallback for the rare `null` case.
 */
function formatAddr(addr: string | null): string {
  return addr ?? "—";
}

/**
 * Status pill color. Stopped = gray, starting = amber,
 * running = green, error = red. The pill text comes from
 * `statusText()`.
 */
function pillClass(state: ProxyState, starting: boolean): string {
  if (starting) return "bg-amber-900/40 text-amber-200 border-amber-700";
  switch (state) {
    case "running":
      return "bg-emerald-900/40 text-emerald-200 border-emerald-700";
    case "error":
      return "bg-red-900/40 text-red-200 border-red-700";
    case "stopped":
    default:
      return "bg-slate-800 text-slate-300 border-slate-700";
  }
}

/**
 * Human-readable status text for the pill. Includes the
 * `listener_addr` for `running` and the `last_error`
 * truncated for `error`.
 */
function statusText(status: ProxyStatus, starting: boolean): string {
  if (starting) return "Starting…";
  switch (status.state) {
    case "running":
      return `Running on ${formatAddr(status.listener_addr)}`;
    case "error":
      return `Error: ${(status.last_error ?? "unknown").slice(0, 60)}`;
    case "stopped":
    default:
      return "Stopped";
  }
}

/**
 * `ProxyControl` — the top-bar widget. Renders a 2-state
 * control (start vs stop) + a status pill + the
 * `rulesActive` badge (P0 #1 bonus affordance, per the
 * Phase 6 §6 open item).
 */
export function ProxyControl() {
  const status = useProxyStore((s) => s.status);
  const setStatus = useProxyStore((s) => s.setStatus);
  const scopeRules = useUiStore((s) => s.scopeRules);
  const matchReplaceRules = useUiStore((s) => s.matchReplaceRules);

  // Local-only "starting" affordance. Cleared on the next
  // store update that lands in `running` or `error`. Cleared
  // on the immediate post-click IPC resolution so the
  // button is responsive in dev. See the file header for
  // the full state machine.
  const [starting, setStarting] = useState(false);

  // Seed the store from the backend on mount. Cheap
  // round-trip (the proxy handle is in-process); the seed
  // is what the user sees if they open the app and the
  // proxy is already running (e.g. they reloaded the
  // webview).
  useEffect(() => {
    let cancelled = false;
    void proxyStatus()
      .then((s) => {
        if (cancelled) return;
        // Defensive: the IPC layer can return `null` in
        // test environments where the Tauri bridge is
        // mocked. The Rust side always returns a real
        // `ProxyStatus` (the DTO has no `null` variants
        // for in-process calls), so a `null` here is
        // a "no canonical status" signal that we ignore.
        if (
          s &&
          (s.state === "stopped" ||
            s.state === "running" ||
            s.state === "error")
        ) {
          setStatus(s);
        }
      })
      .catch((e) => {
        // Non-fatal: the proxy status IPC failed (e.g. the
        // browser-mode WS isn't connected yet). The store
        // keeps the default `stopped` state; the start
        // button still works.
        console.error("ProxyControl: initial proxyStatus() failed:", e);
      });
    return () => {
      cancelled = true;
    };
  }, [setStatus]);

  // When the store transitions to a non-`stopped` state
  // while `starting` is true, clear the local flag. The
  // transition can come from the App-level `proxy_event`
  // wire handler OR from the immediate post-click IPC
  // reply (in the `startProxy` handler below). The
  // effect-based clearing keeps the two paths from
  // double-firing.
  useEffect(() => {
    if (
      starting &&
      status &&
      (status.state === "running" || status.state === "error")
    ) {
      setStarting(false);
    }
  }, [starting, status]);

  // Re-fetch the canonical status on every `proxy_event`
  // the App-level handler dispatches. The App handler
  // already updates the store; this effect is a no-op if
  // the store is up to date. We keep it as a defensive
  // measure for the case where the wire event arrives
  // BEFORE the synchronous `startProxy` IPC reply
  // (browser mode + remote `bk-server`).

  async function handleStart() {
    if (starting) return; // no race-clicks
    setStarting(true);
    try {
      await startProxy();
      // Refresh the status from the canonical source so
      // the pill shows the listener_addr + fingerprint.
      // The store is also updated by the App-level
      // `proxy_event` handler; this is a no-op in that
      // case.
      const next = await proxyStatus();
      setStatus(next);
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setStatus({
        state: "error",
        listener_addr: null,
        ca_fingerprint: null,
        last_error: message,
      });
      setStarting(false);
    }
  }

  async function handleStop() {
    if (starting) return; // cannot stop while starting
    try {
      await stopProxy();
      const next = await proxyStatus();
      setStatus(next);
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setStatus({
        state: "error",
        listener_addr: null,
        ca_fingerprint: null,
        last_error: message,
      });
    }
  }

  const isRunning = status.state === "running" && !starting;
  const isError = status.state === "error" && !starting;
  const buttonLabel = isError
    ? "Retry"
    : isRunning
      ? "Stop"
      : "Start proxy";
  const rulesActive = scopeRules.length + matchReplaceRules.length;

  return (
    <div
      data-testid="proxy-control"
      className="ml-auto flex items-center gap-2"
    >
      {/* Status pill. Full fingerprint / error in the
       * `title` attribute (browser tooltip); the pill
       * text is the truncated form. */}
      <span
        data-testid="proxy-control-pill"
        title={
          status.state === "running" && status.ca_fingerprint
            ? `CA fingerprint: ${status.ca_fingerprint}`
            : status.state === "error" && status.last_error
              ? status.last_error
              : undefined
        }
        className={`rounded border px-2 py-0.5 text-xs ${pillClass(
          status.state,
          starting,
        )}`}
      >
        <span data-testid="proxy-control-pill-text">
          {statusText(status, starting)}
        </span>
        {isRunning && status.ca_fingerprint && (
          <span
            data-testid="proxy-control-fingerprint"
            className="ml-2 font-mono text-[10px] text-slate-400"
          >
            CA {truncateFingerprint(status.ca_fingerprint)}
          </span>
        )}
      </span>
      {/* Active rules badge. Read-only count of the
       * scope + M&R rules in the UI store. The click is
       * a no-op (a v0.5+ follow-up can wire a tooltip
       * with a "go to settings" link). */}
      <span
        data-testid="proxy-control-rules-active"
        title={`${scopeRules.length} scope rule(s) + ${matchReplaceRules.length} match & replace rule(s) will be applied to captures`}
        className="rounded border border-slate-700 bg-bg-rail px-2 py-0.5 text-xs text-slate-300"
        aria-label={`${rulesActive} active rules`}
      >
        {rulesActive} rule{rulesActive === 1 ? "" : "s"} active
      </span>
      <button
        data-testid="proxy-control-toggle"
        type="button"
        disabled={starting}
        onClick={isRunning ? handleStop : handleStart}
        className={
          isRunning
            ? "rounded border border-red-700 bg-transparent px-2 py-0.5 text-xs text-red-200 hover:border-red-500 disabled:cursor-not-allowed disabled:opacity-50"
            : isError
              ? "rounded border border-amber-700 bg-transparent px-2 py-0.5 text-xs text-amber-200 hover:border-amber-500 disabled:cursor-not-allowed disabled:opacity-50"
              : "rounded border border-emerald-700 bg-transparent px-2 py-0.5 text-xs text-emerald-200 hover:border-emerald-500 disabled:cursor-not-allowed disabled:opacity-50"
        }
      >
        {buttonLabel}
      </button>
    </div>
  );
}
