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
    expect(optionLabels).toEqual([
      "— None —",
      "alpha",
      "beta",
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

  // Phase 8 (2026-07-23) — the New Project feature gap.
  // The "+" button next to the dropdown opens the modal
  // (the newProjectModalOpen UI store flag flips to true);
  // the Settings button still works (regression — the
  // adjacent button is unchanged).
  it("the '+ New' button opens the New Project modal", () => {
    render(<Capture />);
    expect(uiStore.getState().newProjectModalOpen).toBe(false);
    fireEvent.click(screen.getByTestId("capture-new-project-button"));
    expect(uiStore.getState().newProjectModalOpen).toBe(true);
  });

  it("the Settings button still opens the Settings modal (Phase 6 §6.7 regression)", () => {
    render(<Capture />);
    expect(uiStore.getState().settingsOpen).toBe(false);
    fireEvent.click(screen.getByTestId("capture-settings-button"));
    expect(uiStore.getState().settingsOpen).toBe(true);
  });
});
