// Vitest cases for the Phase 6 §6.7 `MatchReplaceEditor` component.
//
// The editor:
//   - shows the empty state when no project / no rules
//   - renders the 7-column table (target, pattern, replace,
//     regex, pri, enabled, remove) for each rule
//   - calls `addMatchReplaceRule` and appends on "+ Add rule"
//   - calls `removeMatchReplaceRule` and filters on "×"
//   - disables the Add button when no project is open
//
// The IPC layer is mocked at the `../api` boundary.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import {
  addMatchReplaceRule,
  listMatchReplaceRules,
  removeMatchReplaceRule,
} from "../api";
import { projectStore } from "../state/project";
import { uiStore } from "../state/ui";
import { MatchReplaceEditor } from "./MatchReplaceEditor";
import type { MatchReplaceRule } from "../types/domain";

vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return {
    ...actual,
    listMatchReplaceRules: vi.fn(),
    addMatchReplaceRule: vi.fn(),
    removeMatchReplaceRule: vi.fn(),
  };
});

const listMock = vi.mocked(listMatchReplaceRules);
const addMock = vi.mocked(addMatchReplaceRule);
const removeMock = vi.mocked(removeMatchReplaceRule);

beforeEach(() => {
  uiStore.setState({
    matchReplaceRules: [],
    scopeRules: [],
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
  listMock.mockReset();
  addMock.mockReset();
  removeMock.mockReset();
});

afterEach(() => {
  vi.clearAllMocks();
});

describe("MatchReplaceEditor", () => {
  it("renders the empty state when no rules", async () => {
    listMock.mockResolvedValueOnce([]);
    render(<MatchReplaceEditor />);
    await waitFor(() => {
      expect(screen.getByTestId("match-replace-editor-empty")).toBeDefined();
    });
    expect(screen.getByTestId("match-replace-editor-empty").textContent).toContain(
      "No rules",
    );
  });

  it("renders the 7-column table for each rule", async () => {
    const rules: MatchReplaceRule[] = [
      {
        target: "request_url",
        case_insensitive: false,
        is_regex: false,
        pattern: "/api/v1/",
        replace: "/api/v2/",
        enabled: true,
        priority: 5,
      },
    ];
    listMock.mockResolvedValueOnce(rules);
    render(<MatchReplaceEditor />);
    await waitFor(() => {
      expect(screen.getByTestId("match-replace-row-0")).toBeDefined();
    });
    expect(screen.getByTestId("match-replace-row-target-0").textContent).toBe(
      "request_url",
    );
    expect(screen.getByTestId("match-replace-row-pattern-0").textContent).toBe(
      "/api/v1/",
    );
    expect(screen.getByTestId("match-replace-row-replace-0").textContent).toBe(
      "/api/v2/",
    );
  });

  it("clicking + Add rule calls addMatchReplaceRule and updates the store", async () => {
    listMock.mockResolvedValueOnce([]);
    addMock.mockResolvedValueOnce(undefined);
    render(<MatchReplaceEditor />);
    await waitFor(() => {
      expect(screen.getByTestId("match-replace-editor-add")).toBeDefined();
    });
    await act(async () => {
      fireEvent.click(screen.getByTestId("match-replace-editor-add"));
    });
    expect(addMock).toHaveBeenCalledTimes(1);
    const rules = uiStore.getState().matchReplaceRules;
    expect(rules).toHaveLength(1);
    expect(rules[0].target).toBe("request_url");
    expect(rules[0].enabled).toBe(true);
  });

  it("clicking × on a row calls removeMatchReplaceRule and filters the store", async () => {
    const rules: MatchReplaceRule[] = [
      {
        target: "request_url",
        case_insensitive: false,
        is_regex: false,
        pattern: "/a",
        replace: "/b",
        enabled: true,
        priority: 0,
      },
      {
        target: "request_body",
        case_insensitive: false,
        is_regex: false,
        pattern: "/c",
        replace: "/d",
        enabled: false,
        priority: 0,
      },
    ];
    listMock.mockResolvedValueOnce(rules);
    removeMock.mockResolvedValueOnce(undefined);
    render(<MatchReplaceEditor />);
    await waitFor(() => {
      expect(screen.getByTestId("match-replace-row-0")).toBeDefined();
    });
    await act(async () => {
      fireEvent.click(screen.getByTestId("match-replace-row-remove-0"));
    });
    expect(removeMock).toHaveBeenCalledWith("proj-1", 0);
    const after = uiStore.getState().matchReplaceRules;
    expect(after).toHaveLength(1);
    expect(after[0].pattern).toBe("/c");
  });

  it("disables the Add button when no project is open", () => {
    projectStore.setState({ activeProjectId: null });
    render(<MatchReplaceEditor />);
    const btn = screen.getByTestId(
      "match-replace-editor-add",
    ) as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
  });

  // Phase 7 C-B.2: the "Test" button
  it("disables the Test button when the URL input is empty", async () => {
    listMock.mockResolvedValueOnce([]);
    render(<MatchReplaceEditor />);
    await waitFor(() => {
      expect(
        screen.getByTestId("match-replace-editor-test-button"),
      ).toBeDefined();
    });
    const btn = screen.getByTestId(
      "match-replace-editor-test-button",
    ) as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
  });

  it("clicking Test with a sample URL shows the engine's result", async () => {
    const rules: MatchReplaceRule[] = [
      {
        target: "request_url",
        case_insensitive: false,
        is_regex: false,
        pattern: "/api/v1",
        replace: "/api/v2",
        enabled: true,
        priority: 0,
      },
    ];
    listMock.mockResolvedValueOnce(rules);
    render(<MatchReplaceEditor />);
    await waitFor(() => {
      expect(screen.getByTestId("match-replace-row-0")).toBeDefined();
    });
    // Type the sample URL.
    const urlInput = screen.getByTestId(
      "match-replace-editor-test-url",
    ) as HTMLInputElement;
    await act(async () => {
      fireEvent.change(urlInput, {
        target: { value: "https://x.test/api/v1/users" },
      });
    });
    // Click Test.
    await act(async () => {
      fireEvent.click(screen.getByTestId("match-replace-editor-test-button"));
    });
    // The result is shown.
    const result = screen.getByTestId("match-replace-editor-test-result");
    expect(result.textContent).toBe("https://x.test/api/v2/users");
  });

  it("the Test result updates when rules change after a Test click", async () => {
    const rulesBefore: MatchReplaceRule[] = [
      {
        target: "request_url",
        case_insensitive: false,
        is_regex: false,
        pattern: "/old",
        replace: "/new",
        enabled: true,
        priority: 0,
      },
    ];
    listMock.mockResolvedValueOnce(rulesBefore);
    render(<MatchReplaceEditor />);
    await waitFor(() => {
      expect(screen.getByTestId("match-replace-row-0")).toBeDefined();
    });
    const urlInput = screen.getByTestId(
      "match-replace-editor-test-url",
    ) as HTMLInputElement;
    await act(async () => {
      fireEvent.change(urlInput, {
        target: { value: "https://x.test/old/api" },
      });
    });
    await act(async () => {
      fireEvent.click(screen.getByTestId("match-replace-editor-test-button"));
    });
    // Initial result.
    expect(
      screen.getByTestId("match-replace-editor-test-result").textContent,
    ).toBe("https://x.test/new/api");
    // Now change the rules (via `uiStore.setMatchReplaceRules`).
    // The useEffect on `[rules, testUrl]` re-runs the engine.
    const rulesAfter: MatchReplaceRule[] = [
      {
        target: "request_url",
        case_insensitive: false,
        is_regex: false,
        pattern: "/old",
        replace: "/renamed",
        enabled: true,
        priority: 0,
      },
    ];
    await act(async () => {
      uiStore.getState().setMatchReplaceRules(rulesAfter);
    });
    // The result updates without the user clicking Test again.
    expect(
      screen.getByTestId("match-replace-editor-test-result").textContent,
    ).toBe("https://x.test/renamed/api");
  });

  it("Test without a sample URL shows a placeholder, not a result", async () => {
    listMock.mockResolvedValueOnce([]);
    render(<MatchReplaceEditor />);
    await waitFor(() => {
      expect(screen.getByTestId("match-replace-editor-test-url")).toBeDefined();
    });
    // Type a URL but clear it again.
    const urlInput = screen.getByTestId(
      "match-replace-editor-test-url",
    ) as HTMLInputElement;
    await act(async () => {
      fireEvent.change(urlInput, { target: { value: "x" } });
    });
    expect(
      (screen.getByTestId("match-replace-editor-test-button") as HTMLButtonElement)
        .disabled,
    ).toBe(false);
    // Clear the URL.
    await act(async () => {
      fireEvent.change(urlInput, { target: { value: "" } });
    });
    // The button is disabled again.
    expect(
      (screen.getByTestId("match-replace-editor-test-button") as HTMLButtonElement)
        .disabled,
    ).toBe(true);
    // No result element rendered.
    expect(screen.queryByTestId("match-replace-editor-test-result")).toBeNull();
  });
});
