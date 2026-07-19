use serde::{Deserialize, Serialize};

/// Default read-only tools the agent may invoke without explicit user opt-in.
pub const DEFAULT_READ_ONLY_TOOLS: &[&str] = &[
    "talon_list_recent",
    "talon_search",
    "talon_get_exchange",
    "talon_list_tags",
    "talon_list_tags_for_exchange",
];

/// Configuration for connecting to an OpenAI-compatible LLM provider.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct AgentConfig {
    /// Base URL of the provider, e.g. `http://localhost:11434/v1` or `https://api.openai.com/v1`.
    pub api_base: String,
    /// API key.  Required by the OpenAI wire format; ignored by most local servers.
    pub api_key: String,
    /// Model name to request.
    pub model: String,
    /// Maximum number of LLM calls per run.
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
    /// Tool names the agent is permitted to call.  Defaults to the read-only list.
    #[serde(default = "default_allowed_tools")]
    pub allowed_tools: Vec<String>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            api_base: "http://localhost:11434/v1".to_string(),
            api_key: "ollama".to_string(),
            model: "qwen2.5-coder:32b".to_string(),
            max_iterations: default_max_iterations(),
            allowed_tools: default_allowed_tools(),
        }
    }
}

impl AgentConfig {
    /// Build a config for tests against a mock or local provider.
    pub fn for_test(api_base: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            api_base: api_base.into(),
            api_key: "test".to_string(),
            model: model.into(),
            max_iterations: 20,
            allowed_tools: default_allowed_tools(),
        }
    }
}

fn default_max_iterations() -> u32 {
    20
}

fn default_allowed_tools() -> Vec<String> {
    DEFAULT_READ_ONLY_TOOLS
        .iter()
        .map(|s| s.to_string())
        .collect()
}
