// Vitest cases for the `JsonTreeView` component
// (Phase 7 C-B.5).
//
// The component renders a JSON value as a tree:
//   - primitives (string, number, boolean, null) are
//     inline `<span>`s with type-specific classes
//   - objects/arrays are nested lists with `▶`/`▼` toggles
//   - depth is capped at 10 levels (defensive)
//
// The cases mirror the `objective:` block's enumeration
// (2-3 cases for JsonTreeView).

import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { JsonTreeView } from "./JsonTreeView";

afterEach(() => {
  cleanup();
});

describe("JsonTreeView", () => {
  it("renders an empty object as {} inline", () => {
    render(<JsonTreeView value={{}} />);
    expect(
      screen.getByTestId("json-tree-view-empty-object"),
    ).toBeTruthy();
  });

  it("renders a nested object with key-value pairs", () => {
    const value = { a: { b: { c: 1 } } };
    render(<JsonTreeView value={value} />);
    expect(screen.getByTestId("json-tree-view-object")).toBeTruthy();
    expect(screen.getByTestId("json-tree-view-key-a")).toBeTruthy();
    expect(screen.getByTestId("json-tree-view-number-a.b.c")).toBeTruthy();
  });

  it("renders an array of primitives with numeric keys", () => {
    const value = [1, 2, 3];
    render(<JsonTreeView value={value} />);
    expect(screen.getByTestId("json-tree-view-array")).toBeTruthy();
    expect(screen.getByTestId("json-tree-view-number-0")).toBeTruthy();
    expect(screen.getByTestId("json-tree-view-number-1")).toBeTruthy();
  });

  it("caps the depth at 10 levels (defensive — no infinite recursion)", () => {
    // Build a 12-level deep object.
    const value: Record<string, unknown> = {};
    let cur: Record<string, unknown> = value;
    for (let i = 0; i < 12; i++) {
      const next: Record<string, unknown> = {};
      cur["n"] = next;
      cur = next;
    }
    cur["leaf"] = "x";
    render(<JsonTreeView value={value} />);
    // The 10th-level node should have hit the cap.
    expect(
      screen.getByTestId("json-tree-view-depth-capped-n.n.n.n.n.n.n.n.n.n"),
    ).toBeTruthy();
  });
});
