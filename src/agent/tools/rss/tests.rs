use super::*;
use crate::agent::tools::base::ExecutionContext;
use std::collections::HashMap;

fn test_ctx() -> ExecutionContext {
    ExecutionContext {
        channel: "test".to_string(),
        chat_id: "test-chat".to_string(),
        context_summary: None,
        metadata: HashMap::new(),
    }
}

#[tokio::test]
async fn test_onboard_needs_profile() {
    let tool = RssTool::new_for_test();
    let ctx = test_ctx();
    let result = tool
        .execute(serde_json::json!({"action": "onboard"}), &ctx)
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("interest"));
}

#[tokio::test]
async fn test_set_profile_validates_length() {
    let tool = RssTool::new_for_test();
    let ctx = test_ctx();
    let result = tool
        .execute(
            serde_json::json!({"action": "set_profile", "interests": "short"}),
            &ctx,
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("20"));
}

#[tokio::test]
async fn test_set_profile_transitions_state() {
    let tool = RssTool::new_for_test();
    let ctx = test_ctx();
    tool.execute(
        serde_json::json!({
            "action": "set_profile",
            "interests": "AI engineering, Rust programming, distributed systems"
        }),
        &ctx,
    )
    .await
    .unwrap();

    let result = tool
        .execute(serde_json::json!({"action": "onboard"}), &ctx)
        .await
        .unwrap();
    assert!(result.content.to_lowercase().contains("feed"));
}

#[tokio::test]
async fn test_action_gating() {
    let tool = RssTool::new_for_test();
    let ctx = test_ctx();

    // scan should be gated before onboarding
    let result = tool
        .execute(serde_json::json!({"action": "scan"}), &ctx)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(
        result.content.contains("onboarding") || result.content.contains("needs_profile"),
        "expected onboarding/needs_profile mention, got: {}",
        result.content
    );

    // add_feed should be gated in needs_profile state
    let result = tool
        .execute(
            serde_json::json!({"action": "add_feed", "url": "https://example.com/feed.xml"}),
            &ctx,
        )
        .await
        .unwrap();
    assert!(result.is_error);
}

#[test]
fn test_tool_name() {
    let tool = RssTool::new_for_test();
    assert_eq!(tool.name(), "rss");
}

#[test]
fn test_tool_capabilities() {
    let tool = RssTool::new_for_test();
    let caps = tool.capabilities();
    assert!(caps.built_in);
    assert!(caps.network_outbound);
    assert_eq!(caps.category, ToolCategory::Web);
    assert_eq!(caps.actions.len(), 11);
}

#[test]
fn test_execution_timeout() {
    let tool = RssTool::new_for_test();
    assert_eq!(tool.execution_timeout(), Duration::from_mins(5));
}

#[test]
fn test_parameters_schema_has_all_actions() {
    let tool = RssTool::new_for_test();
    let params = tool.parameters();
    let actions = params["properties"]["action"]["enum"]
        .as_array()
        .expect("action enum should exist");
    let action_names: Vec<&str> = actions.iter().filter_map(|v| v.as_str()).collect();
    for expected in [
        "onboard",
        "set_profile",
        "add_feed",
        "remove_feed",
        "list_feeds",
        "scan",
        "get_articles",
        "accept",
        "reject",
        "get_article_detail",
        "feed_stats",
    ] {
        assert!(
            action_names.contains(&expected),
            "missing action: {expected}"
        );
    }
}
