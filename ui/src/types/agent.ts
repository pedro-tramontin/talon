// Type definitions mirroring the bk-agent Rust schema.
//
// These are HAND-ROLLED to match the serde-derived JSON shape from
// `bk-agent/src/config.rs` and `bk-agent/src/events.rs`. We do not
// auto-generate them (no ts-rs / specta / utoipa in v0.1) because the
// schema is small and the surface is stable. Re-derive or hand-mirror
// if the Rust side changes.
//
// The shapes here are the contract the React UI binds to.

/**
 * Connection config for an OpenAI-compatible LLM provider. Mirrors
 * `bk_agent::AgentConfig` (see `crates/bk-agent/src/config.rs`).
 *
 * Note: `api_key` is `Option<String>` and may be `null` —
 * the Rust `validate()` method will reject a missing key, so the UI
 * should not let the user submit a config without one.
 */
export interface AgentConfig {
  /** Base URL of the provider, e.g. `http://localhost:11434/v1`. */
  api_base: string;
  /** API key. May be null; the Rust validate() enforces presence. */
  api_key: string | null;
  /** Model name to request, e.g. `qwen2.5-coder:32b`. */
  model: string;
  /** Maximum LLM calls per run. */
  max_iterations: number;
  /** Tool names the agent is permitted to call. */
  allowed_tools: string[];
}

/**
 * Progress event from a running agent. Mirrors
 * `bk_agent::AgentEvent`. The `event` field is a snake_case tag
 * (e.g. `agent_started`, `agent_message`, `agent_finished`,
 * `agent_error`); the React side discriminates on it.
 */
export type AgentEvent =
  | { event: "agent_started"; agent_id: string; goal: string; model: string }
  | { event: "agent_thinking"; agent_id: string }
  | {
      event: "agent_tool_call";
      agent_id: string;
      tool_name: string;
      args: unknown;
      result_summary: string;
    }
  | { event: "agent_message"; agent_id: string; text: string }
  | {
      event: "agent_finished";
      agent_id: string;
      answer: string;
      iterations: number;
    }
  | { event: "agent_error"; agent_id: string; error: string };

/**
 * Request sent to the WebView when the agent wants to call a write
 * tool. The UI shows a `ConfirmDialog` and replies via
 * `agentConfirmWrite`.
 */
export interface ConfirmRequestPayload {
  /** The agent run this confirmation belongs to. */
  run_id: string;
  /** Tool name (e.g. `talon_delete_exchange`). */
  tool_name: string;
  /** Arguments the LLM supplied for the tool call. */
  args: unknown;
  /** Short human-readable description. */
  description: string;
}

/**
 * Resolution event sent back to the WebView so the modal can close.
 */
export interface ConfirmResponsePayload {
  run_id: string;
  tool_name: string;
  resolution: "allow" | "deny" | "timeout" | "cancelled";
  remember: boolean;
}
