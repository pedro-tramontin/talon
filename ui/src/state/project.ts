// Zustand store for the projects list + active project.
//
// Per the §4.3-4.4 spec, this is a per-feature store (not a
// global app store). The `projects` list is a snapshot of the
// currently-open projects as returned by the §4.1
// `open_project` Tauri command; in v0.1 there's no separate
// "list all projects on disk" Tauri command, so the list is
// populated by calls to `openProject` and pruned by
// `closeProject` / `removeProject`.
//
// The active project is the one the user has selected in the
// dropdown in the Capture top bar. §4.5 reads `activeProjectId`
// to drive the exchange-list query.

import { createStore, useStore } from "zustand";
import type { StoreApi } from "zustand/vanilla";
import type { ProjectMeta } from "../types/domain";
import type { ProjectId } from "../types/ids";

/** Top-level store shape. */
export type ProjectStore = {
  /** Snapshot of currently-open projects. */
  projects: ProjectMeta[];
  /** The project the user has selected. `null` means "no project". */
  activeProjectId: ProjectId | null;

  /** Replace the whole list (used after a refresh). */
  setProjects: (projects: ProjectMeta[]) => void;
  /** Mark `id` as the active project. */
  setActiveProject: (id: ProjectId | null) => void;
  /** Append a freshly-opened project. */
  addProject: (project: ProjectMeta) => void;
  /** Remove a project from the list (does NOT close it on disk). */
  removeProject: (id: ProjectId) => void;
};

function createProjectStore() {
  return createStore<ProjectStore>((set) => ({
    projects: [],
    activeProjectId: null,

    setProjects(projects) {
      set({ projects });
    },

    setActiveProject(id) {
      set({ activeProjectId: id });
    },

    addProject(project) {
      set((state) => {
        // Idempotent: if `id` is already in the list, replace the
        // existing entry (so a re-open with the same id picks up
        // the new `db_filename` / `target_host`). Otherwise
        // append.
        const without = state.projects.filter((p) => p.id !== project.id);
        return { projects: [...without, project] };
      });
    },

    removeProject(id) {
      set((state) => {
        const projects = state.projects.filter((p) => p.id !== id);
        // If the removed project was active, clear the active id
        // so the UI doesn't render stale data.
        const activeProjectId =
          state.activeProjectId === id ? null : state.activeProjectId;
        return { projects, activeProjectId };
      });
    },
  }));
}

// Singleton store for app-wide use.
export const projectStore: StoreApi<ProjectStore> = createProjectStore();

/**
 * React hook for the project store. Use with a selector to
 * limit re-renders to the slice you care about (e.g.
 * `useProjectStore((s) => s.activeProjectId)`).
 */
export function useProjectStore<T>(selector: (state: ProjectStore) => T): T {
  return useStore(projectStore, selector);
}
