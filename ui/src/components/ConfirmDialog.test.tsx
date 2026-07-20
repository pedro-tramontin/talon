// Tests for `ConfirmDialog`. We exercise the destructive-tool
// double-confirm flow ("type DELETE") and the Allow / Deny buttons.
// The component reads `respondConfirm` from the agent store; we spy
// on that to assert the arguments the dialog passes through.

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ConfirmDialog } from "./ConfirmDialog";
import { agentStore } from "../state/agent";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async () => undefined),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => undefined),
}));

function resetStore() {
  agentStore.setState({
    runs: {},
    activeRunId: null,
    confirmTimeouts: new Map(),
  });
}

beforeEach(() => {
  resetStore();
});

afterEach(() => {
  resetStore();
});

describe("ConfirmDialog", () => {
  it("renders the tool name and pretty-printed args", () => {
    render(
      <ConfirmDialog
        runId="r1"
        toolName="talon_delete_exchange"
        args={{ id: "abc-123", force: true }}
      />,
    );
    expect(
      screen.getByTestId("confirm-dialog-tool-name").textContent,
    ).toBe("talon_delete_exchange");
    const argsPre = screen.getByTestId("confirm-dialog-args");
    expect(argsPre.textContent).toContain("abc-123");
    expect(argsPre.textContent).toContain("force");
  });

  it("disables Allow for a destructive tool until DELETE is typed", () => {
    render(
      <ConfirmDialog
        runId="r1"
        toolName="talon_delete_exchange"
        args={{ id: "abc" }}
      />,
    );
    const allow = screen.getByTestId("confirm-dialog-allow");
    expect(allow).toBeDisabled();
    const input = screen.getByTestId("confirm-dialog-destructive-input");
    fireEvent.change(input, { target: { value: "delete" } });
    expect(allow).toBeDisabled();
    fireEvent.change(input, { target: { value: "DELETE" } });
    expect(allow).not.toBeDisabled();
  });

  it("clicking Allow calls respondConfirm(true, remember)", async () => {
    const spy = vi
      .spyOn(agentStore.getState(), "respondConfirm")
      .mockResolvedValue();
    render(
      <ConfirmDialog
        runId="r1"
        toolName="talon_delete_exchange"
        args={{ id: "abc" }}
      />,
    );
    const input = screen.getByTestId("confirm-dialog-destructive-input");
    fireEvent.change(input, { target: { value: "DELETE" } });
    // Toggle remember on.
    fireEvent.click(screen.getByTestId("confirm-dialog-remember"));
    fireEvent.click(screen.getByTestId("confirm-dialog-allow"));
    await waitFor(() => expect(spy).toHaveBeenCalledWith("r1", true, true));
    spy.mockRestore();
  });

  it("clicking Deny calls respondConfirm(false, remember)", async () => {
    const spy = vi
      .spyOn(agentStore.getState(), "respondConfirm")
      .mockResolvedValue();
    render(
      <ConfirmDialog
        runId="r2"
        toolName="talon_delete_exchange"
        args={{ id: "abc" }}
      />,
    );
    fireEvent.click(screen.getByTestId("confirm-dialog-deny"));
    await waitFor(() => expect(spy).toHaveBeenCalledWith("r2", false, false));
    spy.mockRestore();
  });

  it("for a non-destructive tool, Allow is enabled without typing", () => {
    render(
      <ConfirmDialog
        runId="r3"
        toolName="talon_some_write_tool"
        args={{ ok: true }}
      />,
    );
    // No destructive input should be rendered.
    expect(
      screen.queryByTestId("confirm-dialog-destructive-input"),
    ).toBeNull();
    expect(screen.getByTestId("confirm-dialog-allow")).not.toBeDisabled();
  });
});
