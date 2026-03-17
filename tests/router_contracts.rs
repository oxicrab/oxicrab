mod common;

use common::{
    MockLLMProvider, TestAgentOverrides, ToolCapturingProvider, create_test_agent_with,
    text_response, tool_call, tool_response,
};
use oxicrab::agent::AgentRunOverrides;
use oxicrab::dispatch::{ActionDispatch, ActionSource};
use oxicrab::router::context::{ActionDirective, RouterContext};
use oxicrab::router::{MessageRouter, RoutingDecision};

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

#[tokio::test]
async fn contract_invalid_tool_params_are_rejected_before_execution() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let provider = MockLLMProvider::with_responses(vec![
        tool_response(vec![tool_call("tc-1", "list_dir", serde_json::json!({}))]),
        text_response("done"),
    ]);
    let calls = provider.calls.clone();
    let agent = create_test_agent_with(provider, &tmp, TestAgentOverrides::default()).await;

    let _ = agent
        .process_direct("list files", "test:param_validation", "telegram", "chat1")
        .await
        .expect("process_direct");

    let call_log = calls.lock().expect("lock calls");
    assert!(
        call_log.len() >= 2,
        "expected follow-up turn containing tool validation error"
    );
    let second_messages = &call_log[1].messages;
    let saw_validation_error = second_messages.iter().any(|m| {
        m.role == "tool"
            && m.content.contains("Invalid arguments for tool 'list_dir'")
            && m.content.contains("missing required parameter 'path'")
    });
    assert!(
        saw_validation_error,
        "expected invalid-parameter error to be injected into next model turn"
    );
}

#[test]
fn contract_stale_context_falls_back_to_full_llm() {
    let router = MessageRouter::new(vec![], std::collections::HashMap::new(), "/".to_string());
    let now = oxicrab::router::now_ms();
    let mut ctx = RouterContext::default();
    ctx.set_active_tool(Some("rss".to_string()));
    ctx.install_directives(vec![ActionDirective {
        trigger: oxicrab::router::context::DirectiveTrigger::Exact("continue".to_string()),
        tool: "rss".to_string(),
        params: serde_json::json!({}),
        single_use: true,
        ttl_ms: 10,
        created_at_ms: now - 50,
    }]);
    ctx.prune_expired(now);

    let decision = router.route("check weather", &ctx, None);
    assert!(
        matches!(decision, RoutingDecision::FullLLM),
        "expected stale context to route FullLLM, got {decision:?}"
    );
}

#[tokio::test]
async fn contract_router_replay_command_handles_empty_history() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let provider = MockLLMProvider::with_responses(vec![text_response("ok")]);
    let agent = create_test_agent_with(provider, &tmp, TestAgentOverrides::default()).await;

    let replay = agent
        .process_direct_with_overrides(
            "ignored",
            "test:replay",
            "telegram",
            "chat1",
            &AgentRunOverrides {
                action: Some(ActionDispatch {
                    tool: "_router_replay".to_string(),
                    params: serde_json::json!({"index": -1}),
                    source: ActionSource::Command {
                        raw: "!router_replay -1".to_string(),
                    },
                }),
                ..Default::default()
            },
        )
        .await
        .expect("router replay")
        .content;

    assert!(
        replay.contains("No router replay traces are available"),
        "expected empty-history replay output, got: {replay}"
    );
}
