// Vitest cases for the Phase 8 `NewProjectModal` component.
//
// The modal:
//   - is unmounted when `newProjectModalOpen` is false
//   - renders the title + 2 inputs + 2 buttons when open
//   - the Create button is disabled until both fields are
//     non-empty AND the target_host passes `isValidHostShape`
//   - on successful submit: calls `openProject` (Tauri
//     IPC), then `addProject` + `setActiveProject`, and
//     closes the modal
//   - on Tauri error: shows the error in a red banner and
//     the modal stays open
//   - closes on overlay click + Escape + Cancel
//
// The IPC layer is mocked at the `../api` boundary.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { uiStore } from "../state/ui";
import { projectStore } from "../state/project";
import { NewProjectModal } from "./NewProjectModal";
import { openProject } from "../api";
import type { ProjectMeta } from "../types/domain";
import { asProjectId } from "../types/ids";

vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return {
    ...actual,
    openProject: vi.fn(),
  };
});

const openProjectMock = vi.mocked(openProject);

function makeMeta(name: string, targetHost: string): ProjectMeta {
  return {
    id: asProjectId(`00000000-0000-0000-0000-${name.padStart(12, "0")}`),
    name,
    target_host: targetHost,
    db_filename: `${name}.db`,
  };
}

beforeEach(() => {
  uiStore.setState({
    newProjectModalOpen: false,
    settingsOpen: false,
  });
  projectStore.setState({
    activeProjectId: null,
    projects: [],
  });
  openProjectMock.mockReset();
});

afterEach(() => {
  vi.clearAllMocks();
});

describe("NewProjectModal", () => {
  it("does not render anything when newProjectModalOpen is false", () => {
    const { container } = render(<NewProjectModal />);
    expect(container.firstChild).toBeNull();
    expect(screen.queryByTestId("new-project-modal")).toBeNull();
  });

  it("disables the Create button when either field is empty or invalid", () => {
    // The Create button must be disabled until both fields
    // are non-empty AND target_host passes isValidHostShape.
    // Covers the spec's cases 2, 3, 4 in one parameterized
    // assertion.
    act(() => {
      uiStore.getState().setNewProjectModalOpen(true);
    });
    render(<NewProjectModal />);
    const nameInput = screen.getByTestId("new-project-modal-name");
    const hostInput = screen.getByTestId("new-project-modal-target-host");
    const create = screen.getByTestId("new-project-modal-create");
    // Both fields empty.
    expect(create).toBeDisabled();
    // Name filled, host empty.
    act(() => {
      fireEvent.change(nameInput, { target: { value: "acme-web" } });
    });
    expect(create).toBeDisabled();
    // Both filled but host fails isValidHostShape.
    act(() => {
      fireEvent.change(hostInput, { target: { value: "foo bar" } });
      fireEvent.blur(hostInput);
    });
    expect(create).toBeDisabled();
    expect(
      screen.getByTestId("new-project-modal-target-host-error"),
    ).toBeInTheDocument();
  });

  it("on successful submit: calls openProject, adds + selects, closes the modal", async () => {
    const meta = makeMeta("acme-web", "api.acme.example.com");
    openProjectMock.mockResolvedValueOnce(meta);
    act(() => {
      uiStore.getState().setNewProjectModalOpen(true);
    });
    render(<NewProjectModal />);
    act(() => {
      fireEvent.change(screen.getByTestId("new-project-modal-name"), {
        target: { value: "  acme-web  " },
      });
      fireEvent.blur(screen.getByTestId("new-project-modal-name"));
    });
    act(() => {
      fireEvent.change(screen.getByTestId("new-project-modal-target-host"), {
        target: { value: "api.acme.example.com" },
      });
      fireEvent.blur(screen.getByTestId("new-project-modal-target-host"));
    });
    const create = screen.getByTestId("new-project-modal-create");
    expect(create).not.toBeDisabled();
    act(() => {
      fireEvent.click(create);
    });
    await waitFor(() => {
      expect(openProjectMock).toHaveBeenCalledWith("acme-web", "api.acme.example.com");
    });
    // After the awaited promise resolves, the store + close
    // should land.
    await waitFor(() => {
      expect(projectStore.getState().projects).toContainEqual(meta);
      expect(projectStore.getState().activeProjectId).toBe(meta.id);
      expect(uiStore.getState().newProjectModalOpen).toBe(false);
    });
  });

  it("on Tauri error: displays the error in a red banner and the modal stays open", async () => {
    openProjectMock.mockRejectedValueOnce(
      "target_host \"foo bar\" is not a valid hostname or IPv4 literal",
    );
    act(() => {
      uiStore.getState().setNewProjectModalOpen(true);
    });
    render(<NewProjectModal />);
    act(() => {
      fireEvent.change(screen.getByTestId("new-project-modal-name"), {
        target: { value: "acme-web" },
      });
      fireEvent.change(screen.getByTestId("new-project-modal-target-host"), {
        target: { value: "api.acme.example.com" },
      });
    });
    const create = screen.getByTestId("new-project-modal-create");
    expect(create).not.toBeDisabled();
    act(() => {
      fireEvent.click(create);
    });
    // The error banner appears + the modal stays open.
    await waitFor(() => {
      expect(
        screen.getByTestId("new-project-modal-error"),
      ).toBeInTheDocument();
    });
    expect(
      screen.getByTestId("new-project-modal-error").textContent,
    ).toContain("is not a valid hostname");
    // The store was NOT mutated; the modal is still open.
    expect(projectStore.getState().projects).toEqual([]);
    expect(projectStore.getState().activeProjectId).toBeNull();
    expect(uiStore.getState().newProjectModalOpen).toBe(true);
  });
});
