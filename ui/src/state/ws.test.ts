// Tests for `useWsStore` (ui/src/state/ws.ts).
//
// The store is a global module-level singleton. Each test
// resets it via `wsStore.setState` so tests are independent.

import { beforeEach, describe, expect, it } from "vitest";
import { wsStore } from "./ws";

function resetStore() {
  wsStore.setState({
    connectionState: "disconnected",
    lastError: null,
    droppedGaps: 0,
  });
}

beforeEach(() => {
  resetStore();
});

describe("useWsStore", () => {
  it("defaults to 'disconnected' with no error and no dropped gaps", () => {
    expect(wsStore.getState().connectionState).toBe("disconnected");
    expect(wsStore.getState().lastError).toBeNull();
    expect(wsStore.getState().droppedGaps).toBe(0);
  });

  it("setConnectionState transitions between states", () => {
    wsStore.getState().setConnectionState("reconnecting");
    expect(wsStore.getState().connectionState).toBe("reconnecting");
    wsStore.getState().setConnectionState("connected");
    expect(wsStore.getState().connectionState).toBe("connected");
  });

  it("setLastError stores and clears the error", () => {
    wsStore.getState().setLastError("WS closed: code 1006");
    expect(wsStore.getState().lastError).toBe("WS closed: code 1006");
    wsStore.getState().setLastError(null);
    expect(wsStore.getState().lastError).toBeNull();
  });

  it("addDroppedGap increments the gap counter", () => {
    wsStore.getState().addDroppedGap();
    wsStore.getState().addDroppedGap();
    wsStore.getState().addDroppedGap();
    expect(wsStore.getState().droppedGaps).toBe(3);
  });
});
