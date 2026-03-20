mod common;

use common::{
    MockLLMProvider, TestAgentOverrides, create_test_agent_with, text_response, tool_call,
    tool_response,
};
use oxicrab::agent::approval::ApprovalDecision;
use oxicrab::config::ApprovalConfig;
use serde_json::json;
use std::time::Duration;
use tempfile::TempDir;

// ===========================================================================
// Test 1: Approval disabled — tools execute freely
// ===========================================================================

#[tokio::test]
async fn test_approval_disabled_tools_execute_freely() {
    let tmp = TempDir::new().expect("create temp dir");

    // Provider returns a write_file tool call, then a text response
    let provider = MockLLMProvider::with_responses(vec![
        tool_response(vec![tool_call(
            "tc1",
            "write_file",
            json!({
                "path": tmp.path().join("test_output.txt").to_str().unwrap(),
                "content": "hello from approval test"
            }),
        )]),
        text_response("File written successfully."),
    ]);

    let agent = create_test_agent_with(
        provider,
        &tmp,
        TestAgentOverrides {
            // Approval disabled (default)
            approval_config: Some(ApprovalConfig::default()),
            restrict_to_workspace: Some(true),
            ..Default::default()
        },
    )
    .await;

    let response = agent
        .process_direct(
            "Write a file",
            "test:appr_disabled",
            "telegram",
            "appr_disabled",
        )
        .await
        .expect("process message");

    // Tool should have executed without any approval gating
    assert_eq!(response, "File written successfully.");

    // Verify the file was actually written
    let content =
        std::fs::read_to_string(tmp.path().join("test_output.txt")).expect("read written file");
    assert_eq!(content, "hello from approval test");
}

// ===========================================================================
// Test 2: Approval disabled — legacy requires_approval block still active
// ===========================================================================

#[tokio::test]
async fn test_approval_disabled_legacy_block_still_active() {
    let tmp = TempDir::new().expect("create temp dir");

    // Provider calls github.create_issue which has requires_approval_for_action = true
    let provider = MockLLMProvider::with_responses(vec![
        tool_response(vec![tool_call(
            "tc1",
            "github",
            json!({
                "action": "create_issue",
                "owner": "test",
                "repo": "test",
                "title": "test issue"
            }),
        )]),
        text_response("I was unable to create the issue due to approval restrictions."),
    ]);
    let calls = provider.calls.clone();

    let agent = create_test_agent_with(
        provider,
        &tmp,
        TestAgentOverrides {
            // Approval disabled — legacy block should still apply
            approval_config: Some(ApprovalConfig::default()),
            ..Default::default()
        },
    )
    .await;

    // Check if github tool is registered (it won't be without config)
    let registry = agent.tool_registry();
    if registry.get("github").is_none() {
        // GitHub tool not registered — skip this test gracefully.
        // The legacy block is only testable when the tool is registered.
        // This is expected in the default test setup (no GitHub config).
        eprintln!(
            "SKIP: GitHub tool not registered (no config). \
             Legacy block tested via unit tests in helpers.rs."
        );
        return;
    }

    let _response = agent
        .process_direct(
            "Create a GitHub issue",
            "test:legacy_block",
            "telegram",
            "legacy_block",
        )
        .await
        .expect("process message");

    // The second LLM call should have received the tool error
    let recorded = calls.lock().expect("lock");
    assert!(recorded.len() >= 2, "should have at least 2 LLM calls");
    let second_msgs = &recorded[1].messages;
    let tool_msg = second_msgs.iter().find(|m| m.role == "tool");
    assert!(
        tool_msg.is_some(),
        "second call should include a tool result"
    );
    let tool_content = &tool_msg.unwrap().content;
    assert!(
        tool_content.contains("requires approval"),
        "tool result should mention approval requirement, got: {tool_content}"
    );
}

// ===========================================================================
// Test 3: Approval enabled — timeout auto-denies
// ===========================================================================

#[tokio::test]
async fn test_approval_enabled_timeout_auto_denies() {
    let tmp = TempDir::new().expect("create temp dir");

    // Provider: first returns a write_file call, then responds after getting timeout error
    let provider = MockLLMProvider::with_responses(vec![
        tool_response(vec![tool_call(
            "tc1",
            "write_file",
            json!({
                "path": tmp.path().join("should_not_exist.txt").to_str().unwrap(),
                "content": "should never be written"
            }),
        )]),
        text_response("The write operation was denied due to timeout."),
    ]);
    let calls = provider.calls.clone();

    let agent = create_test_agent_with(
        provider,
        &tmp,
        TestAgentOverrides {
            approval_config: Some(ApprovalConfig {
                enabled: true,
                channel: String::new(), // self-approval mode
                timeout: 1,             // 1 second timeout
                actions: vec![],        // empty = all mutating actions
            }),
            restrict_to_workspace: Some(true),
            ..Default::default()
        },
    )
    .await;

    let response = agent
        .process_direct(
            "Write a file",
            "test:appr_timeout",
            "telegram",
            "appr_timeout",
        )
        .await
        .expect("process message");

    assert_eq!(response, "The write operation was denied due to timeout.");

    // Verify the file was NOT written (approval timed out)
    assert!(
        !tmp.path().join("should_not_exist.txt").exists(),
        "file should not exist after approval timeout"
    );

    // Check that the LLM received the timeout error as a tool result
    let recorded = calls.lock().expect("lock");
    assert!(recorded.len() >= 2, "should have at least 2 LLM calls");
    let second_msgs = &recorded[1].messages;
    let tool_msg = second_msgs
        .iter()
        .find(|m| m.role == "tool")
        .expect("should have tool result message");
    assert!(
        tool_msg.content.contains("timed out"),
        "tool result should mention timeout, got: {}",
        tool_msg.content
    );
    assert!(tool_msg.is_error, "tool result should be flagged as error");
}

// ===========================================================================
// Test 4: Approval enabled — approve flow
// ===========================================================================

#[tokio::test]
async fn test_approval_enabled_approve_flow() {
    let tmp = TempDir::new().expect("create temp dir");
    let target_path = tmp.path().join("approved_file.txt");

    let provider = MockLLMProvider::with_responses(vec![
        tool_response(vec![tool_call(
            "tc1",
            "write_file",
            json!({
                "path": target_path.to_str().unwrap(),
                "content": "approved content"
            }),
        )]),
        text_response("File written after approval."),
    ]);

    let agent = create_test_agent_with(
        provider,
        &tmp,
        TestAgentOverrides {
            approval_config: Some(ApprovalConfig {
                enabled: true,
                channel: String::new(), // self-approval mode
                timeout: 10,            // 10 seconds (plenty of time)
                actions: vec![],        // empty = all mutating actions
            }),
            restrict_to_workspace: Some(true),
            ..Default::default()
        },
    )
    .await;

    let store = agent.approval_store();

    // Spawn a background task that waits briefly then approves
    let store_clone = store.clone();
    tokio::spawn(async move {
        // Poll for the pending approval (it may take a moment to register)
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let ids = store_clone.pending_ids();
            if let Some(id) = ids.first() {
                // Resolve with Approved — empty source channel matches self-approval
                let result = store_clone.resolve(id, "", ApprovalDecision::Approved);
                assert!(result.is_ok(), "resolve should succeed: {:?}", result);
                return;
            }
        }
        panic!("no pending approval found within timeout");
    });

    let response = agent
        .process_direct(
            "Write a file",
            "test:appr_approve",
            "telegram",
            "appr_approve",
        )
        .await
        .expect("process message");

    assert_eq!(response, "File written after approval.");

    // Verify the file WAS written (approval granted)
    let content = std::fs::read_to_string(&target_path).expect("read approved file");
    assert_eq!(content, "approved content");
}

// ===========================================================================
// Test 5: Approval enabled — deny flow
// ===========================================================================

#[tokio::test]
async fn test_approval_enabled_deny_flow() {
    let tmp = TempDir::new().expect("create temp dir");
    let target_path = tmp.path().join("denied_file.txt");

    let provider = MockLLMProvider::with_responses(vec![
        tool_response(vec![tool_call(
            "tc1",
            "write_file",
            json!({
                "path": target_path.to_str().unwrap(),
                "content": "should never be written"
            }),
        )]),
        text_response("The write was denied by the operator."),
    ]);
    let calls = provider.calls.clone();

    let agent = create_test_agent_with(
        provider,
        &tmp,
        TestAgentOverrides {
            approval_config: Some(ApprovalConfig {
                enabled: true,
                channel: String::new(), // self-approval mode
                timeout: 10,            // 10 seconds
                actions: vec![],        // empty = all mutating actions
            }),
            restrict_to_workspace: Some(true),
            ..Default::default()
        },
    )
    .await;

    let store = agent.approval_store();

    // Spawn a background task that waits briefly then denies
    let store_clone = store.clone();
    tokio::spawn(async move {
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let ids = store_clone.pending_ids();
            if let Some(id) = ids.first() {
                let result = store_clone.resolve(id, "", ApprovalDecision::Denied { reason: None });
                assert!(result.is_ok(), "resolve should succeed: {:?}", result);
                return;
            }
        }
        panic!("no pending approval found within timeout");
    });

    let response = agent
        .process_direct("Write a file", "test:appr_deny", "telegram", "appr_deny")
        .await
        .expect("process message");

    assert_eq!(response, "The write was denied by the operator.");

    // Verify the file was NOT written (approval denied)
    assert!(!target_path.exists(), "file should not exist after denial");

    // Check that the LLM received the denial error as a tool result
    let recorded = calls.lock().expect("lock");
    assert!(recorded.len() >= 2, "should have at least 2 LLM calls");
    let second_msgs = &recorded[1].messages;
    let tool_msg = second_msgs
        .iter()
        .find(|m| m.role == "tool")
        .expect("should have tool result message");
    assert!(
        tool_msg.content.contains("denied by operator"),
        "tool result should mention denial, got: {}",
        tool_msg.content
    );
    assert!(tool_msg.is_error, "tool result should be flagged as error");
}
