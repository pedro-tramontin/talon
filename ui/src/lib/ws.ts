// Phase 8 wire-bus client (browser + Tauri).
//
// `WireClient` autodetects the transport at construction time:
// - **Tauri mode** (`'__TAURI_INTERNALS__' in window`): subscribes to
//   the Tauri `wire_event` channel via `@tauri-apps/api/event`'s
//   `listen<WireEvent>('wire_event', ...)`. No reconnection logic —
//   the Tauri shell is a long-lived parent process.
// - **Browser mode**: opens a `WebSocket('ws://<host>:<port>/ws')`
//   and parses incoming `WireEvent` JSON messages. Reconnects on
//   close with an exponential backoff (the 8 h capture session is
//   long enough to assume the WS will flap at least once).
//
// Consumers subscribe per-kind via `subscribe(kind, handler)` and get
// back an `unsubscribe()` closure. The `dispatch` method is the
// internal fan-out from the inbound `wire_event` payload to the
// registered per-kind handlers. Tests inject events via the public
// `dispatch` method (the `vi.mock` for `@tauri-apps/api/event` makes
// the production path injectable in tests; the in-test `dispatch`
// path is the canonical way to drive the client's behavior).
//
// The on-wire shape (from `bk-events::WireEvent`):
//   { kind: "engine_event" | "agent_event" | "proxy_event",
//   { kind: "engine_event" | "agent_event" | "proxy_event" | "replay_event",
//     payload: <kind-specific JSON>, seq: <monotonic u64> }
//
// The kind strings are pinned by the `WireEventKind::as_str` method
// in `crates/bk-events/src/lib.rs` and must match here.

import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type WireEventKind =
  | "engine_event"
  | "agent_event"
  | "proxy_event"
  | "replay_event";

/**
 * Payload of a `replay_event` WireEvent. Mirrors the Rust
 * `bk_events::ReplayEvent` shape. The `tab_id` is the
 * client-generated UUID the `ReplayStore` uses to identify
 * a tab; the Rust side does not generate it (it just
 * round-trips whatever the JS dispatch sent in the future
 * for cross-tab sync; for v1 the `tab_id` is `""` because
 * the send path is synchronous via `send_replay` IPC).
 */
export interface ReplayEventPayload {
  readonly tab_id: string;
  readonly kind: "send_complete" | "send_failed";
  readonly exchange_id: string | null;
  readonly error: string | null;
}

export interface WireEvent {
  readonly kind: WireEventKind;
  readonly payload: unknown;
  readonly seq: number;
}

export type WireHandler = (payload: unknown, ev: WireEvent) => void;

export interface WireClientOptions {
  /** Browser-mode WS URL (Phase 8). Ignored in Tauri mode. */
  readonly wsUrl?: string;
  /**
   * Initial backoff in ms (browser mode only).
   */
  readonly initialBackoffMs?: number;
  /**
   * Max backoff in ms (browser mode only).
   */
  readonly maxBackoffMs?: number;
  /**
   * Optional auth token for the WS upgrade (Phase 8
   * remote mode). When set, the client sends the
   * `Sec-WebSocket-Protocol: talon-auth.<token>`
   * subprotocol on the upgrade request (browsers forbid
   * the `Authorization` header on WS upgrades). The
   * server's WS handler reads the subprotocol and
   * verifies the token with `subtle::ConstantTimeEq`.
   * Ignored in Tauri mode (the Tauri shell uses an
   * in-process channel that doesn't need a subprotocol).
   */
  readonly authToken?: string;
}

/** Returns true if the page is running inside a Tauri webview. */
function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/**
 * Wire event bus client. Owns the transport subscription and the
 * per-kind handler fan-out. The same class works in Tauri mode
 * (subscribes to the `wire_event` Tauri event) and browser mode
 * (opens a WS to the Phase 8 `bk-server`). Consumers do not need
 * to know which mode is active.
 */
export class WireClient {
  private readonly handlers = new Map<WireEventKind, Set<WireHandler>>();
  private readonly opts: WireClientOptions;
  private unlisten: UnlistenFn | null = null;
  private ws: WebSocket | null = null;
  private reconnectAttempt = 0;
  private closed = false;
  private lastSeq = 0;
  private droppedGaps: Array<{ from: number; to: number }> = [];

  constructor(opts: WireClientOptions = {}) {
    this.opts = {
      wsUrl: opts.wsUrl ?? this.defaultWsUrl(),
      authToken: opts.authToken,
      initialBackoffMs: opts.initialBackoffMs ?? 250,
      maxBackoffMs: opts.maxBackoffMs ?? 30_000,
    };
  }

  /**
   * Open the transport and start receiving events. In Tauri mode
   * this is a one-shot `listen()` registration. In browser mode
   * it opens the WS (and reconnects on close). Calling `connect`
   * twice is a no-op.
   */
  async connect(): Promise<void> {
    if (this.unlisten || this.ws || this.closed) return;
    if (isTauri()) {
      this.unlisten = await listen<WireEvent>("wire_event", (e) => {
        this.dispatch(e.payload);
      });
    } else {
      this.openWs();
    }
  }

  /**
   * Close the transport. After `disconnect` the client is in a
   * terminal state — `connect` will not reopen. Use this in tests
   * to clean up the mock listener.
   */
  async disconnect(): Promise<void> {
    this.closed = true;
    if (this.unlisten) {
      this.unlisten();
      this.unlisten = null;
    }
    if (this.ws) {
      this.ws.close();
      this.ws = null;
    }
  }

  /**
   * Register a handler for a specific kind. Returns an unsubscribe
   * function. Multiple handlers per kind are allowed and called in
   * registration order.
   */
  subscribe(kind: WireEventKind, handler: WireHandler): () => void {
    let set = this.handlers.get(kind);
    if (!set) {
      set = new Set();
      this.handlers.set(kind, set);
    }
    set.add(handler);
    return () => {
      set!.delete(handler);
    };
  }

  /**
   * Dispatch a wire event to the per-kind handlers. The seq is
   * checked for monotonicity — a gap is recorded in `droppedGaps`
   * but events are NOT dropped (the §3.5c-style design: surface
   * the gap to the UI, but keep streaming). The `droppedGaps`
   * accessor (used by the React `useWsStore`) exposes the count.
   */
  dispatch(ev: WireEvent): void {
    // Drop detection: if the new seq is not exactly lastSeq + 1,
    // and not equal to lastSeq (a duplicate from a re-subscribe),
    // record the gap. The seq is u64 in Rust but JSON encodes it
    // as a number — at Phase 8's expected rates (< 1k events/s)
    // 53-bit float precision is fine for the next ~285 years.
    if (ev.seq <= this.lastSeq) {
      // Either a duplicate (re-subscribe) or a reorder (shouldn't
      // happen with the global seq, but tolerate it). No gap.
    } else if (ev.seq > this.lastSeq + 1 && this.lastSeq > 0) {
      this.droppedGaps.push({ from: this.lastSeq, to: ev.seq });
    }
    this.lastSeq = ev.seq;

    const set = this.handlers.get(ev.kind);
    if (!set) return;
    for (const h of set) {
      try {
        h(ev.payload, ev);
      } catch (err) {
        // A handler exception must not break the dispatch loop —
        // log to the console and continue. The §4.0 contract is
        // that the wire is the bus; consumers must not throw
        // across it.
        console.error("WireClient handler threw:", err);
      }
    }
  }

  /** Read-only view of the seq gaps observed so far. */
  getDroppedGaps(): ReadonlyArray<{ from: number; to: number }> {
    return this.droppedGaps;
  }

  /** The most recently seen seq. 0 means "nothing yet". */
  getLastSeq(): number {
    return this.lastSeq;
  }

  /**
   * Reset seq tracking (used when a test drives the client
   * directly via `dispatch` and wants to start from a clean
   * slate). The Phase 8 reconnect path calls this on a fresh
   * WS open so the gap counter doesn't carry over across a
   * server restart.
   */
  resetSeq(): void {
    this.lastSeq = 0;
    this.droppedGaps = [];
  }

  // --- private ---

  private defaultWsUrl(): string {
    if (typeof window === "undefined") return "ws://localhost:8080/ws";
    const proto = window.location.protocol === "https:" ? "wss" : "ws";
    const host = window.location.hostname || "localhost";
    // Phase 8: the `bk-server` binds to 8080 by default. The
    // capture UI's Vite dev server runs on 1420 (per the
    // Tauri config), so the WS port is independent. Tests
    // override this via the constructor option.
    return `${proto}://${host}:8080/ws`;
  }

  private openWs(): void {
    if (this.closed) return;
    // The `wsUrl` is always non-null after the
    // constructor default; assert with a fallback.
    const wsUrl = this.opts.wsUrl ?? this.defaultWsUrl();
    // Build the constructor arg list. The 2nd arg of
    // `WebSocket` is the optional `subprotocols` list;
    // browsers send these as the
    // `Sec-WebSocket-Protocol` request header. When the
    // auth token is set, we pass `["talon-auth.<token>"]`
    // (the server-side handler reads this subprotocol
    // and verifies the token).
    const protocols: string[] | undefined = this.opts.authToken
      ? [`talon-auth.${this.opts.authToken}`]
      : undefined;
    const ws = protocols
      ? new WebSocket(wsUrl, protocols)
      : new WebSocket(wsUrl);
    this.ws = ws;
    ws.addEventListener("message", (msg) => {
      try {
        const ev = JSON.parse(msg.data) as WireEvent;
        this.dispatch(ev);
      } catch (err) {
        console.error("WireClient: failed to parse WS message:", err);
      }
    });
    ws.addEventListener("close", () => {
      this.ws = null;
      if (this.closed) return;
      // Exponential backoff with cap. Reset the seq on a full
      // disconnect (Phase 8 server restart) so the gap counter
      // is fresh. The `initialBackoffMs` / `maxBackoffMs`
      // defaults are filled in by the constructor.
      this.reconnectAttempt += 1;
      const initial = this.opts.initialBackoffMs ?? 250;
      const max = this.opts.maxBackoffMs ?? 30_000;
      const backoff = Math.min(
        initial * 2 ** (this.reconnectAttempt - 1),
        max,
      );
      this.resetSeq();
      setTimeout(() => this.openWs(), backoff);
    });
  }
}

// ---------------------------------------------------------------------------
// Singleton accessor
//
// The `WireClient` is a single app-wide instance. It's lazy-initialized
// on the first call to `getWireClient()` so the constructor doesn't run
// at module-import time (Pitfall #37: no top-level `let` capturing
// handlers). Tests can call `setWireClient(new WireClient(...))` to
// swap in a mock before any `getWireClient()` call resolves.

let _wireClient: WireClient | null = null;

/**
 * Get the singleton `WireClient`. Lazily constructs a default
 * instance on first call. `connect()` is NOT called here — the
 * `App.tsx` mount effect is responsible for that (so the
 * connection only opens once, in the React tree).
 */
export function getWireClient(): WireClient {
  if (_wireClient === null) {
    _wireClient = new WireClient();
  }
  return _wireClient;
}

/**
 * Replace the singleton with a caller-provided instance. Tests
 * use this to inject a mock before any `getWireClient()` call
 * lands. Returns the previous instance so the caller can
 * restore state if needed.
 */
export function setWireClient(client: WireClient | null): WireClient | null {
  const prev = _wireClient;
  _wireClient = client;
  return prev;
}
