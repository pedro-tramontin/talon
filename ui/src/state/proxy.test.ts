// Tests for `useProxyStore` (ui/src/state/proxy.ts).
//
// The store is a global module-level singleton. Each test
// resets it via `proxyStore.setState` so tests are
// independent.

import { beforeEach, describe, expect, it } from "vitest";
import { proxyStore } from "./proxy";
import type { ProxyStatus } from "../types/domain";

function resetStore() {
  proxyStore.setState({
    status: {
      state: "stopped",
      listener_addr: null,
      ca_fingerprint: null,
      last_error: null,
    },
  });
}

beforeEach(() => {
  resetStore();
});

describe("useProxyStore", () => {
  it("defaults to 'stopped' with no listener / fingerprint / error", () => {
    expect(proxyStore.getState().status).toEqual({
      state: "stopped",
      listener_addr: null,
      ca_fingerprint: null,
      last_error: null,
    });
  });

  it("setStatus replaces the whole status", () => {
    const next: ProxyStatus = {
      state: "running",
      listener_addr: "127.0.0.1:8080",
      ca_fingerprint: "ab:cd:ef",
      last_error: null,
    };
    proxyStore.getState().setStatus(next);
    expect(proxyStore.getState().status).toEqual(next);
  });

  it("setStatus to 'error' preserves the last_error message", () => {
    const next: ProxyStatus = {
      state: "error",
      listener_addr: null,
      ca_fingerprint: null,
      last_error: "bind: address already in use",
    };
    proxyStore.getState().setStatus(next);
    expect(proxyStore.getState().status.state).toBe("error");
    expect(proxyStore.getState().status.last_error).toBe(
      "bind: address already in use",
    );
  });
});
