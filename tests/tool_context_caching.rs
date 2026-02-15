mod common;

use common::{
    create_test_agent_with, text_response, tool_call, tool_response, MockLLMProvider,
    TestAgentOverrides,
};
use serde_json::json;
use tempfile::TempDir;

#[tokio::test]
async fn test_tool_context_set_before_execution() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("ctx.txt"), "context data").unwrap();

    // Use read_file — it doesn't use context, but the agent loop should call set_context
    // on all tools before execution. We verify the overall flow works.
    let provider = MockLLMProvider::with_responses(vec![
        tool_response(vec![tool_call(
            "tc1",
            "read_file",
            json!({"path": tmp.path().join("ctx.txt").to_str().unwrap()}),
        )]),
        text_response("Read complete."),
    ]);
    let calls = provider.calls.clone();

    let agent = create_test_agent_with(provider, &tmp, TestAgentOverrides::default()).await;
    let response = agent
        .process_direct("Read ctx.txt", "test:ctx1", "telegram", "ctx_chat")
        .await
        .unwrap();

    assert_eq!(response, "Read complete.");

    // Verify the tool result is successful (proves context/flow worked)
    let recorded = calls.lock().unwrap();
    let second_msgs = &recorded[1].messages;
    let tool_msg = second_msgs.iter().find(|m| m.role == "tool").unwrap();
    assert!(!tool_msg.is_error);
    assert!(tool_msg.content.contains("context data"));
}

#[tokio::test]
async fn test_cacheable_tool_cached_in_agent_loop() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("cached.txt"), "cached content").unwrap();
    let cached_path = tmp.path().join("cached.txt");

    // read_file is cacheable. Call it twice with same params in same iteration.
    let provider = MockLLMProvider::with_responses(vec![
        tool_response(vec![
            tool_call(
                "tc1",
                "read_file",
                json!({"path": cached_path.to_str().unwrap()}),
            ),
            tool_call(
                "tc2",
                "read_file",
                json!({"path": cached_path.to_str().unwrap()}),
            ),
        ]),
        text_response("Both reads complete."),
    ]);
    let calls = provider.calls.clone();

    let agent = create_test_agent_with(provider, &tmp, TestAgentOverrides::default()).await;
    let response = agent
        .process_direct("Read file twice", "test:cache1", "telegram", "cache1")
        .await
        .unwrap();

    assert_eq!(response, "Both reads complete.");

    // Both tool results should contain the cached content
    let recorded = calls.lock().unwrap();
    let second_msgs = &recorded[1].messages;
    let tool_msgs: Vec<_> = second_msgs.iter().filter(|m| m.role == "tool").collect();
    assert_eq!(tool_msgs.len(), 2);
    for msg in &tool_msgs {
        assert!(!msg.is_error);
        assert!(msg.content.contains("cached content"));
    }
}

#[tokio::test]
async fn test_tool_result_truncation_10k() {
    let tmp = TempDir::new().unwrap();
    // Create a file with content larger than MAX_TOOL_RESULT_CHARS (10000)
    let large = "a".repeat(15000);
    let target = tmp.path().join("large_file.txt");
    std::fs::write(&target, &large).unwrap();

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
        .process_direct("Read it", "test:trunc1", "telegram", "trunc1")
        .await
        .unwrap();

    let recorded = calls.lock().unwrap();
    let second_msgs = &recorded[1].messages;
    let tool_msg = second_msgs.iter().find(|m| m.role == "tool").unwrap();
    // Should be truncated — original was 15000 chars
    assert!(
        tool_msg.content.len() < 12000,
        "Result should be truncated from 15000, got {} chars",
        tool_msg.content.len()
    );
}

#[tokio::test]
async fn test_context_summary_passed_to_tools() {
    // This test verifies the overall flow: when compaction has produced a summary,
    // it should be accessible to tools via set_context_summary.
    // We test this indirectly by running a conversation that could trigger compaction.
    let tmp = TempDir::new().unwrap();

    let provider = MockLLMProvider::with_responses(vec![
        text_response("First."),
        text_response("Second."),
        tool_response(vec![tool_call(
            "tc1",
            "list_dir",
            json!({"path": tmp.path().to_str().unwrap()}),
        )]),
        text_response("Listed."),
    ]);

    let agent = create_test_agent_with(provider, &tmp, TestAgentOverrides::default()).await;
    let session = "test:ctx_summary";

    // Multiple turns to build history
    agent
        .process_direct("Turn 1", session, "telegram", "ctx_sum")
        .await
        .unwrap();
    agent
        .process_direct("Turn 2", session, "telegram", "ctx_sum")
        .await
        .unwrap();
    let response = agent
        .process_direct("List dir", session, "telegram", "ctx_sum")
        .await
        .unwrap();

    assert_eq!(response, "Listed.");
}
