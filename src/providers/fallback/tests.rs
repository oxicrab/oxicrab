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
        cache_creation_input_tokens: None,
        cache_read_input_tokens: None,
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
        cache_creation_input_tokens: None,
        cache_read_input_tokens: None,
    }
}

fn make_request() -> ChatRequest<'static> {
    ChatRequest {
        messages: vec![],
        tools: None,
        model: None,
        max_tokens: 1024,
        temperature: 0.7,
        tool_choice: None,
        response_format: None,
    }
}

#[tokio::test]
async fn test_primary_succeeds_with_valid_response() {
    let primary = MockProvider::ok("local-model", text_response("hello from local"));
    let fallback = MockProvider::ok("cloud-model", text_response("hello from cloud"));

    let provider = FallbackProvider::new(
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

    let provider = FallbackProvider::new(
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
        cache_creation_input_tokens: None,
        cache_read_input_tokens: None,
    };

    let primary = MockProvider::ok("local-model", bad_response);
    let fallback = MockProvider::ok(
        "cloud-model",
        tool_response("web_search", json!({"query": "test"})),
    );

    let provider = FallbackProvider::new(
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
        cache_creation_input_tokens: None,
        cache_read_input_tokens: None,
    };

    let primary = MockProvider::ok("local-model", bad_response);
    let fallback = MockProvider::ok(
        "cloud-model",
        tool_response("web_search", json!({"query": "test"})),
    );

    let provider = FallbackProvider::new(
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

    let provider = FallbackProvider::new(
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

    let provider = FallbackProvider::new(
        primary,
        fallback,
        "local-model".to_string(),
        "cloud-model".to_string(),
    );

    let result = provider.chat(make_request()).await.unwrap();
    assert_eq!(result.content.as_deref(), Some("just text, no tools"));
    assert!(result.tool_calls.is_empty());
}

#[test]
fn test_default_model_returns_primary() {
    let primary = MockProvider::ok("local-model", text_response(""));
    let fallback = MockProvider::ok("cloud-model", text_response(""));

    let provider = FallbackProvider::new(
        primary,
        fallback,
        "local-model".to_string(),
        "cloud-model".to_string(),
    );

    assert_eq!(provider.default_model(), "local-model");
}
