// Settings modal. Phase 6 §6.7.
//
// A modal that wraps `<MatchReplaceEditor />`. The modal
// is shown / hidden via the `settingsOpen` flag in the
// UI store (set by the top-bar's "Settings" button, which
// is added to `<Capture />`'s top bar in the same PR).
//
// v0.5+ post-batch gap-fix P2 #5 (2026-07-24): the
// modal was historically titled "Settings" but only
// contains the Match & Replace editor. Renamed to
// "Match & Replace" so the title matches the body.
// The broader settings surface (theme, telemetry,
// proxy bind addr, CA management, scope defaults,
// agent timeout) is a future phase; the `aria-label`
// and the `data-testid` were kept as `settings-modal`
// for backward compat with the existing SettingsModal
// tests (the rename is a label change, not a component
// rename).
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
        aria-label="Match and Replace"
      >
        <div className="mb-4 flex items-center justify-between">
          <h2
            data-testid="settings-modal-title"
            className="text-lg font-bold text-slate-100"
          >
            Match &amp; Replace
          </h2>
          <button
            data-testid="settings-modal-close"
            onClick={() => setSettingsOpen(false)}
            className="text-slate-500 hover:text-slate-200"
            aria-label="Close Match and Replace"
          >
            ✕
          </button>
        </div>
        {/* v0.5+ post-batch gap-fix P2 #5: a small
         * `text-xs` note documenting the future settings
         * surface so the user knows the M&R editor is
         * the only settings section for now. */}
        <p
          data-testid="settings-modal-future-note"
          className="mb-3 text-xs text-slate-500"
        >
          Additional settings (theme, telemetry, proxy bind address,
          CA management, scope defaults, agent timeout) will be added
          in a future phase.
        </p>
        <h3 className="mb-2 text-sm font-bold text-slate-300">
          Rules
        </h3>
        <MatchReplaceEditor />
      </div>
    </div>
  );
}
