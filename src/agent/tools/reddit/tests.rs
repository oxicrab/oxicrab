use super::*;
use crate::agent::tools::base::ExecutionContext;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn tool() -> RedditTool {
    RedditTool::new()
}

// --- Validation tests ---

#[tokio::test]
async fn test_missing_subreddit() {
    let result = tool()
        .execute(
            serde_json::json!({"action": "hot"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("subreddit"));
}

#[tokio::test]
async fn test_unknown_action() {
    let result = tool()
        .execute(
            serde_json::json!({"subreddit": "rust", "action": "bogus"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("unknown action"));
}

#[tokio::test]
async fn test_search_missing_query() {
    let result = tool()
        .execute(
            serde_json::json!({"subreddit": "rust", "action": "search"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("query"));
}

#[tokio::test]
async fn test_invalid_subreddit_path_traversal() {
    let result = tool()
        .execute(
            serde_json::json!({"subreddit": "../../api/v1", "action": "hot"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("invalid subreddit"));
}

#[tokio::test]
async fn test_invalid_subreddit_with_slash() {
    let result = tool()
        .execute(
            serde_json::json!({"subreddit": "foo/bar", "action": "hot"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("invalid subreddit"));
}

#[test]
fn test_is_valid_subreddit() {
    assert!(is_valid_subreddit("rust"));
    assert!(is_valid_subreddit("Ask_Reddit"));
    assert!(is_valid_subreddit("programming"));
    assert!(!is_valid_subreddit(""));
    assert!(!is_valid_subreddit("foo/bar"));
    assert!(!is_valid_subreddit("../../etc"));
    assert!(!is_valid_subreddit("a".repeat(22).as_str()));
    assert!(!is_valid_subreddit("has spaces"));
    assert!(!is_valid_subreddit("has-dashes"));
}

// --- Wiremock tests ---

fn reddit_listing(posts: Vec<serde_json::Value>) -> serde_json::Value {
    serde_json::json!({
        "data": {
            "children": posts.into_iter().map(|p| serde_json::json!({"data": p})).collect::<Vec<_>>()
        }
    })
}

#[tokio::test]
async fn test_hot_posts_success() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/r/rust/hot.json"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(reddit_listing(vec![serde_json::json!({
                "title": "Rust 2026 edition released!",
                "score": 1500,
                "num_comments": 200,
                "author": "rustacean",
                "url": "https://blog.rust-lang.org/2026",
                "selftext": ""
            })])),
        )
        .mount(&server)
        .await;

    let tool = RedditTool::with_base_url(server.uri());
    let result = tool
        .execute(
            serde_json::json!({"subreddit": "rust", "action": "hot"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("Rust 2026 edition released!"));
    assert!(result.content.contains("score: 1500"));
    assert!(result.content.contains("u/rustacean"));
}

#[tokio::test]
async fn test_top_posts_with_selftext() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/r/programming/top.json"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(reddit_listing(vec![serde_json::json!({
                "title": "Best practices for async Rust",
                "score": 500,
                "num_comments": 80,
                "author": "dev123",
                "url": "https://reddit.com/r/programming/...",
                "selftext": "Here are my tips for writing async Rust code effectively..."
            })])),
        )
        .mount(&server)
        .await;

    let tool = RedditTool::with_base_url(server.uri());
    let result = tool
        .execute(
            serde_json::json!({"subreddit": "programming", "action": "top", "time": "week"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("Best practices"));
    assert!(result.content.contains("async Rust code"));
}

#[tokio::test]
async fn test_empty_subreddit() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/r/emptysub/hot.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(reddit_listing(vec![])))
        .mount(&server)
        .await;

    let tool = RedditTool::with_base_url(server.uri());
    let result = tool
        .execute(
            serde_json::json!({"subreddit": "emptysub"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("No posts found"));
}

#[tokio::test]
async fn test_subreddit_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/r/nonexistent/hot.json"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let tool = RedditTool::with_base_url(server.uri());
    let result = tool
        .execute(
            serde_json::json!({"subreddit": "nonexistent"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result.content.contains("not found"));
}

#[tokio::test]
async fn test_subreddit_private() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/r/secret/hot.json"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;

    let tool = RedditTool::with_base_url(server.uri());
    let result = tool
        .execute(
            serde_json::json!({"subreddit": "secret"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result.content.contains("private or quarantined"));
}

#[tokio::test]
async fn test_search_success() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/r/rust/search.json"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(reddit_listing(vec![serde_json::json!({
                "title": "Tokio tutorial",
                "score": 300,
                "num_comments": 45,
                "author": "async_fan",
                "url": "https://tokio.rs/tutorial"
            })])),
        )
        .mount(&server)
        .await;

    let tool = RedditTool::with_base_url(server.uri());
    let result = tool
        .execute(
            serde_json::json!({"subreddit": "rust", "action": "search", "query": "tokio"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("Tokio tutorial"));
}

#[tokio::test]
async fn test_search_no_results() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/r/rust/search.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(reddit_listing(vec![])))
        .mount(&server)
        .await;

    let tool = RedditTool::with_base_url(server.uri());
    let result = tool
        .execute(
            serde_json::json!({"subreddit": "rust", "action": "search", "query": "xyznotfound"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("No results"));
}

#[tokio::test]
async fn test_new_posts_action() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/r/rust/new.json"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(reddit_listing(vec![serde_json::json!({
                "title": "Just published my first crate",
                "score": 10,
                "num_comments": 3,
                "author": "newbie",
                "url": "https://crates.io/crates/...",
                "selftext": ""
            })])),
        )
        .mount(&server)
        .await;

    let tool = RedditTool::with_base_url(server.uri());
    let result = tool
        .execute(
            serde_json::json!({"subreddit": "rust", "action": "new"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("first crate"));
}

#[test]
fn test_reddit_capabilities() {
    use crate::agent::tools::base::SubagentAccess;
    let tool = RedditTool::new();
    let caps = tool.capabilities();
    assert!(caps.built_in);
    assert!(caps.network_outbound);
    assert_eq!(caps.subagent_access, SubagentAccess::ReadOnly);
    assert_eq!(caps.actions.len(), 4);
    assert!(caps.actions.iter().all(|a| a.read_only));
}
