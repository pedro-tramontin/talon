// Tests for the Capture route (ui/src/routes/Capture.tsx).
//
// The route is the §4.3-4.4 placeholder: 3-column layout
// (left rail | main | right rail) with the project dropdown
// in the top bar. §4.5 fills the left rail; §4.6 fills the
// right-rail tabs. We assert the column widths and the
// placeholder empty states here.

import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { Capture, LEFT_RAIL_PX, RIGHT_RAIL_PX } from "./Capture";
import { projectStore } from "../state/project";
import { uiStore } from "../state/ui";
import { asProjectId } from "../types/ids";
import type { ProjectMeta } from "../types/domain";

function makeProject(name: string): ProjectMeta {
  return {
    id: asProjectId(`00000000-0000-0000-0000-${name.padStart(12, "0")}`),
    name,
    target_host: "acme.example.com",
    db_filename: `${name}.db`,
  };
}

function resetProjectStore() {
  projectStore.setState({ projects: [], activeProjectId: null });
}

function resetUiStore() {
  uiStore.setState({
    newProjectModalOpen: false,
    settingsOpen: false,
  });
}

beforeEach(() => {
  resetProjectStore();
  resetUiStore();
});

describe("Capture route", () => {
  it("renders the top bar with the project dropdown", () => {
    render(<Capture />);
    expect(screen.getByTestId("capture-top-bar")).toBeInTheDocument();
    expect(
      screen.getByTestId("capture-project-select"),
    ).toBeInTheDocument();
  });

  it("renders the 3 columns with the expected widths", () => {
    render(<Capture />);
    const left = screen.getByTestId("capture-left-rail");
    const main = screen.getByTestId("exchange-detail-empty");
    const right = screen.getByTestId("capture-right-rail");

    // The width is set as an inline `style` so it survives
    // the Tailwind purge. We assert the px values against
    // the exported constants so the test catches
    // accidental width drift.
    expect(left.getAttribute("style")).toContain(
      `width: ${LEFT_RAIL_PX}px`,
    );
    expect(right.getAttribute("style")).toContain(
      `width: ${RIGHT_RAIL_PX}px`,
    );
    // The main column is `flex-1` (no fixed width). It
    // should be present and a `<main>` element so screen
    // readers find it. §4.6 wires `<ExchangeDetail />`
    // here, which renders an empty-state `<main>` when no
    // row is selected — that's the testid we assert.
    expect(main.tagName.toLowerCase()).toBe("main");
  });

  it("renders the empty state in the main column", () => {
    render(<Capture />);
    expect(screen.getByTestId("exchange-detail-empty")).toBeInTheDocument();
    expect(
      screen.getByTestId("exchange-detail-empty").textContent,
    ).toMatch(/Select an exchange to view its request and response/);
  });

  it("renders the virtualized exchange list in the left rail (lands in §4.5)", () => {
    render(<Capture />);
    // The §4.5 virtualized list owns the filter input; its
    // presence in the left rail confirms the placeholder
    // has been replaced.
    const left = screen.getByTestId("capture-left-rail");
    expect(
      left.querySelector('[data-testid="exchange-list"]'),
    ).toBeInTheDocument();
    expect(
      screen.getByTestId("exchange-list-filter"),
    ).toBeInTheDocument();
  });

  it("renders the 4 right-rail tabs (Inspector / Decoder / Diff / Notes)", () => {
    render(<Capture />);
    expect(
      screen.getByTestId("capture-right-rail-tab-inspector"),
    ).toBeInTheDocument();
    expect(
      screen.getByTestId("capture-right-rail-tab-decoder"),
    ).toBeInTheDocument();
    expect(
      screen.getByTestId("capture-right-rail-tab-diff"),
    ).toBeInTheDocument();
    expect(
      screen.getByTestId("capture-right-rail-tab-notes"),
    ).toBeInTheDocument();
  });

  it("the project dropdown lists the projects from the store", () => {
    projectStore.setState({
      projects: [makeProject("alpha"), makeProject("beta")],
      activeProjectId: null,
    });
    render(<Capture />);
    const select = screen.getByTestId(
      "capture-project-select",
    ) as HTMLSelectElement;
    const optionLabels = Array.from(select.options).map((o) => o.textContent);
    // Phase 8 (full v1): a "New..." sentinel item is
    // appended after the project list. This is the
    // §8.4 spec's "switch project" UI affordance.
    expect(optionLabels).toEqual([
      "— None —",
      "alpha",
      "beta",
      "New...",
    ]);
  });

  it("the project dropdown reflects the active project", () => {
    const a = makeProject("alpha");
    projectStore.setState({
      projects: [a],
      activeProjectId: a.id,
    });
    render(<Capture />);
    const select = screen.getByTestId(
      "capture-project-select",
    ) as HTMLSelectElement;
    expect(select.value).toBe(a.id);
  });

  // Phase 8 (full v1) — the "New..." dropdown item
  // opens the NewProjectModal (mirrors the "+ New"
  // button's behavior, but inside the dropdown so
  // the affordance is discoverable from the same
  // surface as the project list).
  it("the 'New...' dropdown item opens the NewProjectModal", () => {
    render(<Capture />);
    expect(uiStore.getState().newProjectModalOpen).toBe(false);
    const select = screen.getByTestId(
      "capture-project-select",
    ) as HTMLSelectElement;
    // The New... option's value is the sentinel
    // "__new__" (defined inside the component).
    fireEvent.change(select, { target: { value: "__new__" } });
    expect(uiStore.getState().newProjectModalOpen).toBe(true);
  });

  // Phase 8 (full v1) — the dropdown returns to the
  // previously-selected project after the New...
  // sentinel is chosen (so cancelling the modal
  // doesn't leave the dropdown in a "broken" state).
  it("the dropdown returns to the previously-selected project after choosing 'New...'", () => {
    const a = makeProject("alpha");
    projectStore.setState({
      projects: [a],
      activeProjectId: a.id,
    });
    render(<Capture />);
    const select = screen.getByTestId(
      "capture-project-select",
    ) as HTMLSelectElement;
    expect(select.value).toBe(a.id);
    fireEvent.change(select, { target: { value: "__new__" } });
    // After choosing New..., the dropdown should snap
    // back to the previously-selected project (the
    // modal opens separately).
    expect(select.value).toBe(a.id);
  });

  // Phase 8 (2026-07-23) — the New Project feature gap.
  // The "+" button next to the dropdown opens the modal
  // (the newProjectModalOpen UI store flag flips to true);
  // the Settings button still works (regression — the
  // adjacent button is unchanged). Per the per-item
  // `objective:` block (case 10), this is one combined
  // case: both buttons drive their respective modal flags.
  it("the '+ New' button opens the New Project modal, and the Settings button still works", () => {
    render(<Capture />);
    expect(uiStore.getState().newProjectModalOpen).toBe(false);
    expect(uiStore.getState().settingsOpen).toBe(false);
    fireEvent.click(screen.getByTestId("capture-new-project-button"));
    expect(uiStore.getState().newProjectModalOpen).toBe(true);
    fireEvent.click(screen.getByTestId("capture-settings-button"));
    expect(uiStore.getState().settingsOpen).toBe(true);
  });
});
