// Capture route. The 3-column layout for the §4.3-4.4 phase:
//   - Top bar (h-12): project dropdown, derived from
//     `useProjectStore`.
//   - Left rail (w-60 / 240px): placeholder for the §4.5
//     virtualized exchange list.
//   - Main (flex-1): empty state until §4.5+ wires the
//     detail view.
//   - Right rail (w-80 / 320px): 3 placeholder tabs
//     (Inspector / Decoder / Notes) that §4.6+ fills in.
//
// No router in v0.1: the `App` component renders `<Capture />`
// directly. `react-router` lands in a later phase when we
// actually need multi-route navigation (the v0.1 UI is
// single-window — `/capture`, `/replay`, `/fuzz` all map to
// the same Tauri window for now).

import { useProjectStore } from "../state/project";

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
      </select>
    </div>
  );
}

/**
 * Left-rail placeholder. The §4.5 PR replaces this with the
 * virtualized `<ExchangeList />`. For now it's a 240px-wide
 * empty panel with a "lands in §4.5" label.
 */
function ExchangeListPlaceholder() {
  return (
    <aside
      data-testid="capture-left-rail"
      className="h-full border-r border-slate-800 bg-bg-rail"
      style={{ width: `${LEFT_RAIL_PX}px` }}
    >
      <div className="p-3 text-xs text-slate-500">
        Exchange list (landed in §4.5)
      </div>
    </aside>
  );
}

/**
 * Main-panel empty state. §4.5 fills the top of main with
 * the virtualized list; §4.6 fills the detail view below.
 */
function MainEmpty() {
  return (
    <main
      data-testid="capture-main"
      className="h-full flex-1 bg-bg-base"
    >
      <div className="flex h-full items-center justify-center">
        <p
          data-testid="capture-main-empty"
          className="text-sm text-slate-500"
        >
          Select an exchange to view details (lands in §4.5+).
        </p>
      </div>
    </main>
  );
}

/**
 * Right-rail placeholder. Three tabs (Inspector / Decoder /
 * Notes) that just render their name. §4.6 fills these in
 * with the actual exchange-detail panes.
 */
function RightRailPlaceholder() {
  return (
    <aside
      data-testid="capture-right-rail"
      className="h-full border-l border-slate-800 bg-bg-rail"
      style={{ width: `${RIGHT_RAIL_PX}px` }}
    >
      <div className="flex border-b border-slate-800">
        <span
          data-testid="capture-right-rail-tab-inspector"
          className="px-3 py-2 text-xs text-slate-300 border-r border-slate-800"
        >
          Inspector
        </span>
        <span
          data-testid="capture-right-rail-tab-decoder"
          className="px-3 py-2 text-xs text-slate-300 border-r border-slate-800"
        >
          Decoder
        </span>
        <span
          data-testid="capture-right-rail-tab-notes"
          className="px-3 py-2 text-xs text-slate-300"
        >
          Notes
        </span>
      </div>
    </aside>
  );
}

/**
 * Capture route. The 3-column layout for Phase 4 Part A.
 */
export function Capture() {
  return (
    <div className="flex h-full w-full flex-col">
      <header
        data-testid="capture-top-bar"
        className="flex items-center border-b border-slate-800 bg-bg-panel px-3"
        style={{ height: `${TOP_BAR_PX}px` }}
      >
        <ProjectDropdown />
      </header>
      <div className="flex flex-1 overflow-hidden">
        <ExchangeListPlaceholder />
        <MainEmpty />
        <RightRailPlaceholder />
      </div>
    </div>
  );
}
