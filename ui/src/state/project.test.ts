// Tests for `useProjectStore` (ui/src/state/project.ts).
//
// The store is a global module-level singleton. Each test
// resets it via `projectStore.setState` so tests are
// independent.

import { beforeEach, describe, expect, it } from "vitest";
import { projectStore } from "./project";
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

function resetStore() {
  projectStore.setState({
    projects: [],
    activeProjectId: null,
  });
}

beforeEach(() => {
  resetStore();
});

describe("useProjectStore", () => {
  it("starts with an empty list and no active project", () => {
    expect(projectStore.getState().projects).toEqual([]);
    expect(projectStore.getState().activeProjectId).toBeNull();
  });

  it("setProjects replaces the whole list", () => {
    const list = [makeProject("alpha"), makeProject("beta")];
    projectStore.getState().setProjects(list);
    expect(projectStore.getState().projects).toEqual(list);
  });

  it("addProject appends a new project", () => {
    projectStore.getState().addProject(makeProject("alpha"));
    projectStore.getState().addProject(makeProject("beta"));
    expect(projectStore.getState().projects.map((p) => p.name)).toEqual([
      "alpha",
      "beta",
    ]);
  });

  it("addProject replaces an existing entry by id (idempotent)", () => {
    const a = makeProject("alpha");
    projectStore.getState().addProject(a);
    const aV2 = { ...a, target_host: "new.acme.example.com" };
    projectStore.getState().addProject(aV2);
    const projects = projectStore.getState().projects;
    expect(projects).toHaveLength(1);
    expect(projects[0].target_host).toBe("new.acme.example.com");
  });

  it("removeProject drops the project by id", () => {
    projectStore.getState().addProject(makeProject("alpha"));
    projectStore.getState().addProject(makeProject("beta"));
    const alphaId = projectStore
      .getState()
      .projects.find((p) => p.name === "alpha")!.id;
    projectStore.getState().removeProject(alphaId);
    const projects = projectStore.getState().projects;
    expect(projects).toHaveLength(1);
    expect(projects[0].name).toBe("beta");
  });

  it("removeProject clears activeProjectId if the removed project was active", () => {
    const a = makeProject("alpha");
    projectStore.getState().addProject(a);
    projectStore.getState().setActiveProject(a.id);
    expect(projectStore.getState().activeProjectId).toBe(a.id);
    projectStore.getState().removeProject(a.id);
    expect(projectStore.getState().activeProjectId).toBeNull();
  });

  it("setActiveProject sets the active id", () => {
    const a = makeProject("alpha");
    projectStore.getState().addProject(a);
    projectStore.getState().setActiveProject(a.id);
    expect(projectStore.getState().activeProjectId).toBe(a.id);
  });

  it("setActiveProject(null) clears the active id", () => {
    const a = makeProject("alpha");
    projectStore.getState().addProject(a);
    projectStore.getState().setActiveProject(a.id);
    projectStore.getState().setActiveProject(null);
    expect(projectStore.getState().activeProjectId).toBeNull();
  });

  it("useProjectStore selector returns the requested slice", () => {
    const a = makeProject("alpha");
    projectStore.getState().addProject(a);
    projectStore.getState().setActiveProject(a.id);
    // The hook itself is just a thin wrapper; we assert the
    // selector behavior on the underlying store.
    const sel = (s: ReturnType<typeof projectStore.getState>) => s.activeProjectId;
    expect(sel(projectStore.getState())).toBe(a.id);
  });
});
