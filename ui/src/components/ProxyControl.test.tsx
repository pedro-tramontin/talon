// Vitest cases for the v0.5+ post-batch gap-fix `ProxyControl`
// component (`ui/src/components/ProxyControl.tsx`).
//
// Coverage target (per the per-item `objective:` block):
//   - 3-5 vitest cases that cover the click flow + the
//     status pill text + the `rulesActive` badge.
//   - Loose mode-A per v0.3.42; the §5b helper-test
//     tolerance applies.
//
// The component talks to the backend via `../api`
// (`startProxy` / `stopProxy` / `proxyStatus`); all three
// are mocked at the api boundary. The Zustand
// `proxyStore` + `uiStore` are real (they're the integration
// point — the test resets them in `beforeEach`).

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { proxyStore } from "../state/proxy";
import { uiStore } from "../state/ui";
import { ProxyControl } from "./ProxyControl";
import { startProxy, stopProxy, proxyStatus } from "../api";

vi.mock("../api", async () => {
  const actual = await vi.importActual<typeof import("../api")>("../api");
  return {
    ...actual,
    startProxy: vi.fn(),
    stopProxy: vi.fn(),
    proxyStatus: vi.fn(),
  };
});

const startProxyMock = vi.mocked(startProxy);
const stopProxyMock = vi.mocked(stopProxy);
const proxyStatusMock = vi.mocked(proxyStatus);

function resetStores() {
  proxyStore.setState({
    status: {
      state: "stopped",
      listener_addr: null,
      ca_fingerprint: null,
      last_error: null,
    },
  });
  uiStore.setState({
    scopeRules: [],
    matchReplaceRules: [],
  });
  startProxyMock.mockReset();
  stopProxyMock.mockReset();
  proxyStatusMock.mockReset();
}

beforeEach(() => {
  resetStores();
  // Default: the initial `proxyStatus()` call returns the
  // canonical "stopped" state. Each test overrides this if
  // it needs a different initial state.
  proxyStatusMock.mockResolvedValue({
    state: "stopped",
    listener_addr: null,
    ca_fingerprint: null,
    last_error: null,
  });
});

afterEach(() => {
  vi.clearAllMocks();
});

describe("ProxyControl", () => {
  it("renders a Start button when the proxy is stopped", async () => {
    render(<ProxyControl />);
    // The initial `proxyStatus()` is in-flight; wait for
    // the store to settle.
    await waitFor(() => {
      expect(proxyStore.getState().status.state).toBe("stopped");
    });
    const button = screen.getByTestId("proxy-control-toggle");
    expect(button.textContent).toBe("Start proxy");
    expect(button).not.toBeDisabled();
    expect(screen.getByTestId("proxy-control-pill-text").textContent).toBe(
      "Stopped",
    );
  });

  it("clicking Start calls startProxy, then refreshes status, and shows a running pill with the listener_addr", async () => {
    // Pre-set the post-click status the mock should
    // return so the test exercises the "running" UI.
    startProxyMock.mockResolvedValue(undefined);
    proxyStatusMock.mockResolvedValueOnce({
      state: "stopped",
      listener_addr: null,
      ca_fingerprint: null,
      last_error: null,
    });
    proxyStatusMock.mockResolvedValueOnce({
      state: "running",
      listener_addr: "127.0.0.1:8080",
      ca_fingerprint: "ab:cd:ef:01:23:45:67:89:ab:cd:ef:01:23:45:67:89:ab:cd:ef:01:23:45:67:89:01:23:45",
      last_error: null,
    });
    render(<ProxyControl />);
    await waitFor(() => {
      expect(proxyStore.getState().status.state).toBe("stopped");
    });
    const button = screen.getByTestId("proxy-control-toggle");
    fireEvent.click(button);
    await waitFor(() => {
      expect(startProxyMock).toHaveBeenCalledTimes(1);
    });
    // After the IPC round-trip + the post-click
    // proxyStatus refresh, the store should be in
    // 'running' state and the pill should reflect it.
    await waitFor(() => {
      expect(screen.getByTestId("proxy-control-pill-text").textContent).toBe(
        "Running on 127.0.0.1:8080",
      );
    });
    expect(screen.getByTestId("proxy-control-toggle").textContent).toBe("Stop");
  });

  it("clicking Stop calls stopProxy and shows the stopped pill", async () => {
    // Pre-seed the store as running so the button shows Stop.
    proxyStore.setState({
      status: {
        state: "running",
        listener_addr: "127.0.0.1:8080",
        ca_fingerprint: null,
        last_error: null,
      },
    });
    stopProxyMock.mockResolvedValue(undefined);
    proxyStatusMock.mockResolvedValue({
      state: "stopped",
      listener_addr: null,
      ca_fingerprint: null,
      last_error: null,
    });
    render(<ProxyControl />);
    const button = screen.getByTestId("proxy-control-toggle");
    expect(button.textContent).toBe("Stop");
    fireEvent.click(button);
    await waitFor(() => {
      expect(stopProxyMock).toHaveBeenCalledTimes(1);
    });
    await waitFor(() => {
      expect(screen.getByTestId("proxy-control-pill-text").textContent).toBe(
        "Stopped",
      );
    });
    expect(screen.getByTestId("proxy-control-toggle").textContent).toBe(
      "Start proxy",
    );
  });

  it("an error from startProxy surfaces an error pill with a Retry button", async () => {
    startProxyMock.mockRejectedValueOnce(new Error("bind: address already in use"));
    // The defensive `proxyStatus()` call after the failed
    // start should not be reached (the catch branch
    // sets the error state directly), but if it is, the
    // mock returns the default 'stopped' status (no
    // listener, no error). The pessimistic 'error' state
    // is what we want to assert.
    render(<ProxyControl />);
    await waitFor(() => {
      expect(proxyStore.getState().status.state).toBe("stopped");
    });
    const button = screen.getByTestId("proxy-control-toggle");
    fireEvent.click(button);
    await waitFor(() => {
      expect(proxyStore.getState().status.state).toBe("error");
    });
    await waitFor(() => {
      expect(screen.getByTestId("proxy-control-pill-text").textContent).toMatch(
        /^Error: bind: address already in use/,
      );
    });
    expect(screen.getByTestId("proxy-control-toggle").textContent).toBe("Retry");
  });

  it("the rulesActive badge shows the count of scopeRules + matchReplaceRules", () => {
    act(() => {
      uiStore.setState({
        scopeRules: [
          {
            kind: "host",
            pattern: "api.example.com",
            action: "in_scope",
            label: "api",
            priority: 1,
          },
          {
            kind: "host",
            pattern: "metrics.example.com",
            action: "out_of_scope",
            label: "metrics",
            priority: 1,
          },
        ],
        matchReplaceRules: [
          {
            target: "response_body",
            case_insensitive: false,
            is_regex: false,
            pattern: "foo",
            replace: "bar",
            enabled: true,
            priority: 1,
          },
        ],
      });
    });
    render(<ProxyControl />);
    const badge = screen.getByTestId("proxy-control-rules-active");
    expect(badge.textContent).toBe("3 rules active");
  });
});
