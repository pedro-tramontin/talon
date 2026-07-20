//! Error surface for the agent.

use thiserror::Error;

/// Agent-level failures.
#[derive(Debug, Error)]
pub enum AgentError {
    /// The requested tool is not in the allowed list.
    #[error("tool {tool} is not in the allowed list")]
    ToolNotAllowed {
        /// Tool name the LLM requested.
        tool: String,
    },

    /// The LLM requested an unknown tool.
    #[error("unknown tool: {tool}")]
    UnknownTool {
        /// Tool name the LLM requested.
        tool: String,
    },

    /// A call to the underlying engine failed.
    #[error("engine error: {0}")]
    Engine(#[from] bk_engine::EngineError),

    /// A call to the LLM provider failed.
    #[error("LLM error: {0}")]
    Llm(String),

    /// The agent hit the iteration cap without finishing.
    #[error("agent reached max iterations ({max})")]
    MaxIterations {
        /// Configured cap.
        max: u32,
    },

    /// Generic error wrapper for external failures.
    #[error("{0}")]
    Other(#[from] anyhow::Error),
}

/// Convenience result type.
pub type Result<T> = std::result::Result<T, AgentError>;
