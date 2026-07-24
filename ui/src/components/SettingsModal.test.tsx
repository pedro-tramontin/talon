// Vitest cases for the Phase 6 §6.7 `SettingsModal` component.
//
// v0.5+ post-batch gap-fix P2 #5 (2026-07-24): the
// modal was historically titled "Settings" but only
// contains the Match & Replace editor. The rename
// (P2 #5) is a label change, not a component rename —
// the `data-testid` and the `aria-label` were updated
// to reflect the new title. The "renders the title" +
// "aria-label" cases assert the new "Match & Replace"
// string.
//
// The modal:
//   - is unmounted when `settingsOpen` is false (the
//     `if (!settingsOpen) return null` gate)
//   - is mounted with the title + the MatchReplaceEditor
//     child when `settingsOpen` is true
//   - closes on overlay click
//   - does NOT close on inner-panel click (the
//     `e.stopPropagation` on the inner div)
//   - closes on the explicit "✕" close button
//   - shows the future-settings note (a `text-xs`
//     hint that the broader settings surface is a
//     future phase)

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { uiStore } from "../state/ui";
import { projectStore } from "../state/project";
import { SettingsModal } from "./SettingsModal";

// The MatchReplaceEditor is a sub-component; its IPC calls
// are mocked below to keep these tests focused on the
// modal's own click semantics.
vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return {
    ...actual,
    listMatchReplaceRules: vi.fn().mockResolvedValue([]),
    addMatchReplaceRule: vi.fn().mockResolvedValue(undefined),
    removeMatchReplaceRule: vi.fn().mockResolvedValue(undefined),
  };
});

beforeEach(() => {
  uiStore.setState({
    settingsOpen: false,
    matchReplaceRules: [],
    scopeRules: [],
  });
  projectStore.setState({
    activeProjectId: null,
    projects: [],
  });
});

afterEach(() => {
  vi.clearAllMocks();
});

describe("SettingsModal", () => {
  it("does not render anything when settingsOpen is false", () => {
    const { container } = render(<SettingsModal />);
    expect(container.firstChild).toBeNull();
    expect(screen.queryByTestId("settings-modal")).toBeNull();
  });

  it("renders the title + the MatchReplaceEditor when settingsOpen is true", () => {
    act(() => {
      uiStore.getState().setSettingsOpen(true);
    });
    render(<SettingsModal />);
    // P2 #5: the title is "Match & Replace" (was "Settings"
    // in v0.4). The character entity is decoded to `&`
    // by the DOM, so we assert the decoded string.
    expect(screen.getByTestId("settings-modal-title").textContent).toBe(
      "Match & Replace",
    );
    expect(screen.getByTestId("settings-modal")).toBeDefined();
    // P2 #5: the `aria-label` on the dialog also reflects
    // the new title (a11y is in sync with the visible
    // label; mismatches would break screen readers).
    expect(
      screen.getByTestId("settings-modal").getAttribute("aria-label"),
    ).toBe("Match and Replace");
    // The MatchReplaceEditor renders an add button + table;
    // both should be present in the modal.
    expect(screen.getByTestId("match-replace-editor")).toBeDefined();
    expect(screen.getByTestId("match-replace-editor-add")).toBeDefined();
  });

  it("shows the future-settings note when the modal is open", () => {
    act(() => {
      uiStore.getState().setSettingsOpen(true);
    });
    render(<SettingsModal />);
    // P2 #5: the small `text-xs` note documents the
    // future settings surface (theme, telemetry, etc.)
    // so the user knows the M&R editor is the only
    // settings section for now. Asserting on the
    // testid is enough; the prose is documentation.
    const note = screen.getByTestId("settings-modal-future-note");
    expect(note).toBeDefined();
    expect(note.textContent).toMatch(/future phase/);
  });

  it("closes on overlay click", () => {
    act(() => {
      uiStore.getState().setSettingsOpen(true);
    });
    render(<SettingsModal />);
    expect(screen.getByTestId("settings-modal")).toBeDefined();
    act(() => {
      fireEvent.click(screen.getByTestId("settings-modal-overlay"));
    });
    expect(uiStore.getState().settingsOpen).toBe(false);
  });

  it("does NOT close when the user clicks inside the modal panel", () => {
    act(() => {
      uiStore.getState().setSettingsOpen(true);
    });
    render(<SettingsModal />);
    act(() => {
      fireEvent.click(screen.getByTestId("settings-modal"));
    });
    // The inner-panel click must NOT bubble to the overlay.
    expect(uiStore.getState().settingsOpen).toBe(true);
  });

  it("closes when the user clicks the ✕ close button", () => {
    act(() => {
      uiStore.getState().setSettingsOpen(true);
    });
    render(<SettingsModal />);
    act(() => {
      fireEvent.click(screen.getByTestId("settings-modal-close"));
    });
    expect(uiStore.getState().settingsOpen).toBe(false);
  });
});
