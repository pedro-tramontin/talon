// Replay history panel. Per-tab send history. Renders the
// entries in `tab.history` newest-first. Each row shows
// `#<index+1> <METHOD> <url> <status> <time>`. Clicking a
// row calls `setDraft(tabId, entry.request)` (LOADS, does
// NOT auto-send — the §5.5 spec calls this out: "click
// to load semantics"). The user can then click Send to
// re-send, or edit first.
//
// A "Fork" button per row (Phase 7 C-B.5) is a shortcut
// for the click-to-load flow: it calls the same
// `setDraft(tabId, entry.request)` so the user can then
// click Send (or edit first). The button is a UI
// affordance, not a separate IPC round-trip.
//
// Empty state: "No sends yet." (the vitest case pins
// this).
//
// Phase 5 — §5.5. Phase 7 C-B.5 — Fork button.

import { useReplayStore } from "../state/replay";

interface Props {
  tabId: string;
}

export function ReplayHistoryPanel({ tabId }: Props) {
  const tab = useReplayStore((s) => s.tabs.find((t) => t.id === tabId));
  const setDraft = useReplayStore((s) => s.setDraft);

  if (!tab || tab.history.length === 0) {
    return (
      <p
        data-testid="replay-history-panel-empty"
        className="p-3 text-xs text-slate-500"
      >
        No sends yet.
      </p>
    );
  }

  // Newest-first: iterate the history in reverse.
  return (
    <div data-testid="replay-history-panel" className="text-xs">
      <h3 className="mb-2 p-2 text-xs font-bold uppercase text-slate-400">
        History
      </h3>
      <div className="space-y-0">
        {[...tab.history].reverse().map((entry, i) => {
          const realIndex = tab.history.length - 1 - i;
          const status = entry.response?.status ?? 0;
          const statusClass =
            status < 300
              ? "text-green-400"
              : status < 400
                ? "text-yellow-400"
                : "text-red-400";
          return (
            <div
              key={i}
              data-testid="replay-history-panel-row"
              data-history-index={realIndex}
              onClick={() => setDraft(tabId, entry.request)}
              className="cursor-pointer border-b border-slate-800 px-3 py-1.5 hover:bg-bg-panel"
            >
              <div className="flex items-center gap-2">
                <span className="text-[10px] text-slate-500">
                  #{realIndex + 1}
                </span>
                <span className="font-mono text-slate-200">
                  {entry.request.method}
                </span>
                <span className="flex-1 truncate font-mono text-slate-400">
                  {entry.request.url}
                </span>
                {entry.response && (
                  <span className={statusClass}>{status}</span>
                )}
                <span className="text-[10px] text-slate-500">
                  {entry.timestamp.toLocaleTimeString()}
                </span>
                <button
                  type="button"
                  data-testid={`replay-history-panel-fork-${realIndex}`}
                  onClick={(e) => {
                    e.stopPropagation();
                    setDraft(tabId, entry.request);
                  }}
                  className="rounded border border-slate-700 px-2 py-0.5 text-[10px] text-accent hover:bg-slate-700"
                  aria-label="Fork this send"
                >
                  Fork
                </button>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
