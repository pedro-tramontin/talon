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
    /// Stored as `Option<String>` to force callers to opt in — the
    /// default `AgentConfig` deliberately does not ship a placeholder
    /// string. The `validate()` method enforces presence + non-empty.
    #[serde(default)]
    pub api_key: Option<String>,
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
            // api_key is required; the default deliberately leaves it
            // unset so callers (production OR tests) must opt in
            // explicitly. The "ollama" string used elsewhere is a
            // localhost placeholder, not a real secret.
            api_key: None,
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
            api_key: Some("test".to_string()),
            model: model.into(),
            max_iterations: 20,
            allowed_tools: default_allowed_tools(),
        }
    }

    /// Validate that the config is usable: `api_base` parses as an
    /// http(s) URL and `api_key` is present and non-empty.
    pub fn validate(&self) -> Result<(), String> {
        let url = url::Url::parse(&self.api_base)
            .map_err(|e| format!("api_base is not a valid URL: {e}"))?;
        match url.scheme() {
            "http" | "https" => {}
            other => {
                return Err(format!(
                    "api_base scheme must be http or https, got {other}"
                ));
            }
        }
        match self.api_key.as_deref() {
            Some(key) if !key.is_empty() => Ok(()),
            Some(_) => Err("api_key is empty".to_string()),
            None => Err("api_key is required".to_string()),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_no_api_key() {
        // SEC-2 regression guard: the default must NOT embed a literal
        // placeholder like "ollama" — that's one config flip away from
        // shipping a real-looking secret. Callers must opt in.
        let cfg = AgentConfig::default();
        assert!(
            cfg.api_key.is_none(),
            "default AgentConfig must not set api_key; found {:?}",
            cfg.api_key
        );
    }

    #[test]
    fn default_config_uses_read_only_tool_allowlist() {
        let cfg = AgentConfig::default();
        assert_eq!(cfg.allowed_tools.len(), DEFAULT_READ_ONLY_TOOLS.len());
        for tool in DEFAULT_READ_ONLY_TOOLS {
            assert!(cfg.allowed_tools.iter().any(|t| t == tool));
        }
    }

    #[test]
    fn validate_accepts_http_url_with_key() {
        let cfg = AgentConfig::for_test("http://localhost:11434/v1", "m");
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn validate_accepts_https_url_with_key() {
        let cfg = AgentConfig {
            api_base: "https://api.openai.com/v1".into(),
            api_key: Some("sk-abc".into()),
            model: "gpt-4o".into(),
            max_iterations: 5,
            allowed_tools: vec![],
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn validate_rejects_non_http_scheme() {
        let cfg = AgentConfig {
            api_base: "file:///etc/passwd".into(),
            api_key: Some("k".into()),
            model: "m".into(),
            max_iterations: 5,
            allowed_tools: vec![],
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("scheme"), "got: {err}");
    }

    #[test]
    fn validate_rejects_unparseable_url() {
        let cfg = AgentConfig {
            api_base: "not a url".into(),
            api_key: Some("k".into()),
            model: "m".into(),
            max_iterations: 5,
            allowed_tools: vec![],
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("api_base"), "got: {err}");
    }

    #[test]
    fn validate_rejects_missing_api_key() {
        let cfg = AgentConfig {
            api_base: "http://localhost:11434/v1".into(),
            api_key: None,
            model: "m".into(),
            max_iterations: 5,
            allowed_tools: vec![],
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("api_key"), "got: {err}");
    }

    #[test]
    fn validate_rejects_empty_api_key() {
        let cfg = AgentConfig {
            api_base: "http://localhost:11434/v1".into(),
            api_key: Some(String::new()),
            model: "m".into(),
            max_iterations: 5,
            allowed_tools: vec![],
        };
        let err = cfg.validate().unwrap_err();
        assert!(
            err.contains("api_key") || err.contains("empty"),
            "got: {err}"
        );
    }

    #[test]
    fn configs_with_same_fields_are_equal() {
        // SC-7 regression: exercises the PartialEq derive by constructing
        // two identical configs and confirming the derive is sound.
        let a = AgentConfig::for_test("http://localhost:1/v1", "m");
        let b = AgentConfig::for_test("http://localhost:1/v1", "m");
        assert_eq!(a, b);
    }
}
