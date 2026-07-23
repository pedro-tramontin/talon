import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";

// The `WireClient` autodetects Tauri mode via the
// `__TAURI_INTERNALS__` global. The vi.mock for
// `@tauri-apps/api/event` swaps the production `listen` for a
// vi.fn() that returns a fake unlisten handle. The test then
// drives the client via the public `dispatch` path — the same
// path the production `listen` callback uses (it's a 1-line
// `this.dispatch(e.payload)`).

const TauriInternalsMock = "__TAURI_INTERNALS__";

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (_event: string, _handler: (e: { payload: unknown }) => void) => {
    return () => {
      // fake unlisten — no-op
    };
  }),
}));

import { WireClient } from "./ws";

function setTauriMode(enabled: boolean) {
  if (enabled) {
    (window as unknown as Record<string, unknown>)[TauriInternalsMock] = {};
  } else {
    delete (window as unknown as Record<string, unknown>)[TauriInternalsMock];
  }
}

describe("WireClient (Tauri mode)", () => {
  beforeEach(() => {
    setTauriMode(true);
  });

  afterEach(async () => {
    setTauriMode(false);
  });

  it("connect() registers a Tauri listener for 'wire_event'", async () => {
    const { listen } = await import("@tauri-apps/api/event");
    const client = new WireClient();
    await client.connect();
    expect(listen).toHaveBeenCalledTimes(1);
    expect(listen).toHaveBeenCalledWith("wire_event", expect.any(Function));
    await client.disconnect();
  });

  it("dispatch routes events to handlers subscribed by kind", async () => {
    const client = new WireClient();
    const engineHandler = vi.fn();
    const agentHandler = vi.fn();
    client.subscribe("engine_event", engineHandler);
    client.subscribe("agent_event", agentHandler);

    client.dispatch({
      kind: "engine_event",
      payload: { project: "acme" },
      seq: 1,
    });
    client.dispatch({
      kind: "agent_event",
      payload: { event: "started" },
      seq: 2,
    });

    expect(engineHandler).toHaveBeenCalledTimes(1);
    expect(engineHandler).toHaveBeenCalledWith(
      { project: "acme" },
      expect.objectContaining({ seq: 1 }),
    );
    expect(agentHandler).toHaveBeenCalledTimes(1);
    expect(agentHandler).toHaveBeenCalledWith(
      { event: "started" },
      expect.objectContaining({ seq: 2 }),
    );

    await client.disconnect();
  });

  it("seq gaps are recorded in droppedGaps but events still flow", () => {
    const client = new WireClient();
    const handler = vi.fn();
    client.subscribe("proxy_event", handler);

    client.dispatch({ kind: "proxy_event", payload: "a", seq: 1 });
    client.dispatch({ kind: "proxy_event", payload: "b", seq: 5 }); // gap: 2,3,4
    client.dispatch({ kind: "proxy_event", payload: "c", seq: 6 });

    expect(handler).toHaveBeenCalledTimes(3); // all events still delivered
    const gaps = client.getDroppedGaps();
    expect(gaps).toHaveLength(1);
    expect(gaps[0]).toEqual({ from: 1, to: 5 });
    expect(client.getLastSeq()).toBe(6);

    client.resetSeq();
    expect(client.getLastSeq()).toBe(0);
    expect(client.getDroppedGaps()).toHaveLength(0);
  });

  it("subscribe returns an unsubscribe that removes the handler", () => {
    const client = new WireClient();
    const handler = vi.fn();
    const unsub = client.subscribe("engine_event", handler);
    client.dispatch({ kind: "engine_event", payload: {}, seq: 1 });
    expect(handler).toHaveBeenCalledTimes(1);

    unsub();
    client.dispatch({ kind: "engine_event", payload: {}, seq: 2 });
    expect(handler).toHaveBeenCalledTimes(1); // still 1 — no second call
  });

  it("handler exceptions do not break the dispatch loop", () => {
    const client = new WireClient();
    const goodHandler = vi.fn();
    client.subscribe("engine_event", () => {
      throw new Error("boom");
    });
    client.subscribe("engine_event", goodHandler);
    // Suppress the expected console.error so the test output stays clean.
    const errSpy = vi.spyOn(console, "error").mockImplementation(() => {});

    client.dispatch({ kind: "engine_event", payload: "x", seq: 1 });

    expect(goodHandler).toHaveBeenCalledTimes(1);
    expect(errSpy).toHaveBeenCalled();
    errSpy.mockRestore();
  });
});

describe("WireClient (browser mode)", () => {
  beforeEach(() => {
    setTauriMode(false);
  });

  afterEach(async () => {
    setTauriMode(false);
  });

  it("connect() does NOT call the Tauri listen when not in Tauri mode", async () => {
    const { listen } = await import("@tauri-apps/api/event");
    // Clear the mock so we count only this test's call.
    (listen as unknown as { mockClear: () => void }).mockClear();

    const client = new WireClient({ wsUrl: "ws://invalid-host:9999/ws" });
    // connect() will fire a WebSocket constructor which will fail in jsdom;
    // we only care that `listen` was NOT called.
    try {
      await client.connect();
    } catch {
      // expected — jsdom WebSocket support is partial
    }
    expect(listen).not.toHaveBeenCalled();
    await client.disconnect();
  });

  // Phase 8 (full v1) — when the auth token is set,
  // the WireClient passes
  // `talon-auth.<token>` as the WebSocket
  // subprotocols list on the WS upgrade. The
  // server's WS handler reads this subprotocol
  // and verifies the token with
  // `subtle::ConstantTimeEq` (browsers forbid the
  // `Authorization` header on WS upgrade requests,
  // so the subprotocol is the standard handoff).
  it("sends 'talon-auth.<token>' subprotocol when authToken is set", () => {
    // Spy on the global WebSocket constructor so we
    // can capture the protocols arg without making a
    // real connection (jsdom's WebSocket is partial).
    const realWS = globalThis.WebSocket;
    const wsSpy = vi.fn().mockImplementation(function () {
      // The mock just records the args; no real
      // network is attempted.
    });
    class MockWS {
      constructor(url: string | URL, protocols?: string | string[]) {
        wsSpy(url, protocols);
      }
    }
    // @ts-expect-error -- mocking the global
    globalThis.WebSocket = MockWS;

    try {
      const token = "abc123def456";
      // Use a non-Tauri mode (the `beforeEach`
      // already cleared the Tauri internals).
      const client = new WireClient({
        wsUrl: "ws://localhost:8080/ws",
        authToken: token,
      });
      // Connect: this triggers the WebSocket
      // constructor with the subprotocol.
      try {
        // jsdom will throw on the missing addEventListener
        // but that's fine; the constructor was already
        // called before the throw.
        // We access the internal `openWs` via a hack:
        // `connect()` is async and the WebSocket
        // constructor runs synchronously inside
        // `openWs`. The `addEventListener` calls fail
        // on the mock.
        // @ts-expect-error -- accessing private method for the test
        void client.openWs();
      } catch {
        // expected
      }
      // Assert: the WebSocket constructor was
      // called with the URL and the
      // `talon-auth.<token>` subprotocols list.
      expect(wsSpy).toHaveBeenCalled();
      const [, protocols] = wsSpy.mock.calls[0];
      expect(protocols).toEqual([`talon-auth.${token}`]);
    } finally {
      globalThis.WebSocket = realWS;
    }
  });
});
