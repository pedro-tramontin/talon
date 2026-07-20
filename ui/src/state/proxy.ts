// Zustand store for the proxy status.
//
// Per the §4.3-4.4 spec, this is a per-feature store (not a
// global app store). The `status` field is the §4.1 DTO
// (`app::proxy_handle::ProxyStatus`), updated on:
//   1. App mount (initial `proxyStatus()` call)
//   2. Wire-bus `proxy_event` of shape ProxyStatus (the
//      `bk-proxy` lifecycle events translate to status
//      updates before being forwarded as wire events).
//
// Start/stop actions live on the Tauri side (`startProxy`,
// `stopProxy` in `ui/src/api.ts`); this store just holds the
// observable state.

import { createStore, useStore } from "zustand";
import type { StoreApi } from "zustand/vanilla";
import type { ProxyStatus } from "../types/domain";

/** Default initial status — the proxy hasn't been queried yet. */
const DEFAULT_STATUS: ProxyStatus = {
  state: "stopped",
  listener_addr: null,
  ca_fingerprint: null,
  last_error: null,
};

/** Top-level store shape. */
export type ProxyStore = {
  /** The current proxy status (a §4.1 DTO). */
  status: ProxyStatus;
  /** Replace the status (called from the wire-bus handler
   * and on the initial `proxyStatus()` call). */
  setStatus: (status: ProxyStatus) => void;
};

function createProxyStore() {
  return createStore<ProxyStore>((set) => ({
    status: { ...DEFAULT_STATUS },
    setStatus(status) {
      set({ status });
    },
  }));
}

// Singleton store for app-wide use.
export const proxyStore: StoreApi<ProxyStore> = createProxyStore();

/**
 * React hook for the proxy store. Use with a selector to
 * limit re-renders to the slice you care about (e.g.
 * `useProxyStore((s) => s.status.state)`).
 */
export function useProxyStore<T>(selector: (state: ProxyStore) => T): T {
  return useStore(proxyStore, selector);
}
