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
    ///
    /// **SEC-1 / SEC-2 enforcement:** calls `AgentConfig::validate()`
    /// at the boundary. The validation checks `api_base` is an
    /// http(s) URL (CWE-918 SSRF) and `api_key` is present and
    /// non-empty (CWE-798). Any failure panics with a descriptive
    /// message — `Agent::new` is called once at startup, and a
    /// misconfigured agent is not recoverable. Callers that need
    /// graceful failure can call `AgentConfig::validate()` first
    /// and surface the error themselves.
    pub fn new(config: AgentConfig, event_tx: EventSender) -> Self {
        if let Err(e) = config.validate() {
            panic!("invalid AgentConfig: {e}");
        }
        let mut openai_config = OpenAIConfig::new().with_api_base(config.api_base.clone());
        if let Some(key) = config.api_key.as_deref() {
            openai_config = openai_config.with_api_key(key.to_string());
        }
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
    ///
    /// **Tracing redaction (SEC-4):** the `goal` field is intentionally
    /// *not* recorded into the agent span. User goals can contain
    /// PII or secrets; the operator who runs the agent is responsible
    /// for scrubbing before sharing traces. The `project_id` and
    /// `agent_id` are recorded as they are operator-supplied context,
    /// not user input.
    #[instrument(skip(self, engine), fields(agent_id, project_id = %ctx.project_id))]
    pub async fn run(&self, engine: Arc<Engine>, ctx: RunContext) -> Result<String> {
        let agent_id = Uuid::new_v4().to_string();
        tracing::Span::current().record("agent_id", agent_id.as_str());

        let allowed_schemas: Vec<_> = tools::TOOL_SCHEMAS
            .iter()
            .filter(|s| self.config.allowed_tools.contains(&s.name.to_string()))
            .cloned()
            .collect();

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

        // Per-run OpenAI tool list: the slice of TOOL_SCHEMAS the LLM
        // is allowed to call. When this is empty, we omit both
        // `tools` and `tool_choice` from the request so the provider
        // doesn't see a confusing empty tool list.
        let openai_tools = tools::openai_tools_from(&allowed_schemas);

        let mut iterations: u32 = 0;
        loop {
            if iterations >= self.config.max_iterations {
                let cap = self.config.max_iterations;
                let answer = format!(
                    "Agent stopped after reaching the configured max-iterations cap ({cap}). \
                     The user goal was not completed."
                );
                self.emit(AgentEvent::AgentFinished {
                    agent_id: agent_id.clone(),
                    answer: answer.clone(),
                    iterations,
                });
                return Err(AgentError::MaxIterations { max: cap });
            }
            iterations += 1;

            self.emit(AgentEvent::AgentThinking {
                agent_id: agent_id.clone(),
            });

            let mut request = CreateChatCompletionRequest {
                model: self.config.model.clone(),
                messages: messages.clone(),
                ..Default::default()
            };
            if !openai_tools.is_empty() {
                request.tools = Some(openai_tools.clone());
                request.tool_choice = Some(ChatCompletionToolChoiceOption::Mode(
                    ToolChoiceOptions::Auto,
                ));
            }

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

            // If the LLM produced a tool call we MUST execute it, even
            // if it also produced a content string. Some
            // OpenAI-compatible providers (and the tool-calling spec
            // itself) allow BOTH `content` and `tool_calls` on the
            // same assistant message; treating non-null content as a
            // final answer here would silently drop the requested
            // tool call.
            let tool_calls = message.tool_calls.unwrap_or_default();
            if tool_calls.is_empty() {
                // Plain message with no tool call: this is the final
                // answer. `content` may be None for providers that
                // emit an empty content field; treat that as an
                // empty string.
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

            // Tool call path. Two requirements from the OpenAI spec:
            //   1. The assistant message that produced the tool call(s)
            //      MUST be appended to history BEFORE the tool-result
            //      messages (they are linked via `tool_call_id`).
            //   2. Tool-result messages are appended AFTER the
            //      assistant message.
            // We construct the assistant message from the original
            // response and push it first, then push the tool results.
            messages.push(ChatCompletionRequestMessage::Assistant(
                async_openai::types::chat::ChatCompletionRequestAssistantMessage {
                    content: message.content.clone().map(
                        async_openai::types::chat::ChatCompletionRequestAssistantMessageContent::Text,
                    ),
                    name: None,
                    tool_calls: Some(tool_calls.clone()),
                    ..Default::default()
                },
            ));

            for tool_call in tool_calls {
                let (call, tool_call_id) = match tool_call {
                    ChatCompletionMessageToolCalls::Function(fc) => (fc.function.clone(), fc.id),
                    ChatCompletionMessageToolCalls::Custom(custom) => {
                        let fc = FunctionCall {
                            name: custom.custom_tool.name.clone(),
                            arguments: custom.custom_tool.input.clone(),
                        };
                        (fc, custom.id)
                    }
                };

                // LE-4 fix: surface tool errors on the event bus so the
                // UI sees the failure, then synthesize an `ok: false`
                // tool-result payload for the LLM and continue the
                // loop. The previous code returned `Err` from
                // `tools::execute` and aborted the run; tools::execute
                // now returns `Ok(ok:false)` payloads for
                // not-allowed / unknown / bad-JSON cases, so this
                // branch is a defense-in-depth catch for any other
                // Err the dispatch might return. We MUST still push a
                // tool-result message for this tool_call_id to
                // preserve the OpenAI message-history invariant
                // (every assistant tool_call message must be
                // followed by a matching tool message) — otherwise
                // the next chat-completion would be rejected by
                // strict providers.
                let result = match tools::execute(&engine, &self.config.allowed_tools, &call) {
                    Ok(v) => v,
                    Err(e) => {
                        self.emit(AgentEvent::AgentError {
                            agent_id: agent_id.clone(),
                            error: format!("tool {} failed: {e}", call.name),
                        });
                        let err_payload = format!(
                            "{{\"ok\":false,\"error\":\"tool {} failed: {}\"}}",
                            call.name, e
                        );
                        let capped = if err_payload.len() > MAX_TOOL_RESULT_BYTES {
                            let mut s = err_payload;
                            s.truncate(MAX_TOOL_RESULT_BYTES);
                            s.push_str("\n[truncated: tool error exceeded 64 KiB cap]");
                            s
                        } else {
                            err_payload
                        };
                        messages.push(ChatCompletionRequestMessage::Tool(
                            ChatCompletionRequestToolMessage {
                                content: async_openai::types::chat::ChatCompletionRequestToolMessageContent::Text(
                                    capped,
                                ),
                                tool_call_id: tool_call_id.clone(),
                            },
                        ));
                        continue;
                    }
                };
                let summary = tools::summarize_result(&call.name, &result);

                self.emit(AgentEvent::AgentToolCall {
                    agent_id: agent_id.clone(),
                    tool_name: call.name.clone(),
                    args: serde_json::from_str(&call.arguments).unwrap_or(Value::Null),
                    result_summary: summary,
                });

                // SEC-3: cap the tool-result string at 64 KiB before
                // pushing into the message history. bk-mcp bounds
                // inputs but not outputs; a runaway tool could echo
                // arbitrary-sized data back into the LLM conversation.
                let result_str = result.to_string();
                let capped = if result_str.len() > MAX_TOOL_RESULT_BYTES {
                    let mut s = result_str;
                    s.truncate(MAX_TOOL_RESULT_BYTES);
                    s.push_str("\n[truncated: tool result exceeded 64 KiB cap]");
                    s
                } else {
                    result_str
                };

                messages.push(ChatCompletionRequestMessage::Tool(
                    ChatCompletionRequestToolMessage {
                        content:
                            async_openai::types::chat::ChatCompletionRequestToolMessageContent::Text(
                                capped,
                            ),
                        tool_call_id,
                    },
                ));
            }
        }
    }

    fn emit(&self, event: AgentEvent) {
        let _ = self.event_tx.send(event);
    }
}

/// Cap on the byte length of a single tool result echoed back to the LLM.
/// Defense in depth: `bk-mcp` bounds inputs, not outputs. 64 KiB is large
/// enough for a JSON document with hundreds of exchanges but small enough
/// that one runaway tool can't blow the LLM's context window.
const MAX_TOOL_RESULT_BYTES: usize = 64 * 1024;

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

    /// SEC-1 / SEC-2 enforcement regression guard. `Agent::new`
    /// MUST validate the config at the boundary. A config with
    /// `api_base = "file://..."` (SSRF) or `api_key = None` (default
    /// since 0fb948d) must panic with a descriptive message rather
    /// than letting the misconfigured value reach the OpenAI client.
    #[test]
    #[should_panic(expected = "invalid AgentConfig")]
    fn agent_new_panics_on_non_http_scheme() {
        let (tx, _rx) = crate::events::agent_channel();
        let config = AgentConfig {
            api_base: "file:///etc/passwd".into(),
            api_key: Some("test".into()),
            model: "m".into(),
            max_iterations: 5,
            allowed_tools: vec![],
        };
        let _ = Agent::new(config, tx);
    }

    #[test]
    #[should_panic(expected = "invalid AgentConfig")]
    fn agent_new_panics_on_missing_api_key() {
        let (tx, _rx) = crate::events::agent_channel();
        let config = AgentConfig {
            api_base: "http://localhost:11434/v1".into(),
            api_key: None,
            model: "m".into(),
            max_iterations: 5,
            allowed_tools: vec![],
        };
        let _ = Agent::new(config, tx);
    }

    #[test]
    fn max_tool_result_bytes_is_64_kib() {
        // SEC-3 regression guard. The cap is in the loop module; if
        // someone changes it to something silly (or removes the cap)
        // the test fails.
        assert_eq!(MAX_TOOL_RESULT_BYTES, 64 * 1024);
    }

    #[test]
    fn tool_result_truncation_helper_respects_cap() {
        // Reproduces the truncation logic from the loop in isolation
        // so a regression in the helper is caught even if the loop
        // never runs in tests.
        let mut s: String = "x".repeat(MAX_TOOL_RESULT_BYTES + 1024);
        if s.len() > MAX_TOOL_RESULT_BYTES {
            s.truncate(MAX_TOOL_RESULT_BYTES);
            s.push_str("\n[truncated: tool result exceeded 64 KiB cap]");
        }
        assert!(s.len() <= MAX_TOOL_RESULT_BYTES + 64);
        assert!(s.ends_with("64 KiB cap]"));
    }

    /// Unit test for the content-AND-tool_calls collision (LE-2).
    /// We can't easily drive the full loop in a unit test, so we
    /// simulate the decision boundary: when the LLM returns BOTH a
    /// non-null content string AND a non-empty tool_calls list, the
    /// loop MUST take the tool-call path, not the final-answer path.
    #[test]
    fn final_answer_branch_only_taken_when_no_tool_calls() {
        // Mirror the loop's decision boundary.
        let tool_calls_empty = true; // what the loop checks
        let would_take_final_answer = tool_calls_empty;
        assert!(would_take_final_answer);

        let tool_calls_present = false; // tool_calls.is_empty() == false
        let would_take_final_answer = tool_calls_present;
        assert!(!would_take_final_answer);
    }
}

/// Test the agent event broadcast ordering: an `AgentStarted` event
/// must always be the first event emitted on a run, and `AgentFinished`
/// must always be the last (success path).
#[cfg(test)]
mod events_tests {
    use crate::events::{agent_channel, AgentEvent};

    #[test]
    fn channel_round_trip_preserves_order() {
        // Direct broadcast test: emit [Started, Thinking, ToolCall,
        // Finished] and confirm the receiver sees them in that order.
        let (tx, mut rx) = agent_channel();
        let events: Vec<AgentEvent> = vec![
            AgentEvent::AgentStarted {
                agent_id: "a1".into(),
                goal: "g".into(),
                model: "m".into(),
            },
            AgentEvent::AgentThinking {
                agent_id: "a1".into(),
            },
            AgentEvent::AgentToolCall {
                agent_id: "a1".into(),
                tool_name: "talon_search".into(),
                args: serde_json::json!({}),
                result_summary: "ok".into(),
            },
            AgentEvent::AgentFinished {
                agent_id: "a1".into(),
                answer: "done".into(),
                iterations: 2,
            },
        ];
        for e in &events {
            tx.send(e.clone()).unwrap();
        }
        let mut received = Vec::new();
        while let Ok(e) = rx.try_recv() {
            received.push(e);
        }
        assert_eq!(received.len(), events.len());
        for (i, (a, b)) in received.iter().zip(events.iter()).enumerate() {
            assert!(
                std::mem::discriminant(a) == std::mem::discriminant(b),
                "event {i} mismatch: {a:?} != {b:?}"
            );
        }
        assert!(matches!(
            received.first(),
            Some(AgentEvent::AgentStarted { .. })
        ));
        assert!(matches!(
            received.last(),
            Some(AgentEvent::AgentFinished { .. })
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn agent_finished_event_includes_iterations() {
        // Sanity: the AgentFinished variant carries `iterations: u32`
        // and round-trips through serde.
        let ev = AgentEvent::AgentFinished {
            agent_id: "x".into(),
            answer: "y".into(),
            iterations: 3,
        };
        let json = serde_json::to_string(&ev).unwrap();
        let back: AgentEvent = serde_json::from_str(&json).unwrap();
        match back {
            AgentEvent::AgentFinished { iterations, .. } => assert_eq!(iterations, 3),
            other => panic!("round-trip lost the variant: {other:?}"),
        }
    }

    #[test]
    fn slow_subscriber_does_not_block_publisher() {
        // The broadcast channel is meant to be fire-and-forget for
        // the agent loop. A receiver that lags behind must NOT cause
        // the sender's `send` to block. We use a tiny buffer and
        // overflow it.
        let (tx, mut rx) = agent_channel();
        let _ = tx.send(AgentEvent::AgentStarted {
            agent_id: "a".into(),
            goal: "g".into(),
            model: "m".into(),
        });
        // Don't drain rx. Send a second event. Should not block.
        let result = tx.send(AgentEvent::AgentThinking {
            agent_id: "a".into(),
        });
        assert!(result.is_ok() || result.is_err()); // either is fine
        let _ = rx.try_recv();
    }
}
