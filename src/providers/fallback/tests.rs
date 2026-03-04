use super::*;
use crate::providers::base::{ChatRequest, LLMResponse, ToolCallRequest};
use serde_json::json;

/// A mock provider that returns a pre-configured result.
struct MockProvider {
    model: String,
    response: Result<LLMResponse, String>,
}

impl MockProvider {
    fn ok(model: &str, response: LLMResponse) -> Arc<dyn LLMProvider> {
        Arc::new(Self {
            model: model.to_string(),
            response: Ok(response),
        })
    }

    fn err(model: &str, error: &str) -> Arc<dyn LLMProvider> {
        Arc::new(Self {
            model: model.to_string(),
            response: Err(error.to_string()),
        })
    }
}

#[async_trait]
impl LLMProvider for MockProvider {
    async fn chat(&self, _req: ChatRequest<'_>) -> anyhow::Result<LLMResponse> {
        match &self.response {
            Ok(r) => Ok(r.clone()),
            Err(e) => Err(anyhow::anyhow!("{}", e)),
        }
    }

    fn default_model(&self) -> &str {
        &self.model
    }
}

fn text_response(text: &str) -> LLMResponse {
    LLMResponse {
        content: Some(text.to_string()),
        tool_calls: vec![],
        reasoning_content: None,
        input_tokens: None,
        output_tokens: None,
        ..Default::default()
    }
}

fn tool_response(name: &str, args: serde_json::Value) -> LLMResponse {
    LLMResponse {
        content: None,
        tool_calls: vec![ToolCallRequest {
            id: "tc_1".to_string(),
            name: name.to_string(),
            arguments: args,
        }],
        reasoning_content: None,
        input_tokens: None,
        output_tokens: None,
        ..Default::default()
    }
}

fn make_request() -> ChatRequest<'static> {
    ChatRequest {
        messages: vec![],
        tools: None,
        model: None,
        max_tokens: 1024,
        temperature: Some(0.7),
        tool_choice: None,
        response_format: None,
    }
}

#[tokio::test]
async fn test_primary_succeeds_with_valid_response() {
    let primary = MockProvider::ok("local-model", text_response("hello from local"));
    let fallback = MockProvider::ok("cloud-model", text_response("hello from cloud"));

    let provider = FallbackProvider::pair(
        primary,
        fallback,
        "local-model".to_string(),
        "cloud-model".to_string(),
    );

    let result = provider.chat(make_request()).await.unwrap();
    assert_eq!(result.content.as_deref(), Some("hello from local"));
}

#[tokio::test]
async fn test_primary_fails_falls_back_to_secondary() {
    let primary = MockProvider::err("local-model", "connection refused");
    let fallback = MockProvider::ok("cloud-model", text_response("hello from cloud"));

    let provider = FallbackProvider::pair(
        primary,
        fallback,
        "local-model".to_string(),
        "cloud-model".to_string(),
    );

    let result = provider.chat(make_request()).await.unwrap();
    assert_eq!(result.content.as_deref(), Some("hello from cloud"));
}

#[tokio::test]
async fn test_malformed_tool_calls_fall_back() {
    // Tool call with empty name
    let bad_response = LLMResponse {
        content: None,
        tool_calls: vec![ToolCallRequest {
            id: "tc_1".to_string(),
            name: String::new(),
            arguments: json!({"key": "value"}),
        }],
        reasoning_content: None,
        input_tokens: None,
        output_tokens: None,
        ..Default::default()
    };

    let primary = MockProvider::ok("local-model", bad_response);
    let fallback = MockProvider::ok(
        "cloud-model",
        tool_response("web_search", json!({"query": "test"})),
    );

    let provider = FallbackProvider::pair(
        primary,
        fallback,
        "local-model".to_string(),
        "cloud-model".to_string(),
    );

    let result = provider.chat(make_request()).await.unwrap();
    assert_eq!(result.tool_calls[0].name, "web_search");
}

#[tokio::test]
async fn test_malformed_tool_args_fall_back() {
    // Tool call with non-object arguments
    let bad_response = LLMResponse {
        content: None,
        tool_calls: vec![ToolCallRequest {
            id: "tc_1".to_string(),
            name: "web_search".to_string(),
            arguments: json!("not an object"),
        }],
        reasoning_content: None,
        input_tokens: None,
        output_tokens: None,
        ..Default::default()
    };

    let primary = MockProvider::ok("local-model", bad_response);
    let fallback = MockProvider::ok(
        "cloud-model",
        tool_response("web_search", json!({"query": "test"})),
    );

    let provider = FallbackProvider::pair(
        primary,
        fallback,
        "local-model".to_string(),
        "cloud-model".to_string(),
    );

    let result = provider.chat(make_request()).await.unwrap();
    assert_eq!(result.tool_calls[0].name, "web_search");
}

#[tokio::test]
async fn test_primary_succeeds_with_valid_tool_calls() {
    let primary = MockProvider::ok(
        "local-model",
        tool_response("web_search", json!({"query": "test"})),
    );
    let fallback = MockProvider::ok("cloud-model", text_response("should not reach"));

    let provider = FallbackProvider::pair(
        primary,
        fallback,
        "local-model".to_string(),
        "cloud-model".to_string(),
    );

    let result = provider.chat(make_request()).await.unwrap();
    assert_eq!(result.tool_calls[0].name, "web_search");
}

#[tokio::test]
async fn test_text_only_response_returned_as_is() {
    let primary = MockProvider::ok("local-model", text_response("just text, no tools"));
    let fallback = MockProvider::ok("cloud-model", text_response("should not reach"));

    let provider = FallbackProvider::pair(
        primary,
        fallback,
        "local-model".to_string(),
        "cloud-model".to_string(),
    );

    let result = provider.chat(make_request()).await.unwrap();
    assert_eq!(result.content.as_deref(), Some("just text, no tools"));
    assert!(result.tool_calls.is_empty());
}

#[tokio::test]
async fn test_text_only_with_tools_available_not_rejected() {
    // With tool_choice=None (auto mode), the primary returning text-only when
    // tools are available should NOT trigger a fallback. The model legitimately
    // chose not to use tools (e.g., for a conversational response).
    let primary = MockProvider::ok("local-model", text_response("Sure, I can help with that."));
    let fallback = MockProvider::ok("cloud-model", text_response("should not reach fallback"));

    let provider = FallbackProvider::pair(
        primary,
        fallback,
        "local-model".to_string(),
        "cloud-model".to_string(),
    );

    // Request WITH tools but tool_choice=None (auto)
    let req = ChatRequest {
        messages: vec![],
        tools: Some(vec![crate::providers::base::ToolDefinition {
            name: "web_search".to_string(),
            description: "Search the web".to_string(),
            parameters: json!({"type": "object"}),
        }]),
        model: None,
        max_tokens: 1024,
        temperature: Some(0.7),
        tool_choice: None, // auto mode -- model can choose text
        response_format: None,
    };

    let result = provider.chat(req).await.unwrap();
    // Primary's text response should be returned (not fallback)
    assert_eq!(
        result.content.as_deref(),
        Some("Sure, I can help with that.")
    );
}

#[tokio::test]
async fn test_both_providers_fail_returns_fallback_error() {
    let primary = MockProvider::err("local-model", "connection refused");
    let fallback = MockProvider::err("cloud-model", "API quota exceeded");

    let provider = FallbackProvider::pair(
        primary,
        fallback,
        "local-model".to_string(),
        "cloud-model".to_string(),
    );

    let err = provider.chat(make_request()).await.unwrap_err();
    assert!(
        err.to_string().contains("API quota exceeded"),
        "should return fallback provider's error"
    );
}

#[test]
fn test_default_model_returns_primary() {
    let primary = MockProvider::ok("local-model", text_response(""));
    let fallback = MockProvider::ok("cloud-model", text_response(""));

    let provider = FallbackProvider::pair(
        primary,
        fallback,
        "local-model".to_string(),
        "cloud-model".to_string(),
    );

    assert_eq!(provider.default_model(), "local-model");
}

// --- Vec-based chain tests ---

#[tokio::test]
async fn test_three_provider_chain_skips_to_third() {
    let p1 = MockProvider::err("model-a", "timeout");
    let p2 = MockProvider::err("model-b", "rate limited");
    let p3 = MockProvider::ok("model-c", text_response("hello from third"));

    let provider = FallbackProvider::new(vec![
        (p1, "model-a".to_string()),
        (p2, "model-b".to_string()),
        (p3, "model-c".to_string()),
    ]);

    let result = provider.chat(make_request()).await.unwrap();
    assert_eq!(result.content.as_deref(), Some("hello from third"));
}

#[tokio::test]
async fn test_chain_all_fail_returns_last_error() {
    let p1 = MockProvider::err("model-a", "timeout");
    let p2 = MockProvider::err("model-b", "rate limited");
    let p3 = MockProvider::err("model-c", "quota exceeded");

    let provider = FallbackProvider::new(vec![
        (p1, "model-a".to_string()),
        (p2, "model-b".to_string()),
        (p3, "model-c".to_string()),
    ]);

    let err = provider.chat(make_request()).await.unwrap_err();
    assert!(err.to_string().contains("quota exceeded"));
}

#[tokio::test]
async fn test_chain_malformed_tools_skip_to_next() {
    let bad_response = LLMResponse {
        content: None,
        tool_calls: vec![ToolCallRequest {
            id: "tc_1".to_string(),
            name: String::new(),
            arguments: json!({"key": "value"}),
        }],
        reasoning_content: None,
        input_tokens: None,
        output_tokens: None,
        ..Default::default()
    };

    let p1 = MockProvider::ok("model-a", bad_response);
    let p2 = MockProvider::ok("model-b", text_response("good response from second"));

    let provider = FallbackProvider::new(vec![
        (p1, "model-a".to_string()),
        (p2, "model-b".to_string()),
    ]);

    let result = provider.chat(make_request()).await.unwrap();
    assert_eq!(result.content.as_deref(), Some("good response from second"));
}

#[tokio::test]
async fn test_single_provider_chain() {
    let p1 = MockProvider::ok("model-a", text_response("only provider"));
    let provider = FallbackProvider::new(vec![(p1, "model-a".to_string())]);
    let result = provider.chat(make_request()).await.unwrap();
    assert_eq!(result.content.as_deref(), Some("only provider"));
}
