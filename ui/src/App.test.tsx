import { render, screen, waitFor } from "@testing-library/react";
import { vi, describe, it, expect, beforeEach } from "vitest";
import { App } from "./App";

// Mock the Tauri IPC + event bridge. The App (and the agent store
// it pulls in) call `invoke` for commands and `listen` for the
// `agent_event` channel. We stub both here so the test doesn't
// require a running Tauri runtime.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async (cmd: string) => {
    if (cmd === "greet") {
      return { message: "Hello from Talon", version: "0.1.0" };
    }
    return null;
  }),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => {
    // Return a no-op unlisten; the agent store subscribes to
    // `agent_event` on init and never gets one in the test, which
    // is the right behavior for a "no active run" smoke test.
    return () => undefined;
  }),
}));

describe("App", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders the title and the greeting from Rust", async () => {
    render(<App />);
    expect(screen.getByText("Talon")).toBeInTheDocument();
    await waitFor(() =>
      expect(screen.getByText(/Hello from Talon/)).toBeInTheDocument()
    );
  });
});
