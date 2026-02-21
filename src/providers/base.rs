use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{debug, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRequest {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone)]
pub struct LLMResponse {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCallRequest>,
    pub reasoning_content: Option<String>,
    /// Input token count reported by the provider (if available).
    /// Used for precise compaction threshold checks.
    pub input_tokens: Option<u64>,
    /// Output token count reported by the provider (if available).
    /// Used for cost tracking.
    pub output_tokens: Option<u64>,
    /// Anthropic prompt caching: tokens written to cache (billed at 125% of input rate).
    pub cache_creation_input_tokens: Option<u64>,
    /// Anthropic prompt caching: tokens read from cache (billed at 10% of input rate).
    pub cache_read_input_tokens: Option<u64>,
}

impl LLMResponse {
    pub fn has_tool_calls(&self) -> bool {
        !self.tool_calls.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct ImageData {
    pub media_type: String, // "image/jpeg", "image/png", etc.
    pub data: String,       // base64-encoded
}

#[derive(Debug, Clone, Default)]
pub struct Message {
    pub role: String,
    pub content: String,
    pub tool_calls: Option<Vec<ToolCallRequest>>,
    pub tool_call_id: Option<String>,
    /// Whether this tool result represents an error (for role="tool" messages)
    pub is_error: bool,
    /// Base64-encoded images attached to this message
    pub images: Vec<ImageData>,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: content.into(),
            ..Default::default()
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: content.into(),
            ..Default::default()
        }
    }

    pub fn user_with_images(content: impl Into<String>, images: Vec<ImageData>) -> Self {
        Self {
            role: "user".into(),
            content: content.into(),
            images,
            ..Default::default()
        }
    }

    pub fn assistant(content: impl Into<String>, tool_calls: Option<Vec<ToolCallRequest>>) -> Self {
        Self {
            role: "assistant".into(),
            content: content.into(),
            tool_calls,
            ..Default::default()
        }
    }

    pub fn tool_result(
        tool_call_id: impl Into<String>,
        content: impl Into<String>,
        is_error: bool,
    ) -> Self {
        Self {
            role: "tool".into(),
            content: content.into(),
            tool_call_id: Some(tool_call_id.into()),
            is_error,
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value, // JSON Schema
}

/// Metrics for provider operations
#[derive(Debug, Clone, Default)]
pub struct ProviderMetrics {
    pub request_count: u64,
    pub token_count: u64,
    pub error_count: u64,
}

/// Configuration for retry behavior
#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub max_retries: usize,
    pub initial_delay_ms: u64,
    pub max_delay_ms: u64,
    pub backoff_multiplier: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_delay_ms: 1000,
            max_delay_ms: 10000,
            backoff_multiplier: 2.0,
        }
    }
}

/// Parameters for a chat request to an LLM provider.
#[derive(Debug, Clone)]
pub struct ChatRequest<'a> {
    pub messages: Vec<Message>,
    pub tools: Option<Vec<ToolDefinition>>,
    pub model: Option<&'a str>,
    pub max_tokens: u32,
    pub temperature: f32,
    /// Tool choice mode: "auto" (default), "any" (force tool use), or "none".
    pub tool_choice: Option<String>,
}

#[async_trait]
pub trait LLMProvider: Send + Sync {
    async fn chat(&self, req: ChatRequest<'_>) -> anyhow::Result<LLMResponse>;

    fn default_model(&self) -> &str;

    /// Pre-warm the provider's HTTP connection (TLS handshake, HTTP/2 negotiation).
    /// Default is a no-op. Providers may override to make a lightweight request.
    async fn warmup(&self) -> anyhow::Result<()> {
        Ok(())
    }

    /// Return accumulated provider metrics (requests, tokens, errors).
    /// Default returns zeroed metrics for providers that don't track them.
    fn metrics(&self) -> ProviderMetrics {
        ProviderMetrics::default()
    }

    /// Chat with automatic retry on transient errors.
    async fn chat_with_retry(
        &self,
        req: ChatRequest<'_>,
        retry_config: Option<RetryConfig>,
    ) -> anyhow::Result<LLMResponse> {
        let config = retry_config.unwrap_or_default();
        let mut last_error = None;

        let messages = req.messages;
        let tools = req.tools;

        for attempt in 0..=config.max_retries {
            if attempt > 0 {
                warn!(
                    "Provider retry attempt {}/{} after error: {}",
                    attempt,
                    config.max_retries,
                    last_error
                        .as_ref()
                        .map(|e: &anyhow::Error| e.to_string())
                        .unwrap_or_default()
                );
            }
            debug!("Sending chat request (attempt {})", attempt);
            let chat_req = ChatRequest {
                messages: messages.clone(),
                tools: tools.clone(),
                model: req.model,
                max_tokens: req.max_tokens,
                temperature: req.temperature,
                tool_choice: req.tool_choice.clone(),
            };
            let result = self.chat(chat_req).await;
            match result {
                Ok(response) => {
                    debug!("Chat request succeeded on attempt {}", attempt);
                    return Ok(response);
                }
                Err(e) => {
                    // Don't retry non-transient errors
                    let is_transient =
                        e.downcast_ref::<crate::errors::OxicrabError>()
                            .is_none_or(|ox| match ox {
                                crate::errors::OxicrabError::Provider { retryable, .. } => {
                                    *retryable
                                }
                                crate::errors::OxicrabError::Auth(_)
                                | crate::errors::OxicrabError::Config(_)
                                | crate::errors::OxicrabError::RateLimit { .. } => false,
                                crate::errors::OxicrabError::Internal(_) => true,
                            });
                    warn!("Chat request failed on attempt {}: {}", attempt, e);
                    if !is_transient {
                        return Err(e);
                    }
                    last_error = Some(e);
                    if attempt < config.max_retries {
                        let delay = (config.initial_delay_ms as f64
                            * config.backoff_multiplier.powi(attempt as i32))
                        .min(config.max_delay_ms as f64) as u64;
                        // Add jitter (up to 25% of delay) to avoid thundering herd
                        let jitter = (delay as f64 * 0.25 * fastrand::f64()) as u64;
                        let total_delay = delay + jitter;
                        debug!(
                            "Waiting {}ms before retry ({}ms base + {}ms jitter)",
                            total_delay, delay, jitter
                        );
                        tokio::time::sleep(tokio::time::Duration::from_millis(total_delay)).await;
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("All retry attempts failed")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_assistant_with_tool_calls() {
        let tc = vec![ToolCallRequest {
            id: "tc1".into(),
            name: "weather".into(),
            arguments: serde_json::json!({"city": "NYC"}),
        }];
        let msg = Message::assistant("thinking", Some(tc));
        assert_eq!(msg.role, "assistant");
        assert_eq!(msg.content, "thinking");
        assert_eq!(msg.tool_calls.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn message_tool_result() {
        let msg = Message::tool_result("tc1", "result data", false);
        assert_eq!(msg.role, "tool");
        assert_eq!(msg.content, "result data");
        assert_eq!(msg.tool_call_id.as_deref(), Some("tc1"));
        assert!(!msg.is_error);
    }

    #[test]
    fn llm_response_has_tool_calls() {
        let empty = LLMResponse {
            content: Some("hi".into()),
            tool_calls: vec![],
            reasoning_content: None,
            input_tokens: None,
            output_tokens: None,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
        };
        assert!(!empty.has_tool_calls());

        let with_tools = LLMResponse {
            content: None,
            tool_calls: vec![ToolCallRequest {
                id: "1".into(),
                name: "test".into(),
                arguments: Value::Null,
            }],
            reasoning_content: None,
            input_tokens: None,
            output_tokens: None,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
        };
        assert!(with_tools.has_tool_calls());
    }

    struct NoopProvider;

    #[async_trait]
    impl LLMProvider for NoopProvider {
        async fn chat(&self, _req: ChatRequest<'_>) -> anyhow::Result<LLMResponse> {
            Ok(LLMResponse {
                content: Some("ok".into()),
                tool_calls: vec![],
                reasoning_content: None,
                input_tokens: None,
                output_tokens: None,
                cache_creation_input_tokens: None,
                cache_read_input_tokens: None,
            })
        }
        fn default_model(&self) -> &'static str {
            "noop"
        }
    }

    #[tokio::test]
    async fn default_warmup_returns_ok() {
        let provider = NoopProvider;
        assert!(provider.warmup().await.is_ok());
    }
}
