mod common;

use async_trait::async_trait;
use common::{
    MockLLMProvider, TestAgentOverrides, create_test_agent_with, text_response, tool_call,
    tool_response,
};
use oxicrab::providers::base::{ChatRequest, LLMProvider, LLMResponse, ToolDefinition};
use serde_json::json;
use std::collections::VecDeque;
use std::sync::Arc;
use tempfile::TempDir;

fn default_agent(
    provider: MockLLMProvider,
    tmp: &TempDir,
) -> impl std::future::Future<Output = oxicrab::agent::AgentLoop> + '_ {
    create_test_agent_with(provider, tmp, TestAgentOverrides::default())
}

/// A mock provider that also captures tool definitions passed by the agent loop.
struct ToolCapturingProvider {
    responses: Arc<std::sync::Mutex<VecDeque<LLMResponse>>>,
    pub tool_defs: Arc<std::sync::Mutex<Vec<Option<Vec<ToolDefinition>>>>>,
}

impl ToolCapturingProvider {
    fn new() -> Self {
        Self {
            responses: Arc::new(std::sync::Mutex::new(VecDeque::new())),
            tool_defs: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }
}

#[async_trait]
impl LLMProvider for ToolCapturingProvider {
    async fn chat(&self, req: ChatRequest<'_>) -> anyhow::Result<LLMResponse> {
        self.tool_defs.lock().unwrap().push(req.tools);
        let response = self.responses.lock().unwrap().pop_front();
        Ok(response.unwrap_or_else(|| LLMResponse {
            content: Some("Mock response".to_string()),
            tool_calls: vec![],
            reasoning_content: None,
            input_tokens: None,
            output_tokens: None,
        }))
    }

    fn default_model(&self) -> &str {
        "mock-model"
    }
}

#[tokio::test]
async fn test_simple_message_response() {
    let tmp = TempDir::new().unwrap();
    let provider = MockLLMProvider::with_responses(vec![text_response("Hello from the agent!")]);

    let agent = default_agent(provider, &tmp).await;

    let response = agent
        .process_direct("Hi there", "test:chat1", "telegram", "chat1")
        .await
        .unwrap();

    assert_eq!(response, "Hello from the agent!");
}

#[tokio::test]
async fn test_empty_message_handled() {
    let tmp = TempDir::new().unwrap();
    let provider =
        MockLLMProvider::with_responses(vec![text_response("I received an empty message.")]);

    let agent = default_agent(provider, &tmp).await;

    let response = agent
        .process_direct("", "test:empty", "telegram", "empty")
        .await
        .unwrap();

    assert_eq!(response, "I received an empty message.");
}

#[tokio::test]
async fn test_session_persists_across_messages() {
    let tmp = TempDir::new().unwrap();
    let provider = MockLLMProvider::with_responses(vec![
        text_response("First response"),
        text_response("Second response"),
    ]);
    let calls = provider.calls.clone();

    let agent = default_agent(provider, &tmp).await;

    // First message
    agent
        .process_direct("Hello", "test:persist", "telegram", "persist")
        .await
        .unwrap();

    // Second message on same session
    agent
        .process_direct("Follow up", "test:persist", "telegram", "persist")
        .await
        .unwrap();

    // The second call should have history from the first exchange
    let recorded = calls.lock().unwrap();
    assert_eq!(recorded.len(), 2);

    // Second call's messages should include history (system + history + current)
    let second_call_msgs = &recorded[1].messages;
    // Should have more messages than just system + user
    assert!(
        second_call_msgs.len() >= 3,
        "Expected history in second call, got {} messages",
        second_call_msgs.len()
    );
}

#[tokio::test]
async fn test_different_sessions_isolated() {
    let tmp = TempDir::new().unwrap();
    let provider = MockLLMProvider::with_responses(vec![
        text_response("Response A"),
        text_response("Response B"),
    ]);
    let calls = provider.calls.clone();

    let agent = default_agent(provider, &tmp).await;

    // Message on session A
    let resp_a = agent
        .process_direct("Hello A", "telegram:chatA", "telegram", "chatA")
        .await
        .unwrap();
    assert_eq!(resp_a, "Response A");

    // Message on session B - should not have session A's history
    let resp_b = agent
        .process_direct("Hello B", "discord:chatB", "discord", "chatB")
        .await
        .unwrap();
    assert_eq!(resp_b, "Response B");

    // Both calls should have been made
    let recorded = calls.lock().unwrap();
    assert_eq!(recorded.len(), 2);

    // Second call should NOT include "Hello A" in its messages
    let second_msgs = &recorded[1].messages;
    let has_hello_a = second_msgs.iter().any(|m| m.content.contains("Hello A"));
    assert!(
        !has_hello_a,
        "Session B should not contain Session A's history"
    );
}

#[tokio::test]
async fn test_tool_call_and_result() {
    let tmp = TempDir::new().unwrap();

    let provider = MockLLMProvider::with_responses(vec![
        tool_response(vec![tool_call(
            "tc1",
            "list_dir",
            json!({"path": tmp.path().to_str().unwrap()}),
        )]),
        text_response("I listed the directory for you."),
    ]);
    let calls = provider.calls.clone();

    let agent = default_agent(provider, &tmp).await;

    let response = agent
        .process_direct("List the directory", "test:tools", "telegram", "tools")
        .await
        .unwrap();

    assert_eq!(response, "I listed the directory for you.");

    // Should have made 2 calls to the provider
    let recorded = calls.lock().unwrap();
    assert_eq!(recorded.len(), 2);

    // Second call should include the tool result
    let second_msgs = &recorded[1].messages;
    let has_tool_result = second_msgs.iter().any(|m| m.role == "tool");
    assert!(has_tool_result, "Second call should include tool result");
}

#[tokio::test]
async fn test_unknown_tool_handled() {
    let tmp = TempDir::new().unwrap();

    let provider = MockLLMProvider::with_responses(vec![
        tool_response(vec![tool_call("tc_bad", "nonexistent_tool", json!({}))]),
        text_response("Sorry, that tool wasn't available."),
    ]);
    let calls = provider.calls.clone();

    let agent = default_agent(provider, &tmp).await;

    let response = agent
        .process_direct("Use magic tool", "test:unknown", "telegram", "unknown")
        .await
        .unwrap();

    assert_eq!(response, "Sorry, that tool wasn't available.");

    // The second call should have a tool result with an error
    let recorded = calls.lock().unwrap();
    let second_msgs = &recorded[1].messages;
    let tool_msg = second_msgs.iter().find(|m| m.role == "tool").unwrap();
    assert!(tool_msg.content.contains("does not exist"));
    assert!(tool_msg.is_error);
}

#[tokio::test]
async fn test_provider_called_with_tools() {
    let tmp = TempDir::new().unwrap();
    let provider = ToolCapturingProvider::new();
    let tool_defs = provider.tool_defs.clone();

    let agent = create_test_agent_with(provider, &tmp, TestAgentOverrides::default()).await;

    agent
        .process_direct("Hello", "test:tools_check", "telegram", "tools_check")
        .await
        .unwrap();

    let recorded = tool_defs.lock().unwrap();
    assert_eq!(recorded.len(), 1);

    // Should have tool definitions passed to the provider
    let tools = recorded[0].as_ref().unwrap();
    assert!(
        !tools.is_empty(),
        "Provider should receive tool definitions"
    );

    // Should include some default tools like read_file, list_dir, exec
    let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert!(
        tool_names.contains(&"read_file"),
        "Should have read_file tool"
    );
    assert!(
        tool_names.contains(&"list_dir"),
        "Should have list_dir tool"
    );
    assert!(tool_names.contains(&"exec"), "Should have exec tool");
}

#[tokio::test]
async fn test_multiple_tool_calls_in_sequence() {
    let tmp = TempDir::new().unwrap();

    // Create a test file for read_file to find
    std::fs::write(tmp.path().join("test.txt"), "test content").unwrap();

    let provider = MockLLMProvider::with_responses(vec![
        // First iteration: two tool calls
        tool_response(vec![
            tool_call(
                "tc1",
                "list_dir",
                json!({"path": tmp.path().to_str().unwrap()}),
            ),
            tool_call(
                "tc2",
                "read_file",
                json!({"path": tmp.path().join("test.txt").to_str().unwrap()}),
            ),
        ]),
        // Second iteration: final response
        text_response("Done reading files."),
    ]);

    let agent = default_agent(provider, &tmp).await;

    let response = agent
        .process_direct("Read files", "test:multi", "telegram", "multi")
        .await
        .unwrap();

    assert_eq!(response, "Done reading files.");
}

#[tokio::test]
async fn test_hallucination_detection_triggers_retry() {
    let tmp = TempDir::new().unwrap();

    let provider = MockLLMProvider::with_responses(vec![
        text_response("I've updated the configuration file for you."),
        text_response(
            "I can help you update the configuration. Which file would you like me to edit?",
        ),
    ]);
    let calls = provider.calls.clone();

    let agent = default_agent(provider, &tmp).await;

    let response = agent
        .process_direct(
            "Update my config",
            "test:hallucination",
            "telegram",
            "hallucination",
        )
        .await
        .unwrap();

    // Should get the corrected (second) response, not the hallucinated one
    assert_eq!(
        response,
        "I can help you update the configuration. Which file would you like me to edit?"
    );

    // Should have made 2 calls — original + retry after correction
    let recorded = calls.lock().unwrap();
    assert_eq!(recorded.len(), 2);

    // Second call should contain the correction message
    let second_msgs = &recorded[1].messages;
    let has_correction = second_msgs
        .iter()
        .any(|m| m.role == "user" && m.content.contains("did not use any tools"));
    assert!(
        has_correction,
        "Second call should contain the hallucination correction"
    );
}

#[tokio::test]
async fn test_no_hallucination_when_tools_used() {
    let tmp = TempDir::new().unwrap();

    let provider = MockLLMProvider::with_responses(vec![
        tool_response(vec![tool_call(
            "tc1",
            "list_dir",
            json!({"path": tmp.path().to_str().unwrap()}),
        )]),
        text_response("I've listed the directory for you."),
    ]);
    let calls = provider.calls.clone();

    let agent = default_agent(provider, &tmp).await;

    let response = agent
        .process_direct(
            "List files",
            "test:legit_action",
            "telegram",
            "legit_action",
        )
        .await
        .unwrap();

    // Should return the response as-is since tools were actually used
    assert_eq!(response, "I've listed the directory for you.");

    // Should have made exactly 2 calls (tool call + final response), no correction retry
    let recorded = calls.lock().unwrap();
    assert_eq!(recorded.len(), 2);
}

#[tokio::test]
async fn test_no_hallucination_for_informational_response() {
    let tmp = TempDir::new().unwrap();

    let provider = MockLLMProvider::with_responses(vec![text_response(
        "To update the config, you need to edit the settings.json file.",
    )]);
    let calls = provider.calls.clone();

    let agent = default_agent(provider, &tmp).await;

    let response = agent
        .process_direct("How do I update config?", "test:info", "telegram", "info")
        .await
        .unwrap();

    assert_eq!(
        response,
        "To update the config, you need to edit the settings.json file."
    );

    // Only 1 call — no retry needed
    let recorded = calls.lock().unwrap();
    assert_eq!(recorded.len(), 1);
}
