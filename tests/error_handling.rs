mod common;

use async_trait::async_trait;
use common::{
    create_test_agent_with, text_response, tool_call, tool_response, MockLLMProvider, RecordedCall,
    TestAgentOverrides,
};
use oxicrab::providers::base::{ChatRequest, LLMProvider, LLMResponse};
use serde_json::json;
use std::sync::Arc;
use tempfile::TempDir;

/// An LLM provider that always returns an error.
struct FailingMockProvider {
    error_message: String,
    calls: Arc<std::sync::Mutex<Vec<RecordedCall>>>,
}

impl FailingMockProvider {
    fn new(error_message: &str) -> Self {
        Self {
            error_message: error_message.to_string(),
            calls: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }
}

#[async_trait]
impl LLMProvider for FailingMockProvider {
    async fn chat(&self, req: ChatRequest<'_>) -> anyhow::Result<LLMResponse> {
        self.calls.lock().unwrap().push(RecordedCall {
            messages: req.messages,
        });
        Err(anyhow::anyhow!("{}", self.error_message))
    }

    fn default_model(&self) -> &str {
        "mock-model"
    }
}

#[tokio::test]
async fn test_provider_error_propagated() {
    let tmp = TempDir::new().unwrap();
    let provider = FailingMockProvider::new("LLM is down");

    let agent = create_test_agent_with(provider, &tmp, TestAgentOverrides::default()).await;

    let result = agent
        .process_direct("Hello", "test:err1", "telegram", "err1")
        .await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("LLM is down"));
}

#[tokio::test]
async fn test_tool_error_forwarded_to_llm() {
    let tmp = TempDir::new().unwrap();
    let missing = tmp.path().join("nonexistent.txt");

    let provider = MockLLMProvider::with_responses(vec![
        tool_response(vec![tool_call(
            "tc1",
            "read_file",
            json!({"path": missing.to_str().unwrap()}),
        )]),
        text_response("The file doesn't exist."),
    ]);
    let calls = provider.calls.clone();

    let agent = create_test_agent_with(provider, &tmp, TestAgentOverrides::default()).await;
    let response = agent
        .process_direct("Read it", "test:err2", "telegram", "err2")
        .await
        .unwrap();

    assert_eq!(response, "The file doesn't exist.");

    // Second LLM call should contain the error tool result
    let recorded = calls.lock().unwrap();
    let second_msgs = &recorded[1].messages;
    let tool_msg = second_msgs.iter().find(|m| m.role == "tool").unwrap();
    assert!(tool_msg.is_error);
}

#[tokio::test]
async fn test_max_iterations_respected() {
    let tmp = TempDir::new().unwrap();

    // Provider always returns a tool call — should stop at max_iterations
    let mut responses = Vec::new();
    for i in 0..15 {
        responses.push(tool_response(vec![tool_call(
            &format!("tc{}", i),
            "list_dir",
            json!({"path": tmp.path().to_str().unwrap()}),
        )]));
    }
    // Add a text response that shouldn't be reached with max_iterations=3
    responses.push(text_response("Should not reach this."));

    let provider = MockLLMProvider::with_responses(responses);
    let calls = provider.calls.clone();

    let agent = create_test_agent_with(
        provider,
        &tmp,
        TestAgentOverrides {
            max_iterations: Some(3),
            ..Default::default()
        },
    )
    .await;

    let response = agent
        .process_direct("Loop forever", "test:err3", "telegram", "err3")
        .await
        .unwrap();

    // Should have stopped and returned something (not panicked)
    assert!(!response.is_empty());

    // Should not have made more than max_iterations + 1 calls
    let recorded = calls.lock().unwrap();
    assert!(
        recorded.len() <= 4,
        "Should respect max_iterations=3, got {} calls",
        recorded.len()
    );
}

#[tokio::test]
async fn test_empty_llm_response_handled() {
    let tmp = TempDir::new().unwrap();

    // Provider returns empty content and no tool calls
    let provider = MockLLMProvider::with_responses(vec![
        LLMResponse {
            content: None,
            tool_calls: vec![],
            reasoning_content: None,
            input_tokens: None,
        },
        LLMResponse {
            content: None,
            tool_calls: vec![],
            reasoning_content: None,
            input_tokens: None,
        },
        LLMResponse {
            content: None,
            tool_calls: vec![],
            reasoning_content: None,
            input_tokens: None,
        },
        text_response("Finally responding."),
    ]);

    let agent = create_test_agent_with(provider, &tmp, TestAgentOverrides::default()).await;
    let response = agent
        .process_direct("Hello?", "test:err4", "telegram", "err4")
        .await
        .unwrap();

    // Should get a response without panicking
    assert!(!response.is_empty());
}

#[tokio::test]
async fn test_invalid_tool_arguments() {
    let tmp = TempDir::new().unwrap();

    // Provide a number where a string is expected for "path"
    let provider = MockLLMProvider::with_responses(vec![
        tool_response(vec![tool_call("tc1", "read_file", json!({"path": 12345}))]),
        text_response("Bad args handled."),
    ]);
    let calls = provider.calls.clone();

    let agent = create_test_agent_with(provider, &tmp, TestAgentOverrides::default()).await;
    let response = agent
        .process_direct("Read file 12345", "test:err5", "telegram", "err5")
        .await
        .unwrap();

    assert_eq!(response, "Bad args handled.");

    // Tool result should be an error about type mismatch
    let recorded = calls.lock().unwrap();
    let second_msgs = &recorded[1].messages;
    let tool_msg = second_msgs.iter().find(|m| m.role == "tool").unwrap();
    assert!(tool_msg.is_error);
    assert!(tool_msg.content.contains("should be string"));
}

#[tokio::test]
async fn test_missing_required_argument() {
    let tmp = TempDir::new().unwrap();

    // Call read_file without the required "path" parameter
    let provider = MockLLMProvider::with_responses(vec![
        tool_response(vec![tool_call("tc1", "read_file", json!({}))]),
        text_response("Missing param handled."),
    ]);
    let calls = provider.calls.clone();

    let agent = create_test_agent_with(provider, &tmp, TestAgentOverrides::default()).await;
    let response = agent
        .process_direct("Read a file", "test:err6", "telegram", "err6")
        .await
        .unwrap();

    assert_eq!(response, "Missing param handled.");

    // Tool result should be an error about missing parameter
    let recorded = calls.lock().unwrap();
    let second_msgs = &recorded[1].messages;
    let tool_msg = second_msgs.iter().find(|m| m.role == "tool").unwrap();
    assert!(tool_msg.is_error);
    assert!(tool_msg.content.contains("missing required"));
}

#[tokio::test]
async fn test_tool_result_truncation() {
    let tmp = TempDir::new().unwrap();

    // Create a file with > 10k characters
    let large_content = "x".repeat(15000);
    let target = tmp.path().join("large.txt");
    std::fs::write(&target, &large_content).unwrap();

    let provider = MockLLMProvider::with_responses(vec![
        tool_response(vec![tool_call(
            "tc1",
            "read_file",
            json!({"path": target.to_str().unwrap()}),
        )]),
        text_response("Read large file."),
    ]);
    let calls = provider.calls.clone();

    let agent = create_test_agent_with(provider, &tmp, TestAgentOverrides::default()).await;
    agent
        .process_direct("Read large", "test:err7", "telegram", "err7")
        .await
        .unwrap();

    // The tool result sent to the LLM should be truncated
    let recorded = calls.lock().unwrap();
    let second_msgs = &recorded[1].messages;
    let tool_msg = second_msgs.iter().find(|m| m.role == "tool").unwrap();
    // Result should be truncated below the original 15000 chars
    assert!(
        tool_msg.content.len() < 12000,
        "Tool result should be truncated, got {} chars",
        tool_msg.content.len()
    );
    assert!(
        tool_msg.content.contains("truncated") || tool_msg.content.len() <= 10500,
        "Should indicate truncation or be within limit"
    );
}
