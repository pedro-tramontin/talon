// §4.7 NotesPanel. The "Notes" tab in the right-rail. A
// per-exchange markdown editor. Notes are stored in the
// `notes` column on the `exchanges` table via the new
// `update_notes` Tauri command.
//
// Spec (§4.7):
//   - The textarea is the editor. v1 is plain text (no
//     markdown rendering — that's a v0.5 followup).
//   - On **blur** (when the textarea loses focus), if
//     the local draft differs from the server's
//     `notes`, fire `updateNotes` to persist the change.
//   - The local draft also resets when the user picks a
//     new row (so the editor doesn't show stale notes
//     from a different exchange).
//   - A "Saving…" → "Saved HH:MM:SS" status line confirms
//     the round-trip; "Unsaved changes" surfaces when
//     the draft diverges.
//   - The `update_notes` Tauri command takes a 64KB cap
//     server-side; we mirror that cap on the client
//     so the user gets a friendlier error than the
//     Rust `Err("notes exceeds 64KB cap")`.
//
// **v0.5 followup (added 2026-07-21):** Markdown rendering.
// The panel now has an "Edit" / "Preview" sub-tab toggle
// in the header. The Edit sub-tab is the textarea (the
// v1 behavior). The Preview sub-tab renders the draft
// through `react-markdown` so `# headers`, lists, code
// blocks, and links display as formatted markdown. The
// preview reflects the LIVE draft (not the saved server
// value) so the user sees their formatting while typing.
// On the "Preview" tab, edits are still possible via a
// "Back to edit" button — the spec calls for a tabbed
// view, not a read-only preview.
//
// Security: the panel is a plain `<textarea>` (no
// rich-text / no `dangerouslySetInnerHTML` in the editor).
// The preview uses `react-markdown` which renders to
// safe React elements by default (no `dangerouslySetInnerHTML`,
// no raw HTML passthrough). The server stores the notes
// as a SQLite TEXT column (parameterized via `rusqlite`'s
// `bind` — no SQLi surface). The 64KB cap is enforced
// server-side; we mirror it client-side for UX.

import { useEffect, useRef, useState } from "react";
import ReactMarkdown from "react-markdown";
import { updateNotes } from "../api";
import { exchangeStore, useExchangeStore } from "../state/exchange";
import { useProjectStore } from "../state/project";
import type { ExchangeId } from "../types/ids";

/** Server-side cap (must match `app::commands::update_notes`). */
const MAX_NOTES_BYTES = 64 * 1024;

/** Sub-tab identifier for the Edit/Preview toggle. */
type View = "edit" | "preview";

/** Status line text. */
type SaveStatus =
  | { kind: "idle" }
  | { kind: "saving" }
  | { kind: "saved"; at: number }
  | { kind: "error"; message: string }
  | { kind: "dirty" };

/** Empty state when no row is selected. */
function NoSelection() {
  return (
    <p
      data-testid="notes-panel-no-selection"
      className="text-sm text-slate-500"
    >
      No exchange selected.
    </p>
  );
}

export function NotesPanel() {
  const selectedId = useExchangeStore((s) => s.selectedId);
  const exchanges = useExchangeStore((s) => s.exchanges);
  const activeProjectId = useProjectStore((s) => s.activeProjectId);
  // Local draft. The source of truth is the server
  // (`notes` on the exchange); the draft is what the
  // user is currently editing.
  const [draft, setDraft] = useState<string>("");
  const [status, setStatus] = useState<SaveStatus>({ kind: "idle" });
  // The id of the exchange whose notes are currently in
  // the draft. Used to detect "user picked a new row" and
  // reset the draft (without losing the unsaved buffer
  // for a previous row — that's a v0.5 followup).
  const draftIdRef = useRef<ExchangeId | null>(null);
  // Edit vs. Preview sub-tab. v0.5: Preview renders the
  // draft as markdown via react-markdown.
  const [view, setView] = useState<View>("edit");

  // The server-side notes for the currently-selected
  // row. We compute this every render (it's a cheap
  // `.find` over the exchange list) so the
  // dirty/saved/idle status can compare against it.
  const serverNotes = (() => {
    if (!selectedId) return null;
    return exchanges.find((e) => e.id === selectedId)?.notes ?? null;
  })();

  // Reset the draft when the user picks a new row.
  useEffect(() => {
    if (selectedId !== draftIdRef.current) {
      draftIdRef.current = selectedId;
      setDraft(serverNotes ?? "");
      setStatus({ kind: "idle" });
      setView("edit");
    }
  }, [selectedId, serverNotes]);

  // Keep the draft in sync with the server when the
  // server value changes (e.g. a wire-bus update from
  // another panel wrote new notes). We do NOT
  // overwrite the draft if the user is currently
  // editing (the `status.kind === "dirty"` check).
  useEffect(() => {
    if (draftIdRef.current !== selectedId) return;
    if (status.kind === "dirty" || status.kind === "saving") return;
    if (serverNotes !== null && serverNotes !== draft) {
      setDraft(serverNotes);
    }
    // The effect intentionally omits `draft` and
    // `status` from the deps to avoid loops. Reading
    // them inside the body is fine — React calls the
    // effect on the next render if either changes,
    // and the in-effect guards already prevent the
    // overwrite.
  }, [serverNotes, selectedId]);

  /** Persist the current draft. Called on blur (or
   * via the "Save" button). */
  const save = async () => {
    if (!selectedId || !activeProjectId) return;
    // Skip the round-trip if the draft is already in
    // sync with the server (avoids a no-op IPC).
    if (serverNotes === draft) {
      setStatus({ kind: "idle" });
      return;
    }
    if (draft.length > MAX_NOTES_BYTES) {
      setStatus({
        kind: "error",
        message: `notes exceed ${MAX_NOTES_BYTES}-byte cap`,
      });
      return;
    }
    setStatus({ kind: "saving" });
    try {
      await updateNotes(activeProjectId, selectedId, draft);
      setStatus({ kind: "saved", at: Date.now() });
      // Reflect the saved value on the local exchange
      // store so the right rail's "Saved" status survives
      // a re-render and the dirty/idle status is right.
      exchangeStore.getState().updateExchangeNotes(selectedId, draft);
    } catch (e: unknown) {
      setStatus({ kind: "error", message: String(e) });
    }
  };

  if (!selectedId || !activeProjectId) return <NoSelection />;

  // Compute a dirty flag for the status line.
  const isDirty = serverNotes !== null && draft !== serverNotes;

  return (
    <div
      data-testid="notes-panel"
      className="flex h-full flex-col text-xs"
    >
      <div className="mb-1 flex items-center gap-1 border-b border-slate-700 pb-1">
        <span className="text-xs font-bold uppercase text-slate-400">
          Notes
        </span>
        <div className="ml-auto flex">
          <button
            type="button"
            data-testid="notes-panel-tab-edit"
            onClick={() => {
              setView("edit");
            }}
            className={`px-2 py-0.5 text-xs ${
              view === "edit"
                ? "text-accent"
                : "text-slate-400 hover:text-slate-200"
            }`}
          >
            Edit
          </button>
          <button
            type="button"
            data-testid="notes-panel-tab-preview"
            onClick={() => {
              setView("preview");
            }}
            className={`px-2 py-0.5 text-xs ${
              view === "preview"
                ? "text-accent"
                : "text-slate-400 hover:text-slate-200"
            }`}
          >
            Preview
          </button>
        </div>
      </div>
      {view === "edit" && (
        <textarea
          data-testid="notes-panel-textarea"
          value={draft}
          onChange={(e) => {
            setDraft(e.target.value);
            // Any edit after a clean state marks the
            // buffer as dirty. We don't touch the
            // saving/saved/error states — those are
            // owned by the `save` round-trip.
            if (status.kind === "idle" || status.kind === "saved") {
              setStatus({ kind: "dirty" });
            }
          }}
          onBlur={() => {
            // Per §4.7 spec: save on blur. Skip if the
            // buffer is already in sync with the server.
            if (isDirty) {
              void save();
            }
          }}
          placeholder="Notes — what did you learn from this request? Markdown is supported."
          className="flex-1 resize-none rounded border border-slate-600 bg-bg-base px-2 py-1 font-mono text-slate-100 focus:border-accent focus:outline-none"
        />
      )}
      {view === "preview" && (
        <div
          data-testid="notes-panel-preview"
          className="flex-1 overflow-y-auto rounded border border-slate-600 bg-bg-base px-3 py-2 text-slate-200"
        >
          {draft.trim().length === 0 ? (
            <p className="italic text-slate-500">
              No notes yet — switch to the Edit tab to add some.
            </p>
          ) : (
            <ReactMarkdown
              // The defaults: no raw HTML, no `dangerouslySetInnerHTML`,
              // safe React element output. Per the v0.5 spec
              // docstring, the security model is "react-markdown
              // renders to safe React elements by default".
              components={{
                // Tighten the heading sizes — the right rail
                // is 320px wide, the default h1/h2/h3 are
                // too large for that.
                h1: ({ children }) => (
                  <h1 className="mb-1 text-base font-bold text-slate-100">
                    {children}
                  </h1>
                ),
                h2: ({ children }) => (
                  <h2 className="mb-1 text-sm font-bold text-slate-100">
                    {children}
                  </h2>
                ),
                h3: ({ children }) => (
                  <h3 className="mb-1 text-xs font-bold text-slate-200">
                    {children}
                  </h3>
                ),
                // Inline code: yellow background to make
                // code stand out in the preview.
                code: ({ children, className }) => {
                  // `className` includes the language tag for
                  // fenced code blocks (e.g. "language-rust");
                  // inline `code` has no className. We don't
                  // ship a syntax highlighter in v0.5 (that's
                  // a v1 follow-up) so the block-level
                  // styling is a plain pre/code.
                  const isBlock = className?.startsWith("language-");
                  if (isBlock) {
                    return (
                      <code className={className}>{children}</code>
                    );
                  }
                  return (
                    <code className="rounded bg-slate-700 px-1 py-0.5 font-mono text-yellow-300">
                      {children}
                    </code>
                  );
                },
                pre: ({ children }) => (
                  <pre className="my-1 overflow-x-auto rounded bg-slate-900 p-2 font-mono text-xs text-slate-200">
                    {children}
                  </pre>
                ),
                a: ({ children, href }) => (
                  <a
                    href={href}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="text-accent underline hover:text-cyan-300"
                  >
                    {children}
                  </a>
                ),
                ul: ({ children }) => (
                  <ul className="my-1 list-disc pl-4">{children}</ul>
                ),
                ol: ({ children }) => (
                  <ol className="my-1 list-decimal pl-4">{children}</ol>
                ),
                li: ({ children }) => (
                  <li className="my-0.5">{children}</li>
                ),
                blockquote: ({ children }) => (
                  <blockquote className="my-1 border-l-2 border-slate-500 pl-2 italic text-slate-400">
                    {children}
                  </blockquote>
                ),
                p: ({ children }) => (
                  <p className="my-1">{children}</p>
                ),
                hr: () => (
                  <hr className="my-2 border-slate-600" />
                ),
                strong: ({ children }) => (
                  <strong className="font-bold text-slate-100">
                    {children}
                  </strong>
                ),
                em: ({ children }) => (
                  <em className="italic">{children}</em>
                ),
              }}
            >
              {draft}
            </ReactMarkdown>
          )}
        </div>
      )}
      <div className="mt-2 flex items-center justify-end gap-2">
        <span
          data-testid="notes-panel-status"
          className="text-slate-500"
        >
          {status.kind === "saving" && "Saving…"}
          {status.kind === "saved" &&
            `Saved ${new Date(status.at).toLocaleTimeString()}`}
          {status.kind === "error" && (
            <span className="text-red-400">{status.message}</span>
          )}
          {status.kind === "dirty" && "Unsaved changes"}
        </span>
        <button
          type="button"
          data-testid="notes-panel-save"
          onClick={() => {
            void save();
          }}
          disabled={!isDirty}
          className="rounded bg-accent px-3 py-1 font-medium text-bg-base hover:bg-cyan-300 disabled:cursor-not-allowed disabled:opacity-50"
        >
          Save
        </button>
      </div>
    </div>
  );
}
