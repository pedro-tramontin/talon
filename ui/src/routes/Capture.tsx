// Capture route. The 3-column layout for the §4.3-4.4 phase:
//   - Top bar (h-12): project dropdown, derived from
//     `useProjectStore`.
//   - Left rail (w-60 / 240px): the §4.5 virtualized
//     exchange list (`<ExchangeList />`).
//   - Main (flex-1): the §4.6 `<ExchangeDetail />` panel
//     (renders the request + response inspector; empty
//     state when no row is selected).
//   - Right rail (w-80 / 320px): 3 placeholder tabs
//     (Inspector / Decoder / Notes) that §4.7 fills in.
//
// No router in v0.1: the `App` component renders `<Capture />`
// directly. `react-router` lands in a later phase when we
// actually need multi-route navigation (the v0.1 UI is
// single-window — `/capture`, `/replay`, `/fuzz` all map to
// the same Tauri window for now).
//
// **v0.5 (added 2026-07-21):** the Capture route mounts the
// `engine_event` wire-bus handler that powers the
// `ExchangeInserted` → `unshiftExchange` + `putDetail`
// pipeline. The engine's wire payload is the FULL
// `HttpExchange` (per the v0.5 `ExchangeInserted.exchange`
// field), so we populate both the thin `ExchangeSummary`
// list and the full `ExchangeDetail` cache from the same
// event. The right-rail reads from the cache; per-click
// `getExchange` round-trips are eliminated for any
// exchange the engine has already announced.

import { useEffect } from "react";
import { useProjectStore } from "../state/project";
import { useUiStore } from "../state/ui";
import { exchangeStore } from "../state/exchange";
import { getWireClient } from "../lib/ws";
import { ExchangeDetail } from "../components/ExchangeDetail";
import { ExchangeList } from "../components/ExchangeList";
import { ProxyControl } from "../components/ProxyControl";
import { RightRail } from "../components/RightRail";
import { ReplayView } from "../components/ReplayView";
import { ScopeRuleEditor } from "../components/ScopeRuleEditor";
import type {
  ExchangeDetail as ExchangeDetailType,
  ExchangeSummary,
} from "../types/domain";
import type { ExchangeId, ProjectId } from "../types/ids";

/**
 * Subscribe to the wire bus's `engine_event` channel and
 * dispatch by event kind. The handler is registered once
 * per `Capture` mount (the v0.5 change) — the previous v0.1
 * design didn't subscribe at all (the UI read `listExchanges`
 * once on project open and never re-synced). The handler
 * is the entry point for both the row-list prepending
 * (`unshiftExchange`) and the detail-cache insertion
 * (`putDetail`), per the v0.5 cache-first detail view.
 */
function useEngineEventHandler() {
  useEffect(() => {
    const client = getWireClient();
    const unsub = client.subscribe("engine_event", (payload) => {
      // v0.5 narrowing: the wire event's `payload` is the
      // serialized `EngineEvent` JSON. The Rust side uses
      // serde_json internally; the field names match the
      // Rust `serde(rename_all = "snake_case")` convention
      // because the engine's `EngineEvent` doesn't have a
      // top-level `#[serde(rename_all)]` attribute but the
      // VARIANTS have `#[serde(rename_all = "snake_case")]`
      // so the `kind` discriminator is the variant's name
      // in snake_case.
      const p = payload as {
        kind?: string;
        id?: ExchangeId;
        project_id?: ProjectId;
        summary?: string;
        exchange?: ExchangeDetailType;
      };
      if (p.kind !== "exchange_inserted") return;
      // The summary path. v0.5 has the engine emit the
      // full exchange, but the list view still shows just
      // the summary line. We derive it from the `exchange`'s
      // `meta.summary` (canonical source).
      const exchange = p.exchange;
      if (!exchange) return;
      const summary: ExchangeSummary = {
        id: exchange.meta.id,
        project_id: exchange.meta.project_id,
        timestamp: exchange.meta.timestamp,
        duration_ns: exchange.meta.duration_ns,
        summary: exchange.meta.summary,
        scope_state: exchange.meta.scope_state,
        notes: exchange.meta.notes,
        starred: exchange.meta.starred,
      };
      // The summary path. v0.5 has the engine emit the
      // full exchange, so the list view's `exchanges` array
      // gets the row AND the detail cache gets the full body
      // in one step. (v0.1 only got a summary string; the
      // right-rail then called `getExchange` to fetch the
      // full body. v0.5 eliminates that round-trip.)
      exchangeStore.getState().unshiftExchange(summary);
      exchangeStore.getState().putDetail(exchange);
    });
    return unsub;
  }, []);
}

/** Width of the left rail in px. Pinned at 240 to match the
 * Tailwind `w-60` class. The Capture.test.tsx test asserts
 * this against the rendered DOM. */
export const LEFT_RAIL_PX = 240;

/** Width of the right rail in px. Pinned at 320 (`w-80`). */
export const RIGHT_RAIL_PX = 320;

/** Height of the top bar in px. `h-12` = 48px. */
export const TOP_BAR_PX = 48;

/**
 * Top bar. The project dropdown reads from `useProjectStore`:
 *   - `activeProjectId` is the selected id
 *   - `projects` is the list of open projects
 *   - `setActiveProject` is the change handler
 *
 * In v0.1 this is a plain `<select>` (no combobox / search).
 * §4.7 wires a richer dropdown if needed.
 */
function ProjectDropdown() {
  const projects = useProjectStore((s) => s.projects);
  const activeProjectId = useProjectStore((s) => s.activeProjectId);
  const setActiveProject = useProjectStore((s) => s.setActiveProject);
  const setSettingsOpen = useUiStore((s) => s.setSettingsOpen);
  const setNewProjectModalOpen = useUiStore((s) => s.setNewProjectModalOpen);

  // Sentinel value for the "New..." item at the bottom
  // of the dropdown. When the user picks it, we open
  // the NewProjectModal + reset the dropdown to the
  // previously-selected project (so cancelling the
  // modal returns to the same selection).
  const NEW_SENTINEL = "__new__";

  return (
    <div className="flex items-center gap-2">
      <label
        htmlFor="capture-project-select"
        className="text-xs uppercase tracking-wide text-slate-400"
      >
        Project
      </label>
      <select
        id="capture-project-select"
        data-testid="capture-project-select"
        value={activeProjectId ?? ""}
        onChange={(e) => {
          const v = e.target.value;
          if (v === NEW_SENTINEL) {
            // Open the NewProjectModal + reset the
            // dropdown so the user sees the
            // previously-selected project when the
            // modal closes (regardless of whether
            // they created a new project or
            // cancelled).
            setNewProjectModalOpen(true);
            setActiveProject(activeProjectId);
            return;
          }
          setActiveProject(v === "" ? null : (v as typeof activeProjectId));
        }}
        className="rounded border border-slate-700 bg-bg-base px-2 py-1 text-sm text-slate-100 focus:border-accent focus:outline-none"
      >
        <option value="">— None —</option>
        {projects.map((p) => (
          <option key={p.id} value={p.id}>
            {p.name}
          </option>
        ))}
        {/* Phase 8 (full v1, per user directive): a
         * "New..." sentinel item at the bottom of the
         * dropdown opens the NewProjectModal. */}
        <option value={NEW_SENTINEL}>New...</option>
      </select>
      {/* Phase 8 (2026-07-23): the "+" button opens the New
       * Project modal that wires `openProject` (Tauri
       * command at `app/src/commands/core.rs:144`) to
       * `projectStore.addProject` (Zustand action). The
       * button's style matches the adjacent Settings
       * button: rounded border, hover state, small text. */}
      <button
        data-testid="capture-new-project-button"
        onClick={() => setNewProjectModalOpen(true)}
        className="rounded border border-slate-700 bg-transparent px-2 py-1 text-xs text-slate-300 hover:border-slate-400 hover:text-accent"
        aria-label="New project"
      >
        + New
      </button>
      {/* Phase 6 §6.7: the Settings button opens the modal that
       * hosts the M&R editor. Lives in the top bar per the
       * spec's "Add a Settings button to the TopBar" line. */}
      <button
        data-testid="capture-settings-button"
        onClick={() => setSettingsOpen(true)}
        className="ml-2 rounded border border-slate-700 bg-transparent px-2 py-1 text-xs text-slate-300 hover:border-slate-400 hover:text-accent"
      >
        Settings
      </button>
    </div>
  );
}

/**
 * Left-rail container. The §4.5 PR wires the actual
 * virtualized list here via `<ExchangeList />`. The
 * `data-testid="capture-left-rail"` stays on the outer
 * element so the Capture.test.tsx layout assertion (width
 * pinning) still works.
 *
 * Phase 6 §6.6: the `<ScopeRuleEditor />` sits below the
 * list in the same column. The editor is a separate
 * component so the test surface is independent of the
 * list's virtualizer.
 */
function ExchangeLeftRail() {
  return (
    <aside
      data-testid="capture-left-rail"
      className="flex h-full flex-col"
      style={{ width: `${LEFT_RAIL_PX}px` }}
    >
      <div className="flex-1 overflow-hidden">
        <ExchangeList />
      </div>
      <ScopeRuleEditor />
    </aside>
  );
}

/**
 * Main column. Renders either the §4.6 `<ExchangeDetail />`
 * (the default for the Capture route) OR the Phase 5
 * `<ReplayView />` based on the UI store's `mode` field.
 * The left rail (`<ExchangeList />`) and the right rail
 * (`<RightRail />`) stay put; only the center column
 * changes. The list and the detail share the same store
 * signal: clicking a row in `<ExchangeList />` sets
 * `selectedId`, and the detail reads it.
 */
function CaptureMain() {
  const mode = useUiStore((s) => s.mode);
  if (mode === "replay") return <ReplayView />;
  return <ExchangeDetail />;
}

/**
 * Right-rail tab strip + body. The §4.7 PR owns the 4 tabs
 * (Inspector / Decoder / Diff / Notes); the Capture route
 * just wraps it with the width constraint. The
 * `data-testid="capture-right-rail"` and the inline width
 * style live on the outer wrapper (this `<div>`) so the
 * Capture.test.tsx layout assertion can find them in one
 * place — the underlying `<RightRail />` is the
 * styled-and-tab-stripped inner element.
 */
function CaptureRightRail() {
  return (
    <div
      data-testid="capture-right-rail"
      className="h-full"
      style={{ width: `${RIGHT_RAIL_PX}px` }}
    >
      <RightRail />
    </div>
  );
}

/**
 * Capture route. The 3-column layout for Phase 4 Part A.
 */
export function Capture() {
  // v0.5: mount the wire-bus handler that powers the
  // `ExchangeInserted` → `unshiftExchange` + `putDetail`
  // pipeline. The hook has no return value; its sole
  // purpose is the side effect of subscribing on mount
  // and unsubscribing on unmount.
  useEngineEventHandler();
  return (
    <div className="flex h-full w-full flex-col">
      <header
        data-testid="capture-top-bar"
        className="flex items-center border-b border-slate-800 bg-bg-panel px-3"
        style={{ height: `${TOP_BAR_PX}px` }}
      >
        <ProjectDropdown />
        {/* v0.5+ post-batch gap-fix (2026-07-24, P0 #1):
         * the MITM proxy control. Sits at the right end of
         * the top bar (`ml-auto` inside the component) so
         * the project dropdown keeps its left-aligned
         * position. The control is a 2-state widget
         * (start/stop) + a status pill + a `rulesActive`
         * badge (per the Phase 6 §6 open item). */}
        <ProxyControl />
      </header>
      <div className="flex flex-1 overflow-hidden">
        <ExchangeLeftRail />
        <CaptureMain />
        <CaptureRightRail />
      </div>
    </div>
  );
}
