// New Project modal. Phase 8 (2026-07-23).
//
// The Capture route's `ProjectDropdown` (in
// `ui/src/routes/Capture.tsx`) lets the user pick from
// `useProjectStore.projects`, but provides NO way to create
// or open a new project. The Rust `open_project` Tauri
// command at `app/src/commands/core.rs:144` and the
// `projectStore.addProject` action at
// `ui/src/state/project.ts:32` both exist; this modal wires
// them together.
//
// Click-to-close semantics (mirrors the v0.5 `SettingsModal`
// pattern):
//   - Click on the overlay (the dark backdrop) → close.
//   - Click on the inner panel → do NOT close (the inner
//     div has `stopPropagation` on click).
//   - Press Escape → close.
//   - Click Cancel → close.
//   - On successful submit, the modal closes itself.
//
// Validation:
//   - Inline validation on blur (not on every keystroke).
//   - `name` and `target_host` are both required
//     (non-empty after trim).
//   - `target_host` must pass the `isValidHostShape` TS
//     mirror of the Rust `is_valid_host_shape` (the Rust
//     validator stays the source of truth; the mirror is
//     for immediate client-side feedback before the Tauri
//     round-trip).
//   - The Create button is disabled until both fields pass
//     validation.
//   - On Tauri error (e.g. the Rust validator disagrees
//     with the mirror), the error message shows in a red
//     banner at the top of the modal. The modal does NOT
//     close on error — the user fixes the input and
//     re-submits.

import { useEffect, useState } from "react";
import { useUiStore } from "../state/ui";
import { useProjectStore } from "../state/project";
import { openProject } from "../api";
import { isValidHostShape } from "../lib/host_validation";

const MAX_NAME_LEN = 80;
const MAX_TARGET_HOST_LEN = 253;

/**
 * The "New Project" modal. Mounted at the top of `App.tsx`,
 * conditional on `useUiStore.newProjectModalOpen`.
 */
export function NewProjectModal() {
  const open = useUiStore((s) => s.newProjectModalOpen);
  const setOpen = useUiStore((s) => s.setNewProjectModalOpen);
  const addProject = useProjectStore((s) => s.addProject);
  const setActiveProject = useProjectStore((s) => s.setActiveProject);

  const [name, setName] = useState("");
  const [targetHost, setTargetHost] = useState("");
  // Validation is tracked separately for each field so we
  // can show an error on the field the user has already
  // touched (the v0.5 Settings modal pattern: "validate on
  // blur, not on every keystroke").
  const [nameTouched, setNameTouched] = useState(false);
  const [targetHostTouched, setTargetHostTouched] = useState(false);
  const [tauriError, setTauriError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  // Reset all form state when the modal opens or closes.
  // Without this, a Cancel-then-reopen would show the
  // user's previous (now stale) input.
  useEffect(() => {
    if (!open) {
      setName("");
      setTargetHost("");
      setNameTouched(false);
      setTargetHostTouched(false);
      setTauriError(null);
      setSubmitting(false);
    }
  }, [open]);

  // Escape key closes the modal. Listener is attached only
  // when the modal is open (avoids the listener firing for
  // every Escape press across the app).
  useEffect(() => {
    if (!open) return;
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") {
        e.preventDefault();
        setOpen(false);
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [open, setOpen]);

  if (!open) return null;

  const trimmedName = name.trim();
  const trimmedTargetHost = targetHost.trim();
  const nameError =
    nameTouched && trimmedName.length === 0 ? "Name is required" : null;
  const targetHostError =
    targetHostTouched && trimmedTargetHost.length === 0
      ? "Target host is required"
      : targetHostTouched && !isValidHostShape(trimmedTargetHost)
        ? "Target host must be a valid hostname (e.g. api.example.com) or IPv4 (e.g. 10.0.0.1)"
        : null;
  const canSubmit =
    !submitting &&
    trimmedName.length > 0 &&
    trimmedName.length <= MAX_NAME_LEN &&
    trimmedTargetHost.length > 0 &&
    trimmedTargetHost.length <= MAX_TARGET_HOST_LEN &&
    isValidHostShape(trimmedTargetHost);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!canSubmit) return;
    setSubmitting(true);
    setTauriError(null);
    try {
      const meta = await openProject(trimmedName, trimmedTargetHost);
      addProject(meta);
      setActiveProject(meta.id);
      setOpen(false);
    } catch (err) {
      setTauriError(String(err));
      setSubmitting(false);
    }
  }

  return (
    <div
      data-testid="new-project-modal-overlay"
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60"
      onClick={() => setOpen(false)}
      role="presentation"
    >
      <form
        data-testid="new-project-modal"
        onClick={(e) => e.stopPropagation()}
        onSubmit={handleSubmit}
        className="w-[480px] rounded border border-slate-700 bg-bg-panel p-6"
        role="dialog"
        aria-modal="true"
        aria-label="New Project"
      >
        <h2
          data-testid="new-project-modal-title"
          className="mb-4 text-center text-lg font-semibold text-slate-100"
        >
          New Project
        </h2>

        {tauriError && (
          <p
            data-testid="new-project-modal-error"
            className="mb-3 rounded border border-red-700 bg-red-900/30 px-3 py-2 text-sm text-red-200"
            role="alert"
          >
            {tauriError}
          </p>
        )}

        <div className="mb-3">
          <label
            htmlFor="new-project-modal-name"
            className="mb-1 block text-xs uppercase tracking-wide text-slate-400"
          >
            Name
          </label>
          <input
            id="new-project-modal-name"
            data-testid="new-project-modal-name"
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            onBlur={() => setNameTouched(true)}
            maxLength={MAX_NAME_LEN}
            placeholder="e.g. acme-web"
            className="w-full rounded border border-slate-700 bg-bg-base px-2 py-1 text-sm text-slate-100 focus:border-accent focus:outline-none"
          />
          {nameError && (
            <p
              data-testid="new-project-modal-name-error"
              className="mt-1 text-xs text-red-400"
            >
              {nameError}
            </p>
          )}
        </div>

        <div className="mb-4">
          <label
            htmlFor="new-project-modal-target-host"
            className="mb-1 block text-xs uppercase tracking-wide text-slate-400"
          >
            Target host
          </label>
          <input
            id="new-project-modal-target-host"
            data-testid="new-project-modal-target-host"
            type="text"
            value={targetHost}
            onChange={(e) => setTargetHost(e.target.value)}
            onBlur={() => setTargetHostTouched(true)}
            maxLength={MAX_TARGET_HOST_LEN}
            placeholder="api.acme.example.com"
            className="w-full rounded border border-slate-700 bg-bg-base px-2 py-1 font-mono text-sm text-slate-100 focus:border-accent focus:outline-none"
          />
          {targetHostError && (
            <p
              data-testid="new-project-modal-target-host-error"
              className="mt-1 text-xs text-red-400"
            >
              {targetHostError}
            </p>
          )}
        </div>

        <div className="flex justify-end gap-2">
          <button
            type="button"
            data-testid="new-project-modal-cancel"
            onClick={() => setOpen(false)}
            className="rounded border border-slate-600 bg-transparent px-3 py-1 text-sm text-slate-300 hover:border-slate-400"
          >
            Cancel
          </button>
          <button
            type="submit"
            data-testid="new-project-modal-create"
            disabled={!canSubmit}
            className="rounded bg-accent px-3 py-1 text-sm font-semibold text-bg-base hover:bg-accent-muted disabled:cursor-not-allowed disabled:opacity-50"
          >
            Create
          </button>
        </div>
      </form>
    </div>
  );
}
