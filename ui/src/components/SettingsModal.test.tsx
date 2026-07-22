// Vitest cases for the Phase 6 §6.7 `SettingsModal` component.
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
    expect(screen.getByTestId("settings-modal-title").textContent).toBe(
      "Settings",
    );
    expect(screen.getByTestId("settings-modal")).toBeDefined();
    // The MatchReplaceEditor renders an add button + table;
    // both should be present in the modal.
    expect(screen.getByTestId("match-replace-editor")).toBeDefined();
    expect(screen.getByTestId("match-replace-editor-add")).toBeDefined();
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
