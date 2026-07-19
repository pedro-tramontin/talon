//! The agent loop.
//!
//! The loop is intentionally simple: system prompt + history, LLM
//! call, tool execution or final answer, repeat up to `max_iterations`.

use crate::{prompt, tools, AgentConfig, AgentError, AgentEvent, EventSender, Result};
use async_openai::{
    config::OpenAIConfig,
    types::chat::{
        ChatCompletionMessageToolCalls, ChatCompletionRequestMessage,
        ChatCompletionRequestSystemMessage, ChatCompletionRequestToolMessage,
        ChatCompletionRequestUserMessage, ChatCompletionResponseMessage,
        ChatCompletionToolChoiceOption, CreateChatCompletionRequest, FunctionCall,
        ToolChoiceOptions,
    },
    Client,
};
use bk_core::ProjectId;
use bk_engine::Engine;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;
use tracing::instrument;
use uuid::Uuid;

/// Per-run context for the agent.
#[derive(Debug, Clone)]
pub struct RunContext {
    /// Human-readable project name.
    pub project_name: String,
    /// Project identifier.
    pub project_id: ProjectId,
    /// Target host for the project.
    pub target_host: String,
    /// User-supplied natural-language goal.
    pub goal: String,
}

/// The agent itself.  Holds only immutable state; the loop runs in
/// [`Agent::run`].
pub struct Agent {
    config: AgentConfig,
    client: Client<OpenAIConfig>,
    event_tx: EventSender,
}

impl Agent {
    /// Build an agent from config and a broadcaster for progress events.
    pub fn new(config: AgentConfig, event_tx: EventSender) -> Self {
        let openai_config = OpenAIConfig::new()
            .with_api_base(config.api_base.clone())
            .with_api_key(config.api_key.clone());
        let client = Client::with_config(openai_config);
        Self {
            config,
            client,
            event_tx,
        }
    }

    /// Run the agent against a single user goal.
    ///
    /// Returns the final answer text, or an error if the loop hits a
    /// terminal failure.
    #[instrument(skip(self, engine), fields(agent_id, project_id = %ctx.project_id, goal = %ctx.goal))]
    pub async fn run(&self, engine: Arc<Engine>, ctx: RunContext) -> Result<String> {
        let agent_id = Uuid::new_v4().to_string();
        tracing::Span::current().record("agent_id", agent_id.as_str());

        let allowed_schemas: Vec<_> = tools::TOOL_SCHEMAS
            .iter()
            .filter(|s| self.config.allowed_tools.contains(&s.name.to_string()))
            .cloned()
            .collect();
        let _tools_for_request = tools::openai_tools();

        let system_text = prompt::render_system_prompt(
            &ctx.project_name,
            ctx.project_id,
            &ctx.target_host,
            &ctx.goal,
            &allowed_schemas,
        );

        self.emit(AgentEvent::AgentStarted {
            agent_id: agent_id.clone(),
            goal: ctx.goal.clone(),
            model: self.config.model.clone(),
        });

        let mut messages: Vec<ChatCompletionRequestMessage> = vec![
            ChatCompletionRequestMessage::System(ChatCompletionRequestSystemMessage {
                content: async_openai::types::chat::ChatCompletionRequestSystemMessageContent::Text(
                    system_text,
                ),
                name: None,
            }),
            ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
                content: async_openai::types::chat::ChatCompletionRequestUserMessageContent::Text(
                    ctx.goal.clone(),
                ),
                name: None,
            }),
        ];

        let mut iterations: u32 = 0;
        loop {
            if iterations >= self.config.max_iterations {
                let err = format!("max iterations ({}) reached", self.config.max_iterations);
                self.emit(AgentEvent::AgentError {
                    agent_id: agent_id.clone(),
                    error: err.clone(),
                });
                return Err(AgentError::MaxIterations {
                    max: self.config.max_iterations,
                });
            }
            iterations += 1;

            self.emit(AgentEvent::AgentThinking {
                agent_id: agent_id.clone(),
            });
            let tools_for_request = tools::openai_tools();

            let request = CreateChatCompletionRequest {
                model: self.config.model.clone(),
                messages: messages.clone(),
                tools: Some(tools_for_request.clone()),
                tool_choice: Some(ChatCompletionToolChoiceOption::Mode(
                    ToolChoiceOptions::Auto,
                )),
                ..Default::default()
            };

            let response = timeout(Duration::from_secs(60), self.client.chat().create(request))
                .await
                .map_err(|_| AgentError::Llm("LLM call timed out after 60s".into()))?
                .map_err(|e| AgentError::Llm(format!("LLM request failed: {e}")))?;

            let choice = response
                .choices
                .into_iter()
                .next()
                .ok_or_else(|| AgentError::Llm("LLM returned no choices".into()))?;
            let message: ChatCompletionResponseMessage = choice.message;

            // If the LLM produced a plain message without a tool call,
            // treat it as the final answer.
            if message.content.is_some() {
                let text = message.content.unwrap_or_default();
                self.emit(AgentEvent::AgentMessage {
                    agent_id: agent_id.clone(),
                    text: text.clone(),
                });
                self.emit(AgentEvent::AgentFinished {
                    agent_id: agent_id.clone(),
                    answer: text.clone(),
                    iterations,
                });
                return Ok(text);
            }

            // Tool call path.
            if let Some(tool_calls) = message.tool_calls {
                for tool_call in tool_calls {
                    let (call, tool_call_id) = match tool_call {
                        ChatCompletionMessageToolCalls::Function(fc) => {
                            (fc.function.clone(), fc.id)
                        }
                        ChatCompletionMessageToolCalls::Custom(custom) => {
                            let fc = FunctionCall {
                                name: custom.custom_tool.name.clone(),
                                arguments: custom.custom_tool.input.clone(),
                            };
                            (fc, custom.id)
                        }
                    };
                    let result = tools::execute(&engine, &self.config.allowed_tools, &call)?;
                    let summary = tools::summarize_result(&call.name, &result);

                    self.emit(AgentEvent::AgentToolCall {
                        agent_id: agent_id.clone(),
                        tool_name: call.name.clone(),
                        args: serde_json::from_str(&call.arguments).unwrap_or(Value::Null),
                        result_summary: summary,
                    });

                    messages.push(ChatCompletionRequestMessage::Tool(
                        ChatCompletionRequestToolMessage {
                            content:
                                async_openai::types::chat::ChatCompletionRequestToolMessageContent::Text(
                                    result.to_string(),
                                ),
                            tool_call_id,
                        },
                    ));
                }
            }
        }
    }

    fn emit(&self, event: AgentEvent) {
        let _ = self.event_tx.send(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_new_builds_client() {
        let (tx, _rx) = crate::events::agent_channel();
        let config = AgentConfig::for_test("http://localhost:1/v1", "model");
        let agent = Agent::new(config.clone(), tx);
        assert_eq!(agent.config.api_base, "http://localhost:1/v1");
        assert_eq!(agent.config.model, "model");
    }
}
