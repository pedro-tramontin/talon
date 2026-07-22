// Replay tab bar. Horizontal strip of tabs at the top of
// the Replay view, one per open `ReplayTab` in the store.
// Each tab shows the tab's display name + a close button
// (`✕`). Click switches active; close calls `closeTab(id)`
// with `e.stopPropagation()` to avoid the parent
// `onClick`. Hidden when no tabs are open (the parent
// `ReplayView` shows the empty state instead).
//
// Phase 5 — §5.4.

import { useReplayStore } from "../state/replay";
import { useUiStore } from "../state/ui";

export function ReplayTabBar() {
  const tabs = useReplayStore((s) => s.tabs);
  const activeId = useReplayStore((s) => s.activeTabId);
  const setActive = useReplayStore((s) => s.setActive);
  const closeTab = useReplayStore((s) => s.closeTab);
  const setMode = useUiStore((s) => s.setMode);

  if (tabs.length === 0) return null;

  return (
    <div
      data-testid="replay-tab-bar"
      className="flex border-b border-slate-700 bg-bg-rail overflow-x-auto"
    >
      {tabs.map((tab) => (
        <div
          key={tab.id}
          data-testid="replay-tab-bar-tab"
          data-tab-id={tab.id}
          onClick={() => {
            setActive(tab.id);
            setMode("replay");
          }}
          className={`flex items-center gap-2 cursor-pointer border-r border-slate-700 px-3 py-1 ${
            activeId === tab.id
              ? "bg-bg-base text-accent"
              : "text-slate-300 hover:text-slate-100"
          }`}
        >
          <span className="font-mono text-xs">{tab.name}</span>
          <button
            type="button"
            data-testid="replay-tab-bar-close"
            onClick={(e) => {
              e.stopPropagation();
              closeTab(tab.id);
            }}
            className="text-xs text-slate-500 hover:text-red-400"
            aria-label={`Close tab ${tab.name}`}
          >
            ✕
          </button>
        </div>
      ))}
    </div>
  );
}
