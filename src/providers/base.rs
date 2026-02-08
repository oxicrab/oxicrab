use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
}

impl LLMResponse {
    pub fn has_tool_calls(&self) -> bool {
        !self.tool_calls.is_empty()
    }
}

#[derive(Debug, Clone, Default)]
pub struct Message {
    pub role: String,
    pub content: String,
    pub tool_calls: Option<Vec<ToolCallRequest>>,
    pub tool_call_id: Option<String>,
    /// Whether this tool result represents an error (for role="tool" messages)
    pub is_error: bool,
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
}

#[async_trait]
pub trait LLMProvider: Send + Sync {
    async fn chat(&self, req: ChatRequest<'_>) -> anyhow::Result<LLMResponse>;

    fn default_model(&self) -> &str;

    /// Chat using streaming (SSE). Default implementation falls back to non-streaming.
    /// Implementations collect streamed events and return the assembled response.
    async fn chat_stream(&self, req: ChatRequest<'_>) -> anyhow::Result<LLMResponse> {
        self.chat(req).await
    }

    /// Chat with automatic retry on transient errors
    async fn chat_with_retry(
        &self,
        req: ChatRequest<'_>,
        retry_config: Option<RetryConfig>,
    ) -> anyhow::Result<LLMResponse> {
        let config = retry_config.unwrap_or_default();
        let mut last_error = None;

        // Use Arc to avoid cloning messages and tools on each retry
        use std::sync::Arc;
        let messages_arc = Arc::new(req.messages);
        let tools_arc = req.tools.map(Arc::new);

        for attempt in 0..=config.max_retries {
            match self
                .chat_stream(ChatRequest {
                    messages: (*messages_arc).clone(),
                    tools: tools_arc.as_ref().map(|t| (**t).clone()),
                    model: req.model,
                    max_tokens: req.max_tokens,
                    temperature: req.temperature,
                })
                .await
            {
                Ok(response) => return Ok(response),
                Err(e) => {
                    last_error = Some(e);
                    if attempt < config.max_retries {
                        let delay = (config.initial_delay_ms as f64
                            * config.backoff_multiplier.powi(attempt as i32))
                        .min(config.max_delay_ms as f64) as u64;
                        tokio::time::sleep(tokio::time::Duration::from_millis(delay)).await;
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
    fn message_system() {
        let msg = Message::system("hello");
        assert_eq!(msg.role, "system");
        assert_eq!(msg.content, "hello");
        assert!(msg.tool_calls.is_none());
        assert!(msg.tool_call_id.is_none());
        assert!(!msg.is_error);
    }

    #[test]
    fn message_user() {
        let msg = Message::user("question");
        assert_eq!(msg.role, "user");
        assert_eq!(msg.content, "question");
    }

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
    fn message_tool_result_error() {
        let msg = Message::tool_result("tc2", "error msg", true);
        assert!(msg.is_error);
    }

    #[test]
    fn message_default() {
        let msg = Message::default();
        assert_eq!(msg.role, "");
        assert_eq!(msg.content, "");
        assert!(msg.tool_calls.is_none());
        assert!(msg.tool_call_id.is_none());
        assert!(!msg.is_error);
    }

    #[test]
    fn llm_response_has_tool_calls() {
        let empty = LLMResponse {
            content: Some("hi".into()),
            tool_calls: vec![],
            reasoning_content: None,
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
        };
        assert!(with_tools.has_tool_calls());
    }

    #[test]
    fn retry_config_defaults() {
        let cfg = RetryConfig::default();
        assert_eq!(cfg.max_retries, 3);
        assert_eq!(cfg.initial_delay_ms, 1000);
        assert_eq!(cfg.max_delay_ms, 10000);
        assert!((cfg.backoff_multiplier - 2.0).abs() < f64::EPSILON);
    }
}
