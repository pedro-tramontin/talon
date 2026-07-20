// §4.7 RightRail. The right-hand column of the Capture
// route. Renders a 4-tab layout (Inspector / Decoder /
// Diff / Notes) and switches the active panel based on
// `useUiStore.activeRightTab`.
//
// Spec (§4.7):
//   - Tab state lives in `useUiStore.activeRightTab` so
//     the choice survives re-renders and the `setActiveRightTab`
//     action is testable as a pure store mutation.
//   - The 4 panels (InspectorPanel / DecoderPanel /
//     DiffPanel / NotesPanel) are imported here and
//     rendered in a switch. Only one is mounted at a
//     time — switching tabs unmounts the previous
//     panel (and resets its local state, which is the
//     intended behavior for v1).
//   - The right rail is `RIGHT_RAIL_PX` (320) wide; the
//     Capture route owns the width. The panel is
//     full-height with a fixed-height tab strip on top
//     and an overflow-y body below.

import { useUiStore, RIGHT_TABS, type RightTab } from "../state/ui";
import { DecoderPanel } from "./DecoderPanel";
import { DiffPanel } from "./DiffPanel";
import { InspectorPanel } from "./InspectorPanel";
import { NotesPanel } from "./NotesPanel";

/** Human-friendly label per tab (the value in
 * `RightTab` is the `data-testid` suffix; the label
 * is the visible text). */
const TAB_LABELS: Readonly<Record<RightTab, string>> = {
  inspector: "Inspector",
  decoder: "Decoder",
  diff: "Diff",
  notes: "Notes",
};

export function RightRail() {
  const activeTab = useUiStore((s) => s.activeRightTab);
  const setActiveTab = useUiStore((s) => s.setActiveRightTab);

  return (
    <aside
      data-testid="capture-right-rail-inner"
      className="flex h-full flex-col border-l border-slate-800 bg-bg-rail"
    >
      <div
        data-testid="capture-right-rail-tabs"
        className="flex border-b border-slate-800"
      >
        {RIGHT_TABS.map((tab) => {
          const isActive = activeTab === tab;
          return (
            <button
              key={tab}
              type="button"
              data-testid={`capture-right-rail-tab-${tab}`}
              data-active={isActive ? "true" : "false"}
              onClick={() => {
                setActiveTab(tab);
              }}
              className={`border-r border-slate-800 px-3 py-2 text-xs ${
                isActive
                  ? "text-accent"
                  : "text-slate-300 hover:text-slate-100"
              }`}
            >
              {TAB_LABELS[tab]}
            </button>
          );
        })}
      </div>
      <div
        data-testid="capture-right-rail-body"
        className="flex-1 overflow-y-auto p-3"
      >
        {activeTab === "inspector" && <InspectorPanel />}
        {activeTab === "decoder" && <DecoderPanel />}
        {activeTab === "diff" && <DiffPanel />}
        {activeTab === "notes" && <NotesPanel />}
      </div>
    </aside>
  );
}
