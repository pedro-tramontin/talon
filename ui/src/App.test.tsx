import { render, screen, waitFor } from "@testing-library/react";
import { vi, describe, it, expect, beforeEach } from "vitest";
import { App } from "./App";

// Mock the Tauri IPC + event bridge. The App (and the agent store
// it pulls in) call `invoke` for commands and `listen` for the
// `wire_event` channel. We stub both here so the test doesn't
// require a running Tauri runtime.
//
// v0.5+ post-batch gap-fix P3 #9 (2026-07-24): the App
// startup hook also calls `list_projects` to rehydrate
// `projectStore.projects`. The mock returns an empty
// array (the test doesn't depend on a project).
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async (cmd: string) => {
    if (cmd === "greet") {
      return { message: "Hello from Talon", version: "0.1.0" };
    }
    if (cmd === "list_projects") {
      return [];
    }
    return null;
  }),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => {
    // Return a no-op unlisten; the wire-bus subscribes to
    // `wire_event` on connect and never gets one in the test,
    // which is the right behavior for a "no events" smoke test.
    return () => undefined;
  }),
}));

describe("App", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders the Capture route and the greeting from Rust", async () => {
    render(<App />);
    // §4.3-4.4: the App renders the <Capture /> route directly
    // (no router in v0.1). The top bar's project dropdown is
    // the visible surface.
    expect(screen.getByTestId("capture-top-bar")).toBeInTheDocument();
    // The §3.5d greeting is preserved in a hidden testid so
    // the "IPC bridge alive" smoke check still works.
    await waitFor(() =>
      expect(screen.getByTestId("app-greeting").textContent).toMatch(
        /Hello from Talon/,
      ),
    );
  });
});
