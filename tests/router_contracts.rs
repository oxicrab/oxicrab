mod common;

use common::{
    MockLLMProvider, TestAgentOverrides, ToolCapturingProvider, create_test_agent_with,
    text_response, tool_call, tool_response,
};
use oxicrab::agent::AgentRunOverrides;

#[tokio::test]
async fn contract_policy_filters_tools_passed_to_model() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let provider = ToolCapturingProvider::with_responses(vec![text_response("ok")]);
    let captured = provider.tool_defs.clone();
    let agent = create_test_agent_with(provider, &tmp, TestAgentOverrides::default()).await;

    let overrides = AgentRunOverrides {
        routing_policy: Some(oxicrab::router::RoutingPolicy {
            allowed_tools: vec!["list_dir".to_string()],
            blocked_tools: vec![],
            context_hint: Some("focus files".to_string()),
            reason: "test_contract",
        }),
        ..Default::default()
    };

    let _ = agent
        .process_direct_with_overrides(
            "list files",
            "test:policy_filter",
            "telegram",
            "chat1",
            &overrides,
        )
        .await
        .expect("process_direct_with_overrides");

    let defs_guard = captured.lock().expect("lock");
    let first = defs_guard
        .first()
        .and_then(|d| d.as_ref())
        .expect("first request should include tools");
    let names: Vec<&str> = first.iter().map(|t| t.name.as_str()).collect();

    assert!(
        names.contains(&"list_dir"),
        "expected list_dir in filtered set, got {:?}",
        names
    );
    assert!(names.contains(&"add_buttons"));
    assert!(!names.contains(&"exec"));
}

#[tokio::test]
async fn contract_blocked_tool_call_is_enforced_before_execution() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let provider = MockLLMProvider::with_responses(vec![
        tool_response(vec![tool_call(
            "tc-1",
            "list_dir",
            serde_json::json!({"path": "."}),
        )]),
        text_response("done"),
    ]);
    let calls = provider.calls.clone();
    let agent = create_test_agent_with(provider, &tmp, TestAgentOverrides::default()).await;

    let overrides = AgentRunOverrides {
        routing_policy: Some(oxicrab::router::RoutingPolicy {
            allowed_tools: vec!["list_dir".to_string()],
            blocked_tools: vec!["list_dir".to_string()],
            context_hint: None,
            reason: "test_contract_block",
        }),
        ..Default::default()
    };

    let _ = agent
        .process_direct_with_overrides(
            "please do it",
            "test:policy_block",
            "telegram",
            "chat1",
            &overrides,
        )
        .await
        .expect("process_direct_with_overrides");

    let call_log = calls.lock().expect("lock calls");
    assert!(
        call_log.len() >= 2,
        "expected follow-up call after tool error"
    );
    let second_messages = &call_log[1].messages;
    let saw_block_error = second_messages
        .iter()
        .any(|m| m.role == "tool" && m.content.contains("not allowed in this routed turn"));
    assert!(
        saw_block_error,
        "expected blocked-tool error to be injected into next model turn"
    );
}
