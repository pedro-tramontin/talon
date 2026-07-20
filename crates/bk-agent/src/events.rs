//! Progress events emitted by the agent for UI consumption.

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// Default broadcast channel capacity for agent events.
const EVENT_BUFFER: usize = 256;

/// A progress event from an agent run.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum AgentEvent {
    /// The agent started a new run.
    AgentStarted {
        /// Stable agent identifier.
        agent_id: String,
        /// User-supplied goal.
        goal: String,
        /// Model name requested from the provider.
        model: String,
    },
    /// The agent is waiting on the LLM.
    AgentThinking {
        /// Stable agent identifier.
        agent_id: String,
    },
    /// The agent called a tool and got a result.
    AgentToolCall {
        /// Stable agent identifier.
        agent_id: String,
        /// Tool name.
        tool_name: String,
        /// Arguments the LLM supplied.
        args: serde_json::Value,
        /// Short, human-readable summary of the result.
        result_summary: String,
    },
    /// The LLM returned a plain message (not a tool call).
    AgentMessage {
        /// Stable agent identifier.
        agent_id: String,
        /// Message text.
        text: String,
    },
    /// The agent produced a final answer and stopped.
    AgentFinished {
        /// Stable agent identifier.
        agent_id: String,
        /// Final summary from the LLM.
        answer: String,
        /// Number of LLM turns used.
        iterations: u32,
    },
    /// The run ended with an error.
    AgentError {
        /// Stable agent identifier.
        agent_id: String,
        /// Error message.
        error: String,
    },
}

/// Sender side of the agent event bus.
pub type EventSender = broadcast::Sender<AgentEvent>;
/// Receiver side of the agent event bus.
pub type EventReceiver = broadcast::Receiver<AgentEvent>;

/// Create a new agent event bus.
pub fn agent_channel() -> (EventSender, EventReceiver) {
    broadcast::channel(EVENT_BUFFER)
}
