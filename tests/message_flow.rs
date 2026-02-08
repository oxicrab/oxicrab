mod common;

use common::MockLLMProvider;
use nanobot::agent::{AgentLoop, AgentLoopConfig};
use nanobot::bus::MessageBus;
use nanobot::config::CompactionConfig;
use nanobot::providers::base::{LLMResponse, ToolCallRequest};
use serde_json::json;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::Mutex;

async fn create_test_agent(provider: MockLLMProvider, tmp: &TempDir) -> AgentLoop {
    let bus = Arc::new(Mutex::new(MessageBus::default()));
    let (outbound_tx, _outbound_rx) = tokio::sync::mpsc::channel(100);
    let outbound_tx = Arc::new(outbound_tx);

    let config = AgentLoopConfig {
        bus,
        provider: Arc::new(provider),
        workspace: tmp.path().to_path_buf(),
        model: Some("mock-model".to_string()),
        max_iterations: 10,
        brave_api_key: None,
        exec_timeout: 30,
        restrict_to_workspace: true,
        allowed_commands: vec![],
        compaction_config: CompactionConfig {
            enabled: false,
            threshold_tokens: 40000,
            keep_recent: 10,
            extraction_enabled: false,
            model: None,
        },
        outbound_tx,
        cron_service: None,
        google_config: None,
        github_config: None,
        weather_config: None,
        todoist_config: None,
        temperature: 0.7,
        tool_temperature: 0.0,
        session_ttl_days: 0, // Disable cleanup in tests
        typing_tx: None,
        channels_config: None,
    };

    AgentLoop::new(config)
        .await
        .expect("Failed to create AgentLoop")
}

#[tokio::test]
async fn test_simple_message_response() {
    let tmp = TempDir::new().unwrap();
    let provider = MockLLMProvider::with_responses(vec![LLMResponse {
        content: Some("Hello from the agent!".to_string()),
        tool_calls: vec![],
        reasoning_content: None,
    }]);

    let agent = create_test_agent(provider, &tmp).await;

    let response = agent
        .process_direct("Hi there", "test:chat1", "telegram", "chat1")
        .await
        .unwrap();

    assert_eq!(response, "Hello from the agent!");
}

#[tokio::test]
async fn test_empty_message_handled() {
    let tmp = TempDir::new().unwrap();
    let provider = MockLLMProvider::with_responses(vec![LLMResponse {
        content: Some("I received an empty message.".to_string()),
        tool_calls: vec![],
        reasoning_content: None,
    }]);

    let agent = create_test_agent(provider, &tmp).await;

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
        LLMResponse {
            content: Some("First response".to_string()),
            tool_calls: vec![],
            reasoning_content: None,
        },
        LLMResponse {
            content: Some("Second response".to_string()),
            tool_calls: vec![],
            reasoning_content: None,
        },
    ]);
    let calls = provider.calls.clone();

    let agent = create_test_agent(provider, &tmp).await;

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
        LLMResponse {
            content: Some("Response A".to_string()),
            tool_calls: vec![],
            reasoning_content: None,
        },
        LLMResponse {
            content: Some("Response B".to_string()),
            tool_calls: vec![],
            reasoning_content: None,
        },
    ]);
    let calls = provider.calls.clone();

    let agent = create_test_agent(provider, &tmp).await;

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

    // First response: LLM requests a tool call (list_dir which is registered by default)
    // Second response: LLM returns final text after seeing tool result
    let provider = MockLLMProvider::with_responses(vec![
        LLMResponse {
            content: None,
            tool_calls: vec![ToolCallRequest {
                id: "tc1".to_string(),
                name: "list_dir".to_string(),
                arguments: json!({"path": tmp.path().to_str().unwrap()}),
            }],
            reasoning_content: None,
        },
        LLMResponse {
            content: Some("I listed the directory for you.".to_string()),
            tool_calls: vec![],
            reasoning_content: None,
        },
    ]);
    let calls = provider.calls.clone();

    let agent = create_test_agent(provider, &tmp).await;

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

    // LLM requests a tool that doesn't exist
    let provider = MockLLMProvider::with_responses(vec![
        LLMResponse {
            content: None,
            tool_calls: vec![ToolCallRequest {
                id: "tc_bad".to_string(),
                name: "nonexistent_tool".to_string(),
                arguments: json!({}),
            }],
            reasoning_content: None,
        },
        LLMResponse {
            content: Some("Sorry, that tool wasn't available.".to_string()),
            tool_calls: vec![],
            reasoning_content: None,
        },
    ]);
    let calls = provider.calls.clone();

    let agent = create_test_agent(provider, &tmp).await;

    let response = agent
        .process_direct("Use magic tool", "test:unknown", "telegram", "unknown")
        .await
        .unwrap();

    assert_eq!(response, "Sorry, that tool wasn't available.");

    // The second call should have a tool result with an error
    let recorded = calls.lock().unwrap();
    let second_msgs = &recorded[1].messages;
    let tool_msg = second_msgs.iter().find(|m| m.role == "tool").unwrap();
    assert!(tool_msg.content.contains("unknown tool"));
    assert!(tool_msg.is_error);
}

#[tokio::test]
async fn test_provider_called_with_tools() {
    let tmp = TempDir::new().unwrap();
    let provider = MockLLMProvider::new();
    let calls = provider.calls.clone();

    let agent = create_test_agent(provider, &tmp).await;

    agent
        .process_direct("Hello", "test:tools_check", "telegram", "tools_check")
        .await
        .unwrap();

    let recorded = calls.lock().unwrap();
    assert_eq!(recorded.len(), 1);

    // Should have tool definitions passed to the provider
    let tools = recorded[0].tools.as_ref().unwrap();
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
        LLMResponse {
            content: None,
            tool_calls: vec![
                ToolCallRequest {
                    id: "tc1".to_string(),
                    name: "list_dir".to_string(),
                    arguments: json!({"path": tmp.path().to_str().unwrap()}),
                },
                ToolCallRequest {
                    id: "tc2".to_string(),
                    name: "read_file".to_string(),
                    arguments: json!({"path": tmp.path().join("test.txt").to_str().unwrap()}),
                },
            ],
            reasoning_content: None,
        },
        // Second iteration: final response
        LLMResponse {
            content: Some("Done reading files.".to_string()),
            tool_calls: vec![],
            reasoning_content: None,
        },
    ]);

    let agent = create_test_agent(provider, &tmp).await;

    let response = agent
        .process_direct("Read files", "test:multi", "telegram", "multi")
        .await
        .unwrap();

    assert_eq!(response, "Done reading files.");
}

#[tokio::test]
async fn test_hallucination_detection_triggers_retry() {
    let tmp = TempDir::new().unwrap();

    // First response: LLM claims it did something without calling tools
    // Second response (after correction): LLM gives honest answer
    let provider = MockLLMProvider::with_responses(vec![
        LLMResponse {
            content: Some("I've updated the configuration file for you.".to_string()),
            tool_calls: vec![],
            reasoning_content: None,
        },
        LLMResponse {
            content: Some(
                "I can help you update the configuration. Which file would you like me to edit?"
                    .to_string(),
            ),
            tool_calls: vec![],
            reasoning_content: None,
        },
    ]);
    let calls = provider.calls.clone();

    let agent = create_test_agent(provider, &tmp).await;

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

    // LLM uses a tool, then claims action — this is legitimate
    let provider = MockLLMProvider::with_responses(vec![
        LLMResponse {
            content: None,
            tool_calls: vec![ToolCallRequest {
                id: "tc1".to_string(),
                name: "list_dir".to_string(),
                arguments: json!({"path": tmp.path().to_str().unwrap()}),
            }],
            reasoning_content: None,
        },
        LLMResponse {
            content: Some("I've listed the directory for you.".to_string()),
            tool_calls: vec![],
            reasoning_content: None,
        },
    ]);
    let calls = provider.calls.clone();

    let agent = create_test_agent(provider, &tmp).await;

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

    // LLM gives an informational response without claiming actions
    let provider = MockLLMProvider::with_responses(vec![LLMResponse {
        content: Some("To update the config, you need to edit the settings.json file.".to_string()),
        tool_calls: vec![],
        reasoning_content: None,
    }]);
    let calls = provider.calls.clone();

    let agent = create_test_agent(provider, &tmp).await;

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
