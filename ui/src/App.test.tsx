import { render, screen, waitFor } from "@testing-library/react";
import { vi, describe, it, expect, beforeEach } from "vitest";
import { App } from "./App";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async () => ({ message: "Hello from Talon", version: "0.1.0" })),
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
