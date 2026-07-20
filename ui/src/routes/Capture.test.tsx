// Tests for the Capture route (ui/src/routes/Capture.tsx).
//
// The route is the §4.3-4.4 placeholder: 3-column layout
// (left rail | main | right rail) with the project dropdown
// in the top bar. §4.5 fills the left rail; §4.6 fills the
// right-rail tabs. We assert the column widths and the
// placeholder empty states here.

import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { Capture, LEFT_RAIL_PX, RIGHT_RAIL_PX } from "./Capture";
import { projectStore } from "../state/project";
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

beforeEach(() => {
  resetProjectStore();
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
    const main = screen.getByTestId("capture-main");
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
    // readers find it.
    expect(main.tagName.toLowerCase()).toBe("main");
  });

  it("renders the empty state in the main column", () => {
    render(<Capture />);
    expect(screen.getByTestId("capture-main-empty")).toBeInTheDocument();
    expect(
      screen.getByTestId("capture-main-empty").textContent,
    ).toMatch(/Select an exchange to view details/);
  });

  it("renders the §4.5 placeholder text in the left rail", () => {
    render(<Capture />);
    const left = screen.getByTestId("capture-left-rail");
    expect(left.textContent).toMatch(/§4\.5/);
  });

  it("renders the 3 right-rail tabs (Inspector / Decoder / Notes)", () => {
    render(<Capture />);
    expect(
      screen.getByTestId("capture-right-rail-tab-inspector"),
    ).toBeInTheDocument();
    expect(
      screen.getByTestId("capture-right-rail-tab-decoder"),
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
});
