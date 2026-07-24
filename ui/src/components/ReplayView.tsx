// Replay view. The main Phase 5 view shown when the UI
// store's `mode === "replay"`. Layout:
//   - Top: `ReplayTabBar` (hidden when no tabs).
//   - Left half: `ReplayRequestEditor` + the collapsible
//     `ReplayHistoryPanel` drawer below.
//   - Right half: `ReplayResponseViewer`.
//
// Empty state when no active tab: "No replay tab open.
// Click 'Replay' on a capture to start one." with a
// "Back to capture" button that flips `mode` back.
//
// Phase 5 — §5.4 + §5.5.
//
// v0.5+ post-batch gap-fix P2 #7 (2026-07-24): the
// "← back to source" button is shown next to the
// tab bar when the active tab has a `sourceExchangeId`
// (it was created from a capture, not from a fresh
// "new replay"). Clicking it: sets
// `exchangeStore.selectedId` to the source id and
// flips `uiStore.mode` back to "capture" so the
// user lands on the source exchange's detail
// view. The button is hidden for tabs without a
// `sourceExchangeId` (replays typed in from scratch).

import { useState } from "react";
import { useReplayStore } from "../state/replay";
import { useUiStore } from "../state/ui";
import { exchangeStore } from "../state/exchange";
import { ReplayTabBar } from "./ReplayTabBar";
import { ReplayRequestEditor } from "./ReplayRequestEditor";
import { ReplayResponseViewer } from "./ReplayResponseViewer";
import { ReplayHistoryPanel } from "./ReplayHistoryPanel";

export function ReplayView() {
  const activeId = useReplayStore((s) => s.activeTabId);
  const tab = useReplayStore((s) =>
    s.activeTabId ? s.tabs.find((t) => t.id === s.activeTabId) : undefined,
  );
  const setMode = useUiStore((s) => s.setMode);
  const setSelectedId = exchangeStore.getState().setSelectedId;

  // Default: history drawer open. The user can collapse
  // it to focus on the editor.
  const [historyOpen, setHistoryOpen] = useState(true);

  // v0.5+ post-batch gap-fix P2 #7: the
  // back-to-source handler. The `tab.sourceExchangeId`
  // is non-null when the tab was created from a
  // capture (the `openTab` source has an
  // `exchangeId`). The button is hidden when the
  // id is null (the user typed in a fresh replay
  // from scratch, with no source capture).
  function handleBackToSource() {
    if (!tab?.sourceExchangeId) return;
    setSelectedId(tab.sourceExchangeId);
    setMode("capture");
  }

  if (!activeId || !tab) {
    return (
      <div
        data-testid="replay-view-empty"
        className="flex flex-1 items-center justify-center text-sm text-slate-500"
      >
        No replay tab open. Click &quot;Replay&quot; on a capture to start one.
        <button
          type="button"
          onClick={() => setMode("capture")}
          className="ml-2 text-accent underline"
        >
          Back to capture
        </button>
      </div>
    );
  }

  return (
    <div
      data-testid="replay-view"
      className="flex min-h-0 flex-1 flex-col"
    >
      <div className="flex items-center justify-between border-b border-slate-800 bg-bg-panel px-2 py-1">
        <ReplayTabBar />
        {/* v0.5+ post-batch gap-fix P2 #7: the
         * "← back to source" button. Hidden when the
         * tab has no source exchange (the user
         * created the replay from scratch). */}
        {tab.sourceExchangeId && (
          <button
            type="button"
            data-testid="replay-view-back-to-source"
            onClick={handleBackToSource}
            className="rounded border border-slate-700 bg-transparent px-2 py-0.5 text-xs text-slate-300 hover:border-accent hover:text-accent"
            aria-label="Back to source exchange"
            title="Navigate to the source exchange in the capture view"
          >
            ← back to source
          </button>
        )}
      </div>
      <div className="flex min-h-0 flex-1">
        <div className="flex w-1/2 flex-col overflow-hidden border-r border-slate-700">
          <ReplayRequestEditor tabId={activeId} />
          {historyOpen && (
            <div className="h-1/3 overflow-y-auto border-t border-slate-700">
              <div className="flex items-center justify-between bg-bg-rail px-2 py-1">
                <span className="text-xs text-slate-400">History</span>
                <button
                  type="button"
                  data-testid="replay-view-history-toggle"
                  onClick={() => setHistoryOpen(false)}
                  className="text-xs text-slate-500"
                >
                  ▼
                </button>
              </div>
              <ReplayHistoryPanel tabId={activeId} />
            </div>
          )}
        </div>
        <div className="w-1/2 overflow-y-auto">
          <ReplayResponseViewer response={tab.latestResponse} />
        </div>
      </div>
    </div>
  );
}
