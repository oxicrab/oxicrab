use super::*;
use crate::providers::base::{ChatRequest, LLMResponse};
use std::collections::HashMap;

struct MockProvider;

#[async_trait::async_trait]
impl LLMProvider for MockProvider {
    async fn chat(&self, _req: &ChatRequest) -> anyhow::Result<LLMResponse> {
        Ok(LLMResponse {
            content: Some("mock summary".to_string()),
            ..LLMResponse::default()
        })
    }

    fn default_model(&self) -> &'static str {
        "mock-model"
    }
}

fn mock_tool() -> WebFetchSummaryTool {
    let provider: Arc<dyn LLMProvider> = Arc::new(MockProvider);
    let fetch = Arc::new(oxicrab_tools_web::web::WebFetchTool::new(50_000).unwrap());
    WebFetchSummaryTool::new(provider, "mock-model".to_string(), fetch)
}

#[test]
fn cache_key_distinguishes_inputs() {
    let a = WebFetchSummaryTool::cache_key("https://x", "p", 600);
    let b = WebFetchSummaryTool::cache_key("https://x", "p", 800);
    let c = WebFetchSummaryTool::cache_key("https://y", "p", 600);
    assert_ne!(a, b);
    assert_ne!(a, c);
}

#[test]
fn truncate_input_caps_at_boundary() {
    let _ = mock_tool();
    let input = "abcdefghij";
    let out = WebFetchSummaryTool::truncate_input(input, 5);
    assert_eq!(out, "abcde…");
    let same = WebFetchSummaryTool::truncate_input(input, 100);
    assert_eq!(same, "abcdefghij");
}

#[test]
fn extract_content_finds_field() {
    let _ = mock_tool();
    let result =
        ToolResult::new(serde_json::json!({"text": "page body", "url": "https://x"}).to_string());
    assert_eq!(
        WebFetchSummaryTool::extract_content(&result),
        Some("page body".to_string())
    );

    let err_result = ToolResult::error("network failed".to_string());
    assert_eq!(WebFetchSummaryTool::extract_content(&err_result), None);
}

#[tokio::test]
async fn cache_hit_short_circuits_provider() {
    let tool = mock_tool();

    let key = WebFetchSummaryTool::cache_key("https://example.com", DEFAULT_PROMPT, 600);
    tool.cache_put(key.clone(), "pre-warmed".to_string());

    let ctx = ExecutionContext {
        channel: "test".to_string(),
        chat_id: "x".to_string(),
        context_summary: None,
        metadata: HashMap::new(),
    };
    let result = tool
        .execute(serde_json::json!({"url": "https://example.com"}), &ctx)
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.starts_with("[cached]"));
    assert!(result.content.contains("pre-warmed"));
}

#[test]
fn cache_expiry_evicts() {
    let tool = mock_tool();
    let key = "test-key".to_string();
    tool.cache_put(key.clone(), "value".to_string());
    // Manually backdate the entry to force expiry.
    {
        let mut cache = tool
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(entry) = cache.get_mut(&key) {
            entry.inserted_at = std::time::Instant::now()
                .checked_sub(std::time::Duration::from_secs(16 * 60))
                .unwrap();
        }
    }
    assert!(tool.cache_get(&key).is_none());
}

#[test]
fn missing_url_returns_error() {
    let tool = mock_tool();
    let ctx = ExecutionContext {
        channel: "test".to_string(),
        chat_id: "x".to_string(),
        context_summary: None,
        metadata: HashMap::new(),
    };
    let result = tokio_test_block_on(tool.execute(serde_json::json!({}), &ctx));
    assert!(result.unwrap().is_error);
}

fn tokio_test_block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(f)
}
