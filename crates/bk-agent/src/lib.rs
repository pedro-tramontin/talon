//! `bk-agent` — internal LLM agent loop for Talon.
//!
//! The crate exposes a single public entry point: [`Agent::run`],
//! which takes an [`AgentConfig`], an engine reference, and a user
//! goal, then runs a bounded loop against an OpenAI-compatible LLM
//! provider.  The agent calls only the tools in `allowed_tools`, and
//! emits progress events on a dedicated broadcast bus that the UI
//! consumes in §3.5d.
//!
//! The default tool list is read-only; any write tool must be opted
//! into explicitly by the user.

#![deny(missing_docs)]
#![warn(rust_2018_idioms)]

/// Agent configuration.
pub mod config;
/// Error types and `Result` alias.
pub mod error;
/// Event broadcast bus for agent progress.
pub mod events;
/// System-prompt rendering.
pub mod prompt;
/// Read-only tool schemas and dispatch.
pub mod tools;

mod r#loop;

pub use config::{AgentConfig, DEFAULT_READ_ONLY_TOOLS};
pub use error::{AgentError, Result};
pub use events::{agent_channel, AgentEvent, EventReceiver, EventSender};
pub use r#loop::{Agent, RunContext};
pub use tools::TOOL_SCHEMAS;
