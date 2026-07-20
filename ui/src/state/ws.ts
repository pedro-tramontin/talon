// Zustand store for the cross-cutting wire-bus transport state.
//
// Per the §4.3-4.4 spec, this is a per-feature store. The
// `WireClient` (in `ui/src/lib/ws.ts`) owns the transport
// (Tauri `listen` or browser `WebSocket`); this store exposes
// the transport's observable state so the UI can render
// banners ("reconnecting…", "missed events").
//
// This store does NOT own the seq-tracking array — that's
// owned by the `WireClient` and exposed via
// `client.getDroppedGaps()`. We store the COUNT here so the
// UI doesn't re-render on the full array reference changing.
//
// The store has no auth-token state (Gate 3): all transport
// state is metadata about the connection, not credentials.

import { createStore, useStore } from "zustand";
import type { StoreApi } from "zustand/vanilla";

/** The transport connection state. */
export type ConnectionState = "connected" | "reconnecting" | "disconnected";

/** Top-level store shape. */
export type WsStore = {
  /** The current transport state. */
  connectionState: ConnectionState;
  /** The last connection error (e.g. WS close code). */
  lastError: string | null;
  /** Count of seq gaps observed so far. The full array
   * stays in the `WireClient`. */
  droppedGaps: number;

  setConnectionState: (state: ConnectionState) => void;
  setLastError: (err: string | null) => void;
  /** Bump the dropped-gap counter. Called from the
   * `WireClient.dispatch` observer (in `App.tsx`). */
  addDroppedGap: () => void;
};

function createWsStore() {
  return createStore<WsStore>((set) => ({
    // Start as "disconnected" — `App.tsx`'s mount effect
    // transitions to "connected" after the WireClient's
    // `connect()` resolves.
    connectionState: "disconnected",
    lastError: null,
    droppedGaps: 0,

    setConnectionState(connectionState) {
      set({ connectionState });
    },

    setLastError(lastError) {
      set({ lastError });
    },

    addDroppedGap() {
      set((state) => ({ droppedGaps: state.droppedGaps + 1 }));
    },
  }));
}

// Singleton store for app-wide use.
export const wsStore: StoreApi<WsStore> = createWsStore();

/**
 * React hook for the wire-bus store. Use with a selector to
 * limit re-renders to the slice you care about (e.g.
 * `useWsStore((s) => s.droppedGaps)`).
 */
export function useWsStore<T>(selector: (state: WsStore) => T): T {
  return useStore(wsStore, selector);
}
