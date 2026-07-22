// Settings modal. Phase 6 §6.7.
//
// A modal that wraps `<MatchReplaceEditor />`. The modal
// is shown / hidden via the `settingsOpen` flag in the
// UI store (set by the top-bar's "Settings" button, which
// is added to `<Capture />`'s top bar in the same PR).
//
// Click-to-close semantics:
//   - Click on the overlay (the dark backdrop) → close.
//   - Click on the inner panel → do NOT close (the inner
//     div has `stopPropagation` on click; without it a
//     click on a button inside the panel would bubble up
//     to the overlay and close the modal unexpectedly).
//   - Click on the "✕" button → close.
//
// The modal is unmounted from the DOM when `settingsOpen`
// is false (the early `if (!settingsOpen) return null`
// gate). This matches the spec's spec'd behaviour.

import { useUiStore } from "../state/ui";
import { MatchReplaceEditor } from "./MatchReplaceEditor";

export function SettingsModal() {
  const settingsOpen = useUiStore((s) => s.settingsOpen);
  const setSettingsOpen = useUiStore((s) => s.setSettingsOpen);
  if (!settingsOpen) return null;
  return (
    <div
      data-testid="settings-modal-overlay"
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60"
      onClick={() => setSettingsOpen(false)}
      role="presentation"
    >
      <div
        data-testid="settings-modal"
        className="max-h-[80vh] w-[800px] overflow-y-auto rounded border border-slate-700 bg-bg-panel p-6"
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
        aria-label="Settings"
      >
        <div className="mb-4 flex items-center justify-between">
          <h2
            data-testid="settings-modal-title"
            className="text-lg font-bold text-slate-100"
          >
            Settings
          </h2>
          <button
            data-testid="settings-modal-close"
            onClick={() => setSettingsOpen(false)}
            className="text-slate-500 hover:text-slate-200"
            aria-label="Close settings"
          >
            ✕
          </button>
        </div>
        <h3 className="mb-2 text-sm font-bold text-slate-300">
          Match &amp; Replace
        </h3>
        <MatchReplaceEditor />
      </div>
    </div>
  );
}
