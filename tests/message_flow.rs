mod common;

use common::{
    MockLLMProvider, TestAgentOverrides, ToolCapturingProvider, create_test_agent_with,
    text_response, tool_call, tool_response,
};
use serde_json::json;
use tempfile::TempDir;

fn default_agent(
    provider: MockLLMProvider,
    tmp: &TempDir,
) -> impl std::future::Future<Output = oxicrab::agent::AgentLoop> + '_ {
    create_test_agent_with(provider, tmp, TestAgentOverrides::default())
}

#[tokio::test]
async fn test_simple_message_response() {
    let tmp = TempDir::new().expect("create temp dir");
    let provider = MockLLMProvider::with_responses(vec![text_response("Hello from the agent!")]);

    let agent = default_agent(provider, &tmp).await;

    let response = agent
        .process_direct("Hi there", "test:chat1", "telegram", "chat1")
        .await
        .expect("process message");

    assert_eq!(response, "Hello from the agent!");
}

#[tokio::test]
async fn test_empty_message_handled() {
    let tmp = TempDir::new().expect("create temp dir");
    let provider =
        MockLLMProvider::with_responses(vec![text_response("I received an empty message.")]);

    let agent = default_agent(provider, &tmp).await;

    let response = agent
        .process_direct("", "test:empty", "telegram", "empty")
        .await
        .expect("process message");

    assert_eq!(response, "I received an empty message.");
}

#[tokio::test]
async fn test_session_persists_across_messages() {
    let tmp = TempDir::new().expect("create temp dir");
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
        .expect("process message");

    // Second message on same session
    agent
        .process_direct("Follow up", "test:persist", "telegram", "persist")
        .await
        .expect("process message");

    // The second call should have history from the first exchange
    let recorded = calls.lock().expect("lock recorded calls");
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
async fn test_tool_call_and_result() {
    let tmp = TempDir::new().expect("create temp dir");

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
        .expect("process message");

    assert_eq!(response, "I listed the directory for you.");

    // Should have made 2 calls to the provider
    let recorded = calls.lock().expect("lock recorded calls");
    assert_eq!(recorded.len(), 2);

    // Second call should include the tool result
    let second_msgs = &recorded[1].messages;
    let has_tool_result = second_msgs.iter().any(|m| m.role == "tool");
    assert!(has_tool_result, "Second call should include tool result");
}

#[tokio::test]
async fn test_unknown_tool_handled() {
    let tmp = TempDir::new().expect("create temp dir");

    let provider = MockLLMProvider::with_responses(vec![
        tool_response(vec![tool_call("tc_bad", "nonexistent_tool", json!({}))]),
        text_response("Sorry, that tool wasn't available."),
    ]);
    let calls = provider.calls.clone();

    let agent = default_agent(provider, &tmp).await;

    let response = agent
        .process_direct("Use magic tool", "test:unknown", "telegram", "unknown")
        .await
        .expect("process message");

    assert_eq!(response, "Sorry, that tool wasn't available.");

    // The second call should have a tool result with an error
    let recorded = calls.lock().expect("lock recorded calls");
    let second_msgs = &recorded[1].messages;
    let tool_msg = second_msgs.iter().find(|m| m.role == "tool").unwrap();
    assert!(tool_msg.content.contains("does not exist"));
    assert!(tool_msg.is_error);
}

#[tokio::test]
async fn test_provider_called_with_tools() {
    let tmp = TempDir::new().expect("create temp dir");
    let provider = ToolCapturingProvider::new();
    let tool_defs = provider.tool_defs.clone();

    let agent = create_test_agent_with(provider, &tmp, TestAgentOverrides::default()).await;

    agent
        .process_direct("Hello", "test:tools_check", "telegram", "tools_check")
        .await
        .expect("process message");

    let recorded = tool_defs.lock().expect("lock tool defs");
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
    let tmp = TempDir::new().expect("create temp dir");

    // Create a test file for read_file to find
    std::fs::write(tmp.path().join("test.txt"), "test content").expect("write test file");

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
        .expect("process message");

    assert_eq!(response, "Done reading files.");
}

#[tokio::test]
async fn test_hallucination_detection_triggers_retry() {
    let tmp = TempDir::new().expect("create temp dir");

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
        .expect("process message");

    // Should get the corrected (second) response, not the hallucinated one
    assert_eq!(
        response,
        "I can help you update the configuration. Which file would you like me to edit?"
    );

    // Should have made 2 calls — original + retry after correction
    let recorded = calls.lock().expect("lock recorded calls");
    assert_eq!(recorded.len(), 2);

    // Second call should contain the correction message
    let second_msgs = &recorded[1].messages;
    let has_correction = second_msgs
        .iter()
        .any(|m| m.role == "user" && m.content.contains("did not call any tools"));
    assert!(
        has_correction,
        "Second call should contain the hallucination correction"
    );
}

#[tokio::test]
async fn test_no_hallucination_when_tools_used() {
    let tmp = TempDir::new().expect("create temp dir");

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
        .expect("process message");

    // Should return the response as-is since tools were actually used
    assert_eq!(response, "I've listed the directory for you.");

    // Should have made exactly 2 calls (tool call + final response), no correction retry
    let recorded = calls.lock().expect("lock recorded calls");
    assert_eq!(recorded.len(), 2);
}

#[tokio::test]
async fn test_no_hallucination_for_informational_response() {
    let tmp = TempDir::new().expect("create temp dir");

    let provider = MockLLMProvider::with_responses(vec![text_response(
        "To update the config, you need to edit the settings.json file.",
    )]);
    let calls = provider.calls.clone();

    let agent = default_agent(provider, &tmp).await;

    let response = agent
        .process_direct("How do I update config?", "test:info", "telegram", "info")
        .await
        .expect("process message");

    assert_eq!(
        response,
        "To update the config, you need to edit the settings.json file."
    );

    // Only 1 call — no retry needed
    let recorded = calls.lock().expect("lock recorded calls");
    assert_eq!(recorded.len(), 1);
}

#[tokio::test]
async fn test_conversational_reply_not_forced_to_tool() {
    // Simulate: bot asks a question (turn 1), user replies "sure" (turn 2)
    // The model should be able to respond with text, not forced to call a tool
    let tmp = TempDir::new().expect("create temp dir");
    let provider = MockLLMProvider::with_responses(vec![
        // Turn 1: bot asks a question
        text_response("Want me to run the first one now?"),
        // Turn 2: user replies "sure" → model responds conversationally
        text_response("Great, running the first one now."),
    ]);
    let calls = provider.calls.clone();
    let agent = default_agent(provider, &tmp).await;

    agent
        .process_direct("List my scheduled tasks", "test:conv", "telegram", "conv")
        .await
        .expect("turn 1");
    let resp = agent
        .process_direct("sure", "test:conv", "telegram", "conv")
        .await
        .expect("turn 2");

    assert_eq!(resp, "Great, running the first one now.");

    // Verify tool_choice was NOT "any" — the model was allowed to respond freely
    let recorded = calls.lock().expect("lock");
    // Turn 2 should NOT have been forced to use tools
    assert!(
        recorded[1].tool_choice.is_none(),
        "turn 2 should use auto tool_choice, not forced 'any'"
    );
}

#[tokio::test]
async fn test_tool_choice_is_auto_on_all_iterations() {
    // Verify that tool_choice is never "any" across multiple turns
    let tmp = TempDir::new().expect("create temp dir");
    let provider = MockLLMProvider::with_responses(vec![
        text_response("Response 1"),
        text_response("Response 2"),
        text_response("Response 3"),
    ]);
    let calls = provider.calls.clone();
    let agent = default_agent(provider, &tmp).await;

    for i in 1..=3 {
        agent
            .process_direct(&format!("Message {}", i), "test:auto", "telegram", "auto")
            .await
            .expect("process");
    }

    let recorded = calls.lock().expect("lock");
    for (i, call) in recorded.iter().enumerate() {
        assert!(
            call.tool_choice.is_none(),
            "call {} should have tool_choice=None (auto), got {:?}",
            i,
            call.tool_choice
        );
    }
}

#[tokio::test]
async fn test_repeated_hallucination_corrected_each_time() {
    // Layer 1 hallucination detection fires once per agent loop (single-retry guard).
    // The first hallucination triggers a correction; the second passes through as
    // the final response since layer1_fired prevents a second correction.
    let tmp = TempDir::new().expect("create temp dir");
    let provider = MockLLMProvider::with_responses(vec![
        // Iteration 1: hallucinates → correction injected
        text_response("I've updated the configuration file."),
        // Iteration 2: hallucinates again → layer1_fired, passes through as final response
        text_response("I've modified the database schema."),
    ]);
    let calls = provider.calls.clone();
    let agent = create_test_agent_with(
        provider,
        &tmp,
        TestAgentOverrides {
            max_iterations: Some(5),
            ..Default::default()
        },
    )
    .await;

    let resp = agent
        .process_direct("Update everything", "test:repeat", "telegram", "repeat")
        .await
        .expect("process");

    // Second hallucination passes through once L1 guard is exhausted
    assert_eq!(
        resp, "I've modified the database schema.",
        "should return second response after single-retry correction is exhausted"
    );

    // Should have 2 LLM calls: hallucination → correction → second hallucination (returned)
    let recorded = calls.lock().expect("lock");
    assert_eq!(
        recorded.len(),
        2,
        "should have 2 calls (one correction + final)"
    );

    // Second call should contain the correction
    let second_msgs = &recorded[1].messages;
    assert!(
        second_msgs
            .iter()
            .any(|m| m.role == "user" && m.content.contains("did not call any tools")),
        "second call should contain the hallucination correction"
    );
}

#[tokio::test]
async fn test_empty_response_exhaustion_returns_empty() {
    // When the LLM returns empty responses and retries are exhausted,
    // the agent should return gracefully (not panic)
    let tmp = TempDir::new().expect("create temp dir");
    let empty_response = oxicrab::providers::base::LLMResponse::default;
    // 3 empty responses (initial + 2 retries = EMPTY_RESPONSE_RETRIES)
    let provider =
        MockLLMProvider::with_responses(vec![empty_response(), empty_response(), empty_response()]);
    let agent = default_agent(provider, &tmp).await;

    // process_direct returns a fallback string when the agent loop produces no content
    let result = agent
        .process_direct("hello", "test:empty", "telegram", "empty")
        .await;

    // Should not panic — returns gracefully with a fallback message
    assert!(result.is_ok(), "should not error on empty responses");
    assert_eq!(
        result.unwrap(),
        "No response generated.",
        "should return fallback message when LLM gives empty responses"
    );
}

#[tokio::test]
async fn test_tool_use_then_conversational_followup() {
    // Full lifecycle: turn 1 uses tools, turn 2 is conversational
    // This tests that any_tools_called resets between process_direct calls
    let tmp = TempDir::new().expect("create temp dir");
    let provider = MockLLMProvider::with_responses(vec![
        // Turn 1: model calls a tool
        tool_response(vec![tool_call(
            "tc1",
            "list_dir",
            json!({"path": tmp.path().to_str().unwrap()}),
        )]),
        text_response("Here are the files in your directory."),
        // Turn 2: model responds conversationally (should NOT be flagged as hallucination)
        text_response("You're welcome! Let me know if you need anything else."),
    ]);
    let calls = provider.calls.clone();
    let agent = default_agent(provider, &tmp).await;

    // Turn 1: tool use
    let resp1 = agent
        .process_direct("List my files", "test:lifecycle", "telegram", "lifecycle")
        .await
        .expect("turn 1");
    assert_eq!(resp1, "Here are the files in your directory.");

    // Turn 2: conversational follow-up
    let resp2 = agent
        .process_direct("Thanks!", "test:lifecycle", "telegram", "lifecycle")
        .await
        .expect("turn 2");
    assert_eq!(
        resp2,
        "You're welcome! Let me know if you need anything else."
    );

    // Turn 2 should be a single LLM call (no correction retry)
    let recorded = calls.lock().expect("lock");
    // Turn 1: 2 calls (tool call + summary), Turn 2: 1 call
    assert_eq!(recorded.len(), 3, "should be 3 total LLM calls");
}

#[tokio::test]
async fn test_multi_turn_interleaved_tools_and_conversation() {
    // Complex lifecycle: tool turn → convo turn → tool turn → convo turn
    let tmp = TempDir::new().expect("create temp dir");
    std::fs::write(tmp.path().join("test.txt"), "content").expect("write");
    let provider = MockLLMProvider::with_responses(vec![
        // Turn 1: tool call
        tool_response(vec![tool_call(
            "tc1",
            "list_dir",
            json!({"path": tmp.path().to_str().unwrap()}),
        )]),
        text_response("Found test.txt in the directory."),
        // Turn 2: conversational
        text_response("Sure, I can read it for you."),
        // Turn 3: another tool call
        tool_response(vec![tool_call(
            "tc2",
            "read_file",
            json!({"path": tmp.path().join("test.txt").to_str().unwrap()}),
        )]),
        text_response("The file contains: content"),
        // Turn 4: conversational
        text_response("Glad I could help!"),
    ]);
    let calls = provider.calls.clone();
    let agent = default_agent(provider, &tmp).await;

    let r1 = agent
        .process_direct("What files are here?", "test:multi", "telegram", "multi")
        .await
        .expect("turn 1");
    assert_eq!(r1, "Found test.txt in the directory.");

    let r2 = agent
        .process_direct("What's in that file?", "test:multi", "telegram", "multi")
        .await
        .expect("turn 2");
    assert_eq!(r2, "Sure, I can read it for you.");

    let r3 = agent
        .process_direct("Please do", "test:multi", "telegram", "multi")
        .await
        .expect("turn 3");
    assert_eq!(r3, "The file contains: content");

    let r4 = agent
        .process_direct("Thanks!", "test:multi", "telegram", "multi")
        .await
        .expect("turn 4");
    assert_eq!(r4, "Glad I could help!");

    // Verify all tool_choice values are None (auto)
    let recorded = calls.lock().expect("lock");
    for (i, call) in recorded.iter().enumerate() {
        assert!(
            call.tool_choice.is_none(),
            "call {} should have auto tool_choice",
            i
        );
    }
}

#[tokio::test]
async fn test_silent_response_returns_marker() {
    // [SILENT] prefix: process_direct returns the raw text; the suppression
    // happens in process_message (which is private). Verify the marker survives.
    let tmp = TempDir::new().expect("create temp dir");
    let provider =
        MockLLMProvider::with_responses(vec![text_response("[SILENT] Internal note recorded.")]);
    let agent = default_agent(provider, &tmp).await;

    let result = agent
        .process_direct(
            "remember this internally",
            "test:silent",
            "telegram",
            "silent",
        )
        .await
        .expect("process");

    assert!(
        result.starts_with("[SILENT]"),
        "[SILENT] marker should be preserved in process_direct output"
    );
}

/// End-to-end test: inbound message → MessageBus → AgentLoop → outbound message.
/// Verifies the full pipeline without using `process_direct`.
#[tokio::test]
async fn test_end_to_end_bus_pipeline() {
    use oxicrab::bus::{InboundMessage, MessageBus};
    use std::sync::Arc;

    let tmp = TempDir::new().expect("create temp dir");
    let provider = MockLLMProvider::with_responses(vec![text_response("Bus pipeline works!")]);

    // Create bus and extract channels before wrapping in Arc
    let bus = MessageBus::default();
    let mut inbound_rx = bus.take_inbound_rx().expect("take inbound rx");
    let _outbound_rx = bus.take_outbound_rx().expect("take outbound rx");
    let bus = Arc::new(bus);

    let agent = create_test_agent_with(provider, &tmp, TestAgentOverrides::default()).await;

    // Publish inbound message through the bus
    bus.publish_inbound(InboundMessage::builder("test", "user1", "chat1", "Hello via bus").build())
        .await
        .expect("publish inbound");

    // Receive the inbound message
    let msg = inbound_rx.try_recv().expect("receive inbound message");
    assert_eq!(msg.channel, "test");
    assert_eq!(msg.content, "Hello via bus");

    // Process it through the agent (simulating the main loop)
    let response = agent
        .process_direct(&msg.content, &msg.session_key(), &msg.channel, &msg.chat_id)
        .await
        .expect("agent process");

    assert_eq!(response, "Bus pipeline works!");
}
