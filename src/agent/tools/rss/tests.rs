use super::*;
use crate::agent::memory::memory_db::rss::{RssArticle, RssFeed};
use crate::agent::tools::base::ExecutionContext;
use std::collections::HashMap;

/// Helper: set up a tool past the `needs_feeds` gate
async fn tool_with_profile() -> (RssTool, ExecutionContext) {
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
    (tool, ctx)
}

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

#[tokio::test]
async fn test_list_feeds_empty() {
    let (tool, ctx) = tool_with_profile().await;
    let result = tool
        .execute(serde_json::json!({"action": "list_feeds"}), &ctx)
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.to_lowercase().contains("no feed"));
}

#[tokio::test]
async fn test_remove_feed_not_found() {
    let (tool, ctx) = tool_with_profile().await;
    let result = tool
        .execute(
            serde_json::json!({
                "action": "remove_feed",
                "feed_id": "nonexistent"
            }),
            &ctx,
        )
        .await
        .unwrap();
    assert!(result.is_error);
}

#[tokio::test]
async fn test_add_feed_invalid_url() {
    let (tool, ctx) = tool_with_profile().await;
    let result = tool
        .execute(
            serde_json::json!({
                "action": "add_feed",
                "url": "not-a-valid-url"
            }),
            &ctx,
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("invalid feed URL"));
}

#[tokio::test]
async fn test_add_feed_ssrf_blocked() {
    let (tool, ctx) = tool_with_profile().await;
    let result = tool
        .execute(
            serde_json::json!({
                "action": "add_feed",
                "url": "http://127.0.0.1/feed.xml"
            }),
            &ctx,
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("invalid feed URL"));
}

// ── Article action helpers ─────────────────────────────────────────────────

fn insert_test_feed_and_articles(
    db: &crate::agent::memory::memory_db::MemoryDB,
) -> (String, Vec<String>) {
    let feed = RssFeed {
        id: "test-feed".into(),
        url: "https://example.com/feed.xml".into(),
        name: "Test Feed".into(),
        site_url: None,
        last_fetched_at_ms: None,
        last_error: None,
        consecutive_failures: 0,
        enabled: true,
        created_at_ms: 1000,
    };
    db.insert_rss_feed(&feed).unwrap();

    let mut article_ids = Vec::new();
    for i in 0..3u64 {
        let art = RssArticle {
            id: format!("art-{i:04}xxxx"),
            feed_id: "test-feed".into(),
            url: format!("https://example.com/post-{i}"),
            title: format!("Article {i}"),
            author: Some("Author".into()),
            published_at_ms: Some(1_700_000_000_000 + i as i64),
            fetched_at_ms: 2000,
            description: Some(format!("Description of article {i}")),
            full_content: None,
            summary: None,
            status: "new".into(),
            read: false,
            created_at_ms: 2000 + i as i64,
        };
        db.insert_rss_article(&art).unwrap();
        article_ids.push(art.id);
    }
    ("test-feed".into(), article_ids)
}

async fn tool_with_calibration_state() -> (RssTool, ExecutionContext) {
    let tool = RssTool::new_for_test();
    let ctx = test_ctx();
    // Set profile to move past needs_profile gate
    tool.execute(
        serde_json::json!({
            "action": "set_profile",
            "interests": "AI engineering, Rust programming, distributed systems"
        }),
        &ctx,
    )
    .await
    .unwrap();
    // Insert feed and articles directly, then set state to needs_calibration
    insert_test_feed_and_articles(&tool.db);
    tool.db
        .set_rss_onboarding_state("needs_calibration", 1000)
        .unwrap();
    (tool, ctx)
}

// ── Article action tests ───────────────────────────────────────────────────

#[tokio::test]
async fn test_get_articles_empty() {
    let empty_tool = RssTool::new_for_test();
    let ctx = test_ctx();
    empty_tool
        .execute(
            serde_json::json!({
                "action": "set_profile",
                "interests": "AI engineering, Rust programming, distributed systems"
            }),
            &ctx,
        )
        .await
        .unwrap();
    // Insert a feed so gate allows get_articles (needs_calibration)
    let feed = RssFeed {
        id: "ef".into(),
        url: "https://example.com/ef.xml".into(),
        name: "Empty Feed".into(),
        site_url: None,
        last_fetched_at_ms: None,
        last_error: None,
        consecutive_failures: 0,
        enabled: true,
        created_at_ms: 1000,
    };
    empty_tool.db.insert_rss_feed(&feed).unwrap();
    empty_tool
        .db
        .set_rss_onboarding_state("needs_calibration", 1000)
        .unwrap();

    let result = empty_tool
        .execute(serde_json::json!({"action": "get_articles"}), &ctx)
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(
        result.content.contains("No articles"),
        "expected 'No articles' message, got: {}",
        result.content
    );
}

#[tokio::test]
async fn test_get_articles_with_status_filter() {
    let (tool, ctx) = tool_with_calibration_state().await;

    // Accept one article directly via DB
    let articles = tool.db.get_rss_articles(None, None, 10, 0).unwrap();
    let first_id = &articles[0].id;
    tool.db
        .update_rss_article_status(first_id, "accepted")
        .unwrap();

    let result = tool
        .execute(
            serde_json::json!({"action": "get_articles", "status": "accepted"}),
            &ctx,
        )
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("accepted"));

    let new_result = tool
        .execute(
            serde_json::json!({"action": "get_articles", "status": "new"}),
            &ctx,
        )
        .await
        .unwrap();
    assert!(!new_result.is_error);
    assert!(new_result.content.contains("new"));
}

#[tokio::test]
async fn test_accept_updates_status() {
    let (tool, ctx) = tool_with_calibration_state().await;

    let articles = tool.db.get_rss_articles(None, None, 10, 0).unwrap();
    let short_id: String = articles[0].id.chars().take(8).collect();

    let result = tool
        .execute(
            serde_json::json!({
                "action": "accept",
                "article_ids": [short_id]
            }),
            &ctx,
        )
        .await
        .unwrap();
    assert!(
        !result.is_error,
        "accept should succeed: {}",
        result.content
    );
    assert!(result.content.contains("accepted"));

    let updated = tool.db.get_rss_article(&articles[0].id).unwrap().unwrap();
    assert_eq!(updated.status, "accepted");
}

#[tokio::test]
async fn test_accept_returns_buttons() {
    let (tool, ctx) = tool_with_calibration_state().await;

    let articles = tool.db.get_rss_articles(None, None, 10, 0).unwrap();
    let short_id: String = articles[0].id.chars().take(8).collect();

    let result = tool
        .execute(
            serde_json::json!({
                "action": "accept",
                "article_ids": [short_id]
            }),
            &ctx,
        )
        .await
        .unwrap();
    assert!(!result.is_error);

    let metadata = result.metadata.expect("metadata should be present");
    let buttons = metadata
        .get("suggested_buttons")
        .expect("suggested_buttons key should exist");
    let arr = buttons
        .as_array()
        .expect("suggested_buttons should be array");
    assert!(!arr.is_empty(), "should have at least one button");
    let ids: Vec<&str> = arr.iter().filter_map(|b| b["id"].as_str()).collect();
    assert!(ids.contains(&"rss-next"), "should have rss-next button");
    assert!(ids.contains(&"rss-done"), "should have rss-done button");
}

#[tokio::test]
async fn test_reject_updates_status() {
    let (tool, ctx) = tool_with_calibration_state().await;

    let articles = tool.db.get_rss_articles(None, None, 10, 0).unwrap();
    let short_id: String = articles[1].id.chars().take(8).collect();

    let result = tool
        .execute(
            serde_json::json!({
                "action": "reject",
                "article_ids": [short_id]
            }),
            &ctx,
        )
        .await
        .unwrap();
    assert!(
        !result.is_error,
        "reject should succeed: {}",
        result.content
    );
    assert!(result.content.contains("rejected"));

    let updated = tool.db.get_rss_article(&articles[1].id).unwrap().unwrap();
    assert_eq!(updated.status, "rejected");
}

#[tokio::test]
async fn test_accept_already_accepted() {
    let (tool, ctx) = tool_with_calibration_state().await;

    let articles = tool.db.get_rss_articles(None, None, 10, 0).unwrap();
    let short_id: String = articles[0].id.chars().take(8).collect();

    // First accept — should succeed
    tool.execute(
        serde_json::json!({
            "action": "accept",
            "article_ids": [short_id]
        }),
        &ctx,
    )
    .await
    .unwrap();

    // Second accept on same article — all IDs failed, so is_error = true
    let result = tool
        .execute(
            serde_json::json!({
                "action": "accept",
                "article_ids": [short_id]
            }),
            &ctx,
        )
        .await
        .unwrap();
    assert!(
        result.is_error,
        "re-accepting should return error: {}",
        result.content
    );
    assert!(
        result.content.contains("already accepted"),
        "expected 'already accepted', got: {}",
        result.content
    );
}

#[tokio::test]
async fn test_accept_empty_ids() {
    let (tool, ctx) = tool_with_calibration_state().await;

    let result = tool
        .execute(serde_json::json!({"action": "accept"}), &ctx)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(
        result.content.contains("no article IDs"),
        "expected no-IDs error, got: {}",
        result.content
    );
}
