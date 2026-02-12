use crate::providers::base::{
    ChatRequest, LLMProvider, LLMResponse, ProviderMetrics, ToolCallRequest,
};
use crate::providers::errors::ProviderErrorHandler;
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};
use std::sync::Mutex;
use std::time::Duration;

const API_URL: &str = "https://api.openai.com/v1/chat/completions";
const CONNECT_TIMEOUT_SECS: u64 = 30;
const REQUEST_TIMEOUT_SECS: u64 = 120;

pub struct OpenAIProvider {
    api_key: String,
    default_model: String,
    base_url: String,
    client: Client,
    metrics: std::sync::Arc<Mutex<ProviderMetrics>>,
}

impl OpenAIProvider {
    pub fn new(api_key: String, default_model: Option<String>) -> Self {
        Self {
            api_key,
            default_model: default_model.unwrap_or_else(|| "gpt-4o".to_string()),
            base_url: API_URL.to_string(),
            client: Client::builder()
                .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
                .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
                .build()
                .unwrap_or_else(|_| Client::new()),
            metrics: std::sync::Arc::new(Mutex::new(ProviderMetrics::default())),
        }
    }

    #[cfg(test)]
    fn with_base_url(api_key: String, default_model: Option<String>, base_url: String) -> Self {
        Self {
            api_key,
            default_model: default_model.unwrap_or_else(|| "gpt-4o".to_string()),
            base_url,
            client: Client::builder()
                .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
                .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
                .build()
                .unwrap_or_else(|_| Client::new()),
            metrics: std::sync::Arc::new(Mutex::new(ProviderMetrics::default())),
        }
    }

    fn parse_response(&self, json: Value) -> Result<LLMResponse> {
        let choice = json["choices"]
            .as_array()
            .and_then(|arr| arr.first())
            .context("No choices in OpenAI response")?;

        let message = &choice["message"];
        let content = message["content"].as_str().map(|s| s.to_string());

        let mut tool_calls = Vec::new();
        if let Some(tool_calls_array) = message["tool_calls"].as_array() {
            for tc in tool_calls_array {
                if let Some(function) = tc["function"].as_object() {
                    let arguments = function["arguments"]
                        .as_str()
                        .and_then(|s| serde_json::from_str(s).ok())
                        .unwrap_or_else(|| json!({}));

                    tool_calls.push(ToolCallRequest {
                        id: tc["id"].as_str().unwrap_or("").to_string(),
                        name: function["name"].as_str().unwrap_or("").to_string(),
                        arguments,
                    });
                }
            }
        }

        Ok(LLMResponse {
            content,
            tool_calls,
            reasoning_content: None,
            input_tokens: None,
        })
    }
}

#[async_trait]
impl LLMProvider for OpenAIProvider {
    async fn chat(&self, req: ChatRequest<'_>) -> Result<LLMResponse> {
        let openai_messages: Vec<Value> = req
            .messages
            .into_iter()
            .map(|msg| {
                let content_value = if !msg.images.is_empty() && msg.role == "user" {
                    let mut parts = Vec::new();
                    if !msg.content.is_empty() {
                        parts.push(json!({
                            "type": "text",
                            "text": msg.content
                        }));
                    }
                    for img in &msg.images {
                        parts.push(json!({
                            "type": "image_url",
                            "image_url": {
                                "url": format!("data:{};base64,{}", img.media_type, img.data)
                            }
                        }));
                    }
                    json!(parts)
                } else {
                    json!(msg.content)
                };
                let mut m = json!({
                    "role": msg.role,
                    "content": content_value,
                });

                if let Some(tool_calls) = msg.tool_calls {
                    m["tool_calls"] = json!(tool_calls
                        .into_iter()
                        .map(|tc| json!({
                            "id": tc.id,
                            "type": "function",
                            "function": {
                                "name": tc.name,
                                "arguments": tc.arguments
                            }
                        }))
                        .collect::<Vec<_>>());
                }

                if let Some(tool_call_id) = msg.tool_call_id {
                    m["tool_call_id"] = json!(tool_call_id);
                }

                m
            })
            .collect();

        let mut payload = json!({
            "model": req.model.unwrap_or(&self.default_model),
            "messages": openai_messages,
            "max_tokens": req.max_tokens,
            "temperature": req.temperature,
        });

        if let Some(tools) = req.tools {
            payload["tools"] = json!(tools
                .into_iter()
                .map(|t| json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters
                    }
                }))
                .collect::<Vec<_>>());
            if let Some(ref choice) = req.tool_choice {
                payload["tool_choice"] = json!(choice);
            }
        }

        let resp = self
            .client
            .post(&self.base_url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .context("Failed to send request to OpenAI API")?;

        let json = ProviderErrorHandler::check_response(resp, "OpenAI", &self.metrics).await?;

        // Update metrics on success
        {
            if let Ok(mut metrics) = self.metrics.lock() {
                metrics.request_count += 1;
                if let Some(usage) = json.get("usage").and_then(|u| u.as_object()) {
                    if let Some(tokens) = usage.get("total_tokens").and_then(|t| t.as_u64()) {
                        metrics.token_count += tokens;
                    }
                }
            }
        }

        self.parse_response(json)
    }

    fn default_model(&self) -> &str {
        &self.default_model
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::base::Message;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // --- Wiremock tests ---

    fn simple_chat_request(content: &str) -> ChatRequest<'_> {
        ChatRequest {
            messages: vec![Message::user(content)],
            tools: None,
            model: None,
            max_tokens: 1024,
            temperature: 0.7,
            tool_choice: None,
        }
    }

    #[tokio::test]
    async fn test_chat_success() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/"))
            .and(header("Authorization", "Bearer test_key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": "Hello! How can I help?"
                    },
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 10, "completion_tokens": 8, "total_tokens": 18}
            })))
            .mount(&server)
            .await;

        let provider = OpenAIProvider::with_base_url("test_key".to_string(), None, server.uri());
        let result = provider.chat(simple_chat_request("Hi")).await.unwrap();

        assert_eq!(result.content.unwrap(), "Hello! How can I help?");
        assert!(result.tool_calls.is_empty());
    }

    #[tokio::test]
    async fn test_chat_with_tool_calls() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [{
                            "id": "call_123",
                            "type": "function",
                            "function": {
                                "name": "weather",
                                "arguments": "{\"city\": \"NYC\"}"
                            }
                        }]
                    },
                    "finish_reason": "tool_calls"
                }],
                "usage": {"prompt_tokens": 15, "completion_tokens": 20, "total_tokens": 35}
            })))
            .mount(&server)
            .await;

        let provider = OpenAIProvider::with_base_url("test_key".to_string(), None, server.uri());
        let result = provider
            .chat(simple_chat_request("What's the weather?"))
            .await
            .unwrap();

        assert!(result.has_tool_calls());
        assert_eq!(result.tool_calls[0].name, "weather");
        assert_eq!(result.tool_calls[0].id, "call_123");
        assert_eq!(result.tool_calls[0].arguments["city"], "NYC");
    }

    #[tokio::test]
    async fn test_chat_unauthorized() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({
                "error": {"type": "authentication_error", "message": "Invalid API key"}
            })))
            .mount(&server)
            .await;

        let provider = OpenAIProvider::with_base_url("bad_key".to_string(), None, server.uri());
        let result = provider.chat(simple_chat_request("Hi")).await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Authentication"), "Error: {}", err);
    }

    #[tokio::test]
    async fn test_chat_rate_limit() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/"))
            .respond_with(
                ResponseTemplate::new(429)
                    .insert_header("retry-after", "60")
                    .set_body_json(json!({
                        "error": {"type": "rate_limit", "message": "Too many requests"}
                    })),
            )
            .mount(&server)
            .await;

        let provider = OpenAIProvider::with_base_url("test_key".to_string(), None, server.uri());
        let result = provider.chat(simple_chat_request("Hi")).await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Rate limit") || err.contains("rate limit"),
            "Error: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_chat_server_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(500).set_body_json(json!({
                "error": {"type": "server_error", "message": "Internal server error"}
            })))
            .mount(&server)
            .await;

        let provider = OpenAIProvider::with_base_url("test_key".to_string(), None, server.uri());
        let result = provider.chat(simple_chat_request("Hi")).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_chat_metrics_updated() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{
                    "message": {"role": "assistant", "content": "Hi"},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7}
            })))
            .mount(&server)
            .await;

        let provider = OpenAIProvider::with_base_url("test_key".to_string(), None, server.uri());
        provider.chat(simple_chat_request("Hi")).await.unwrap();

        let metrics = provider.metrics.lock().unwrap();
        assert_eq!(metrics.request_count, 1);
        assert_eq!(metrics.token_count, 7);
    }

    #[tokio::test]
    async fn test_chat_custom_model() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{
                    "message": {"role": "assistant", "content": "Response from custom model"},
                    "finish_reason": "stop"
                }],
                "usage": {"total_tokens": 10}
            })))
            .mount(&server)
            .await;

        let provider = OpenAIProvider::with_base_url(
            "test_key".to_string(),
            Some("gpt-4-turbo".to_string()),
            server.uri(),
        );
        let result = provider.chat(simple_chat_request("Hi")).await.unwrap();

        assert_eq!(result.content.unwrap(), "Response from custom model");
    }
}
