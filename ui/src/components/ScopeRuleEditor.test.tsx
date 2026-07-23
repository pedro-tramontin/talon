// Vitest cases for the Phase 6 §6.6 `ScopeRuleEditor` component.
//
// The component is responsible for:
//   - showing an empty state when the project has no rules
//   - fetching the active project's rules on mount
//   - rendering each rule's kind / action / pattern / remove button
//   - adding a default rule via `addScopeRule` + optimistically
//     appending to the local store
//   - removing a rule via `removeScopeRule` + optimistically
//     filtering from the local store
//   - colour-coding the action chip per the spec
//   - disabling the Add button when no project is open
//
// The IPC layer is mocked at the `../api` boundary with
// `vi.mock`; the rest of the test exercises the component's
// own state machine (effects + handlers + UI slices).

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import {
  addScopeRule,
  listScopeRules,
  removeScopeRule,
} from "../api";
import { projectStore } from "../state/project";
import { uiStore } from "../state/ui";
import { ScopeRuleEditor } from "./ScopeRuleEditor";
import type { ScopeRule } from "../types/domain";

// Mock the Tauri IPC layer. The mock returns the same
// shape as the real `api.ts` wrappers, so the component
// doesn't need to know it's mocked.
vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return {
    ...actual,
    listScopeRules: vi.fn(),
    addScopeRule: vi.fn(),
    removeScopeRule: vi.fn(),
  };
});

const listScopeRulesMock = vi.mocked(listScopeRules);
const addScopeRuleMock = vi.mocked(addScopeRule);
const removeScopeRuleMock = vi.mocked(removeScopeRule);

// Reset state + mocks between tests so each test starts
// from the default store values.
beforeEach(() => {
  uiStore.setState({
    scopeRules: [],
    matchReplaceRules: [],
    settingsOpen: false,
  });
  projectStore.setState({
    activeProjectId: "proj-1" as never,
    projects: [
      {
        id: "proj-1" as never,
        name: "acme",
        target_host: "acme.bb",
        db_filename: "acme.db",
      },
    ],
  });
  listScopeRulesMock.mockReset();
  addScopeRuleMock.mockReset();
  removeScopeRuleMock.mockReset();
});

afterEach(() => {
  vi.clearAllMocks();
});

describe("ScopeRuleEditor", () => {
  it("renders the empty state when the project has no rules", async () => {
    listScopeRulesMock.mockResolvedValueOnce([]);
    render(<ScopeRuleEditor />);
    await waitFor(() => {
      expect(screen.getByTestId("scope-rule-editor-empty")).toBeDefined();
    });
    expect(
      screen.getByTestId("scope-rule-editor-empty").textContent,
    ).toContain("No rules");
  });

  it("renders the 'no project open' state when no project is active", async () => {
    projectStore.setState({ activeProjectId: null });
    render(<ScopeRuleEditor />);
    // No fetch should happen when there's no active project;
    // the empty hint should be the "no project" variant.
    expect(listScopeRulesMock).not.toHaveBeenCalled();
    await waitFor(() => {
      expect(screen.getByTestId("scope-rule-editor-empty")).toBeDefined();
    });
    expect(
      screen.getByTestId("scope-rule-editor-empty").textContent,
    ).toContain("No project open");
  });

  it("renders the rule list with kind / action / pattern / remove button", async () => {
    const rules: ScopeRule[] = [
      {
        kind: "host",
        pattern: "acme.bb",
        action: "in_scope",
        label: "acme in-scope",
        priority: 0,
      },
      {
        kind: "path_prefix",
        pattern: "/admin",
        action: "block",
        label: "admin block",
        priority: 10,
      },
    ];
    listScopeRulesMock.mockResolvedValueOnce(rules);
    render(<ScopeRuleEditor />);
    await waitFor(() => {
      expect(screen.getByTestId("scope-rule-row-0")).toBeDefined();
    });
    expect(screen.getByTestId("scope-rule-row-action-0").textContent).toBe(
      "in_scope",
    );
    expect(screen.getByTestId("scope-rule-row-pattern-0").textContent).toBe(
      "acme.bb",
    );
    expect(screen.getByTestId("scope-rule-row-1")).toBeDefined();
    expect(screen.getByTestId("scope-rule-row-action-1").textContent).toBe(
      "block",
    );
  });

  it("clicking + Add calls addScopeRule and updates the local store", async () => {
    listScopeRulesMock.mockResolvedValueOnce([]);
    addScopeRuleMock.mockResolvedValueOnce(undefined);
    render(<ScopeRuleEditor />);
    await waitFor(() => {
      expect(screen.getByTestId("scope-rule-editor-add")).toBeDefined();
    });
    await act(async () => {
      fireEvent.click(screen.getByTestId("scope-rule-editor-add"));
    });
    expect(addScopeRuleMock).toHaveBeenCalledTimes(1);
    // The optimistically-appended rule should now be in the store.
    const rules = uiStore.getState().scopeRules;
    expect(rules).toHaveLength(1);
    expect(rules[0].kind).toBe("host");
    expect(rules[0].action).toBe("in_scope");
  });

  it("clicking the row's remove button calls removeScopeRule and updates the local store", async () => {
    const rules: ScopeRule[] = [
      {
        kind: "host",
        pattern: "acme.bb",
        action: "in_scope",
        label: "first",
        priority: 0,
      },
      {
        kind: "host",
        pattern: "acme.bb",
        action: "out_of_scope",
        label: "second",
        priority: 0,
      },
    ];
    listScopeRulesMock.mockResolvedValueOnce(rules);
    removeScopeRuleMock.mockResolvedValueOnce(undefined);
    render(<ScopeRuleEditor />);
    await waitFor(() => {
      expect(screen.getByTestId("scope-rule-row-0")).toBeDefined();
    });
    await act(async () => {
      fireEvent.click(screen.getByTestId("scope-rule-row-remove-0"));
    });
    expect(removeScopeRuleMock).toHaveBeenCalledTimes(1);
    expect(removeScopeRuleMock).toHaveBeenCalledWith("proj-1", 0);
    const after = uiStore.getState().scopeRules;
    expect(after).toHaveLength(1);
    expect(after[0].label).toBe("second");
  });

  it("colour-codes the action chip per the spec (in_scope=green, out_of_scope=slate, block=red)", async () => {
    const rules: ScopeRule[] = [
      {
        kind: "host",
        pattern: "a",
        action: "in_scope",
        label: "a",
        priority: 0,
      },
      {
        kind: "host",
        pattern: "b",
        action: "out_of_scope",
        label: "b",
        priority: 0,
      },
      {
        kind: "host",
        pattern: "c",
        action: "block",
        label: "c",
        priority: 0,
      },
    ];
    listScopeRulesMock.mockResolvedValueOnce(rules);
    render(<ScopeRuleEditor />);
    await waitFor(() => {
      expect(screen.getByTestId("scope-rule-row-0")).toBeDefined();
    });
    expect(
      screen.getByTestId("scope-rule-row-action-0").className,
    ).toContain("bg-green-900/40");
    expect(
      screen.getByTestId("scope-rule-row-action-1").className,
    ).toContain("bg-slate-700");
    expect(
      screen.getByTestId("scope-rule-row-action-2").className,
    ).toContain("bg-red-900/40");
  });

  it("disables the Add button when no project is open", () => {
    projectStore.setState({ activeProjectId: null });
    render(<ScopeRuleEditor />);
    const btn = screen.getByTestId("scope-rule-editor-add") as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
  });

  // Phase 7 C-B.4: SecLists bulk-import.
  //
  // The file picker is hidden; clicking the
  // "Bulk import" button triggers `fileInputRef.click()`.
  // We simulate the user picking a file by firing a
  // `change` event on the input with a `File` payload.
  // `FileReader.readAsText` is replaced by the browser
  // `File.text()` Promise method (the editor uses the
  // same; jsdom supports it).

  it("renders the Bulk import button next to + Add", async () => {
    listScopeRulesMock.mockResolvedValueOnce([]);
    render(<ScopeRuleEditor />);
    await waitFor(() => {
      expect(
        screen.getByTestId("scope-rule-editor-bulk-import"),
      ).toBeDefined();
    });
  });

  it("clicking Bulk import triggers a click on the hidden file input", async () => {
    listScopeRulesMock.mockResolvedValueOnce([]);
    render(<ScopeRuleEditor />);
    await waitFor(() => {
      expect(
        screen.getByTestId("scope-rule-editor-bulk-import"),
      ).toBeDefined();
    });
    const fileInput = screen.getByTestId(
      "scope-rule-editor-bulk-import-file",
    ) as HTMLInputElement;
    const clickSpy = vi.spyOn(fileInput, "click");
    await act(async () => {
      fireEvent.click(screen.getByTestId("scope-rule-editor-bulk-import"));
    });
    expect(clickSpy).toHaveBeenCalledTimes(1);
  });

  it("uploading a valid file with 3 hosts calls addScopeRule 3 times and adds 3 rules to the store", async () => {
    listScopeRulesMock.mockResolvedValueOnce([]);
    addScopeRuleMock.mockResolvedValue(undefined);
    render(<ScopeRuleEditor />);
    await waitFor(() => {
      expect(
        screen.getByTestId("scope-rule-editor-bulk-import"),
      ).toBeDefined();
    });
    const file = new File(
      ["a.test\nb.test\nc.test\n"],
      "hosts.txt",
      { type: "text/plain" },
    );
    const fileInput = screen.getByTestId(
      "scope-rule-editor-bulk-import-file",
    ) as HTMLInputElement;
    // `Object.defineProperty` is needed because `files` is
    // a read-only `FileList` on a real `<input type="file">`.
    Object.defineProperty(fileInput, "files", {
      value: [file],
      configurable: true,
    });
    await act(async () => {
      fireEvent.change(fileInput);
    });
    await waitFor(() => {
      expect(addScopeRuleMock).toHaveBeenCalledTimes(3);
    });
    const rules = uiStore.getState().scopeRules;
    expect(rules).toHaveLength(3);
    expect(rules[0].pattern).toBe("a.test");
    expect(rules[1].pattern).toBe("b.test");
    expect(rules[2].pattern).toBe("c.test");
    expect(rules[0].label).toBe("imported");
  });

  it("uploading a file with 1 host + 2 comments calls addScopeRule 1 time", async () => {
    listScopeRulesMock.mockResolvedValueOnce([]);
    addScopeRuleMock.mockResolvedValue(undefined);
    render(<ScopeRuleEditor />);
    await waitFor(() => {
      expect(
        screen.getByTestId("scope-rule-editor-bulk-import"),
      ).toBeDefined();
    });
    const file = new File(
      ["# header\nonly.test\n# trailing\n"],
      "hosts.txt",
      { type: "text/plain" },
    );
    const fileInput = screen.getByTestId(
      "scope-rule-editor-bulk-import-file",
    ) as HTMLInputElement;
    Object.defineProperty(fileInput, "files", {
      value: [file],
      configurable: true,
    });
    await act(async () => {
      fireEvent.change(fileInput);
    });
    await waitFor(() => {
      expect(addScopeRuleMock).toHaveBeenCalledTimes(1);
    });
    expect(uiStore.getState().scopeRules[0].pattern).toBe("only.test");
  });

  it("uploading a file with 0 hosts (comments only) shows the 'no hosts' hint and does not call addScopeRule", async () => {
    listScopeRulesMock.mockResolvedValueOnce([]);
    addScopeRuleMock.mockResolvedValue(undefined);
    render(<ScopeRuleEditor />);
    await waitFor(() => {
      expect(
        screen.getByTestId("scope-rule-editor-bulk-import"),
      ).toBeDefined();
    });
    const file = new File(
      ["# only comments\n# more comments\n"],
      "hosts.txt",
      { type: "text/plain" },
    );
    const fileInput = screen.getByTestId(
      "scope-rule-editor-bulk-import-file",
    ) as HTMLInputElement;
    Object.defineProperty(fileInput, "files", {
      value: [file],
      configurable: true,
    });
    await act(async () => {
      fireEvent.change(fileInput);
    });
    await waitFor(() => {
      expect(
        screen.getByTestId("scope-rule-editor-bulk-import-error"),
      ).toBeDefined();
    });
    expect(
      screen.getByTestId("scope-rule-editor-bulk-import-error").textContent,
    ).toContain("No hosts");
    expect(addScopeRuleMock).not.toHaveBeenCalled();
  });
});
