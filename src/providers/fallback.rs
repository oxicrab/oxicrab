use crate::providers::base::{ChatRequest, LLMProvider, LLMResponse, ToolCallRequest};
use async_trait::async_trait;
use std::sync::Arc;
use tracing::warn;

/// An LLM provider that tries a primary (local) provider first,
/// falling back to a secondary (cloud) provider on error or malformed tool calls.
pub struct FallbackProvider {
    primary: Arc<dyn LLMProvider>,
    fallback: Arc<dyn LLMProvider>,
    primary_model: String,
    fallback_model: String,
}

impl FallbackProvider {
    pub fn new(
        primary: Arc<dyn LLMProvider>,
        fallback: Arc<dyn LLMProvider>,
        primary_model: String,
        fallback_model: String,
    ) -> Self {
        Self {
            primary,
            fallback,
            primary_model,
            fallback_model,
        }
    }
}

/// Validate that all tool calls in a response have well-formed names and arguments.
fn validate_tool_calls(tool_calls: &[ToolCallRequest]) -> bool {
    for tc in tool_calls {
        if tc.name.is_empty() {
            return false;
        }
        if !tc.arguments.is_object() {
            return false;
        }
    }
    true
}

#[async_trait]
impl LLMProvider for FallbackProvider {
    async fn chat(&self, req: ChatRequest<'_>) -> anyhow::Result<LLMResponse> {
        // Build a request for the primary (local) model.
        // Use model: None so the provider uses its own default_model() (already stripped of prefix).
        let primary_req = ChatRequest {
            messages: req.messages.clone(),
            tools: req.tools.clone(),
            model: None,
            max_tokens: req.max_tokens,
            temperature: req.temperature,
            tool_choice: req.tool_choice.clone(),
        };

        match self.primary.chat(primary_req).await {
            Ok(response) => {
                // If there are tool calls, validate them
                if response.has_tool_calls() && !validate_tool_calls(&response.tool_calls) {
                    warn!(
                        "primary provider ({}) returned malformed tool calls, falling back to {}",
                        self.primary_model, self.fallback_model
                    );
                } else {
                    return Ok(response);
                }
            }
            Err(e) => {
                warn!(
                    "primary provider ({}) failed: {}, falling back to {}",
                    self.primary_model, e, self.fallback_model
                );
            }
        }

        // Fall back to cloud provider — also use None to let it pick its own default
        let fallback_req = ChatRequest {
            messages: req.messages,
            tools: req.tools,
            model: None,
            max_tokens: req.max_tokens,
            temperature: req.temperature,
            tool_choice: req.tool_choice,
        };

        self.fallback.chat(fallback_req).await
    }

    fn default_model(&self) -> &str {
        &self.primary_model
    }
}

#[cfg(test)]
mod tests {
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
}
