//! End-to-end test for the bk-agent loop.
//!
//! Boots a wiremock server returning canned OpenAI-compatible
//! responses, constructs an Agent pointing at the mock server, and
//! asserts the agent loop drives the wiremock as expected:
//!   * the LLM endpoint is hit exactly once,
//!   * the agent emits an `AgentToolCall` event for the mock tool,
//!   * the agent emits `AgentStarted` first, then `AgentThinking`,
//!     then `AgentToolCall`, in that order,
//!   * the result of the run is the `Ok(String)` final answer from
//!     the second canned response.
//!
//! The second canned response is a plain assistant message
//! (no tool calls) so the loop terminates after the first tool
//! call without depending on a real engine path.

use bk_agent::events::AgentEvent;
use bk_agent::{Agent, AgentConfig, RunContext};
use bk_core::Project;
use bk_engine::Engine;
use std::sync::Arc;
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_loop_executes_tool_then_terminates_with_answer() {
    // 1. Mock OpenAI server. First call: tool call. Second call: final answer.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "chatcmpl-1",
            "object": "chat.completion",
            "created": 1_700_000_000,
            "model": "test-model",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "talon_list_recent",
                            "arguments": "{\"project_id\": \"00000000-0000-0000-0000-000000000001\", \"limit\": 10}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "chatcmpl-2",
            "object": "chat.completion",
            "created": 1_700_000_001,
            "model": "test-model",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "No recent exchanges found.",
                    "tool_calls": null
                },
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 50, "completion_tokens": 8, "total_tokens": 58}
        })))
        .mount(&server)
        .await;

    // 2. Engine in a temp dir with a project that matches the mock's
    //    project_id so the tool call succeeds.
    let tmp = TempDir::new().unwrap();
    let engine = Arc::new(Engine::new(tmp.path()).unwrap());
    let project_id = bk_core::ProjectId::from_uuid(
        uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
    );
    let mut project = Project::new("test", "example.com", "0.1.0");
    project.info.id = project_id;
    engine.open_project(&project).unwrap();

    // 3. Agent config pointing at the mock.
    let config = AgentConfig {
        api_base: server.uri(),
        api_key: Some("test".into()),
        model: "test-model".into(),
        max_iterations: 5,
        allowed_tools: vec!["talon_list_recent".into()],
    };

    // 4. Subscribe to events so we can assert the broadcast ordering.
    let (event_tx, mut event_rx) = bk_agent::events::agent_channel();

    // 5. Run the agent.
    let agent = Agent::new(config, event_tx);
    let result = agent
        .run(
            engine,
            RunContext {
                project_name: project.info.name.clone(),
                project_id: project.info.id,
                target_host: project.info.target_host.clone(),
                goal: "list recent exchanges".into(),
            },
        )
        .await;

    // 6. Result must be the final-answer string from the second mock.
    let answer = result.expect("agent run must succeed for a final-answer turn");
    assert_eq!(answer, "No recent exchanges found.");

    // 7. Drain the event bus and assert the [Started, Thinking, ToolCall,
    //    Thinking, Message, Finished] ordering. The two `up_to_n_times(1)`
    //    + unlimited mocks give us exactly two LLM calls; the first
    //    yields a tool call, the second yields the final answer.
    let mut events = Vec::new();
    while let Ok(ev) = event_rx.try_recv() {
        events.push(ev);
    }

    // First three: Started, Thinking, ToolCall
    assert!(
        matches!(events.first(), Some(AgentEvent::AgentStarted { .. })),
        "first event must be AgentStarted, got {:?}",
        events.first()
    );
    assert!(
        matches!(events.get(1), Some(AgentEvent::AgentThinking { .. })),
        "second event must be AgentThinking, got {:?}",
        events.get(1)
    );
    let tool_call_event = events.get(2).expect("third event must exist");
    match tool_call_event {
        AgentEvent::AgentToolCall { tool_name, .. } => {
            assert_eq!(tool_name, "talon_list_recent");
        }
        other => panic!("third event must be AgentToolCall, got {other:?}"),
    }

    // Then Thinking for the second iteration, then AgentMessage,
    // then AgentFinished.
    let saw_second_thinking = events
        .iter()
        .skip(3)
        .any(|e| matches!(e, AgentEvent::AgentThinking { .. }));
    assert!(saw_second_thinking, "expected a second AgentThinking event");
    let saw_message = events
        .iter()
        .any(|e| matches!(e, AgentEvent::AgentMessage { text, .. } if text == "No recent exchanges found."));
    assert!(saw_message, "expected an AgentMessage with the final text");
    let finished = events
        .iter()
        .find_map(|e| match e {
            AgentEvent::AgentFinished {
                answer, iterations, ..
            } => Some((answer.clone(), *iterations)),
            _ => None,
        })
        .expect("expected an AgentFinished event");
    assert_eq!(finished.0, "No recent exchanges found.");
    assert_eq!(finished.1, 2, "two LLM turns were issued");
}

/// LE-3 regression test: when the agent hits the max-iterations cap
/// it must emit `AgentFinished` (so UI consumers can clear their
/// spinner) AND return `Err(MaxIterations)`. The pre-fix code emitted
/// `AgentError` and never `AgentFinished`, leaving UIs stuck in a
/// "thinking" state.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn max_iterations_emits_agent_finished_and_returns_err() {
    let server = MockServer::start().await;
    // Always returns a tool call. The loop will exhaust the cap.
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "chatcmpl-loop",
            "object": "chat.completion",
            "created": 1_700_000_000,
            "model": "loop-model",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_x",
                        "type": "function",
                        "function": {
                            "name": "talon_list_recent",
                            "arguments": "{\"project_id\": \"00000000-0000-0000-0000-000000000002\", \"limit\": 1}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        })))
        .mount(&server)
        .await;

    let tmp = TempDir::new().unwrap();
    let engine = Arc::new(Engine::new(tmp.path()).unwrap());
    let project_id = bk_core::ProjectId::from_uuid(
        uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap(),
    );
    let mut project = Project::new("t", "example.com", "0.1.0");
    project.info.id = project_id;
    engine.open_project(&project).unwrap();

    let config = AgentConfig {
        api_base: server.uri(),
        api_key: Some("test".into()),
        model: "loop-model".into(),
        max_iterations: 2, // cap at 2 so the test is fast
        allowed_tools: vec!["talon_list_recent".into()],
    };

    let (event_tx, mut event_rx) = bk_agent::events::agent_channel();
    let agent = Agent::new(config, event_tx);
    let result = agent
        .run(
            engine,
            RunContext {
                project_name: "t".into(),
                project_id,
                target_host: "example.com".into(),
                goal: "loop forever".into(),
            },
        )
        .await;

    // 1. The Result must be Err(MaxIterations).
    let err = result.expect_err("loop must error at cap");
    match err {
        bk_agent::AgentError::MaxIterations { max } => assert_eq!(max, 2),
        other => panic!("expected MaxIterations, got {other:?}"),
    }

    // 2. The event bus must contain an AgentFinished event with the cap
    //    message. Drain (using try_recv so we don't block forever).
    let mut events = Vec::new();
    while let Ok(ev) = event_rx.try_recv() {
        events.push(ev);
    }
    let finished = events
        .iter()
        .find_map(|e| match e {
            bk_agent::events::AgentEvent::AgentFinished { answer, .. } => Some(answer.clone()),
            _ => None,
        })
        .expect("AgentFinished must be emitted before returning Err");
    assert!(
        finished.contains("max-iterations cap"),
        "AgentFinished answer should explain the cap; got: {finished}"
    );
}
