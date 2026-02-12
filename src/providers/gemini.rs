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

const CONNECT_TIMEOUT_SECS: u64 = 30;
const REQUEST_TIMEOUT_SECS: u64 = 120;

const BASE_URL: &str = "https://generativelanguage.googleapis.com/v1";

pub struct GeminiProvider {
    api_key: String,
    default_model: String,
    base_url: String,
    client: Client,
    metrics: std::sync::Arc<Mutex<ProviderMetrics>>,
}

impl GeminiProvider {
    pub fn new(api_key: String, default_model: Option<String>) -> Self {
        Self {
            api_key,
            default_model: default_model.unwrap_or_else(|| "gemini-pro".to_string()),
            base_url: BASE_URL.to_string(),
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
            default_model: default_model.unwrap_or_else(|| "gemini-pro".to_string()),
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
        let candidate = json["candidates"]
            .as_array()
            .and_then(|arr| arr.first())
            .context("No candidates in Gemini response")?;

        let content = candidate["content"]["parts"].as_array().and_then(|parts| {
            parts.iter().find_map(|p| {
                if p["text"].is_string() {
                    p["text"].as_str().map(|s| s.to_string())
                } else {
                    None
                }
            })
        });

        let mut tool_calls = Vec::new();
        if let Some(parts) = candidate["content"]["parts"].as_array() {
            for part in parts {
                if let Some(function_calls) = part["functionCalls"].as_array() {
                    for fc in function_calls {
                        tool_calls.push(ToolCallRequest {
                            id: fc["id"].as_str().unwrap_or("").to_string(),
                            name: fc["name"].as_str().unwrap_or("").to_string(),
                            arguments: fc["args"].clone(),
                        });
                    }
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
impl LLMProvider for GeminiProvider {
    async fn chat(&self, req: ChatRequest<'_>) -> Result<LLMResponse> {
        let gemini_contents: Vec<Value> = req
            .messages
            .into_iter()
            .map(|msg| {
                let role = match msg.role.as_str() {
                    "system" => "user", // Gemini doesn't have system role
                    "user" => "user",
                    "assistant" => "model",
                    "tool" => "function",
                    _ => "user",
                };

                let mut parts = vec![json!({"text": msg.content})];
                if msg.role == "user" {
                    for img in &msg.images {
                        parts.push(json!({
                            "inline_data": {
                                "mime_type": img.media_type,
                                "data": img.data
                            }
                        }));
                    }
                }

                json!({
                    "role": role,
                    "parts": parts
                })
            })
            .collect();

        let mut payload = json!({
            "contents": gemini_contents,
            "generationConfig": {
                "maxOutputTokens": req.max_tokens,
                "temperature": req.temperature,
            },
        });

        if let Some(tools) = req.tools {
            payload["tools"] = json!([{
                "functionDeclarations": tools
                    .into_iter()
                    .map(|t| json!({
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters
                    }))
                    .collect::<Vec<_>>()
            }]);
        }

        let model_name = req.model.unwrap_or(&self.default_model);
        let url = format!(
            "{}/models/{}:generateContent?key={}",
            self.base_url, model_name, self.api_key
        );

        let resp = self
            .client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .context("Failed to send request to Gemini API")?;

        let json = ProviderErrorHandler::check_response(resp, "Gemini", &self.metrics).await?;

        // Update metrics on success
        {
            if let Ok(mut metrics) = self.metrics.lock() {
                metrics.request_count += 1;
                if let Some(usage) = json.get("usageMetadata").and_then(|u| u.as_object()) {
                    if let Some(tokens) = usage.get("totalTokenCount").and_then(|t| t.as_u64()) {
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
    use wiremock::matchers::{method, path, query_param};
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
            .and(path("/models/gemini-pro:generateContent"))
            .and(query_param("key", "test_key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "candidates": [{
                    "content": {
                        "parts": [{"text": "Hello! How can I help you?"}],
                        "role": "model"
                    },
                    "finishReason": "STOP"
                }],
                "usageMetadata": {"totalTokenCount": 15}
            })))
            .mount(&server)
            .await;

        let provider = GeminiProvider::with_base_url("test_key".to_string(), None, server.uri());
        let result = provider.chat(simple_chat_request("Hi")).await.unwrap();

        assert_eq!(result.content.unwrap(), "Hello! How can I help you?");
        assert!(result.tool_calls.is_empty());
    }

    #[tokio::test]
    async fn test_chat_with_tool_calls() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/models/gemini-pro:generateContent"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "candidates": [{
                    "content": {
                        "parts": [{
                            "functionCalls": [{
                                "id": "fc_1",
                                "name": "weather",
                                "args": {"city": "London"}
                            }]
                        }],
                        "role": "model"
                    },
                    "finishReason": "STOP"
                }],
                "usageMetadata": {"totalTokenCount": 20}
            })))
            .mount(&server)
            .await;

        let provider = GeminiProvider::with_base_url("test_key".to_string(), None, server.uri());
        let result = provider
            .chat(simple_chat_request("Weather in London?"))
            .await
            .unwrap();

        assert!(result.has_tool_calls());
        assert_eq!(result.tool_calls[0].name, "weather");
        assert_eq!(result.tool_calls[0].arguments["city"], "London");
    }

    #[tokio::test]
    async fn test_chat_unauthorized() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/models/gemini-pro:generateContent"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({
                "error": {"type": "auth_error", "message": "API key not valid"}
            })))
            .mount(&server)
            .await;

        let provider = GeminiProvider::with_base_url("bad_key".to_string(), None, server.uri());
        let result = provider.chat(simple_chat_request("Hi")).await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Authentication"), "Error: {}", err);
    }

    #[tokio::test]
    async fn test_chat_rate_limit() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/models/gemini-pro:generateContent"))
            .respond_with(ResponseTemplate::new(429).set_body_json(json!({
                "error": {"type": "rate_limit", "message": "Quota exceeded"}
            })))
            .mount(&server)
            .await;

        let provider = GeminiProvider::with_base_url("test_key".to_string(), None, server.uri());
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
            .and(path("/models/gemini-pro:generateContent"))
            .respond_with(ResponseTemplate::new(500).set_body_json(json!({
                "error": {"type": "server_error", "message": "Internal error"}
            })))
            .mount(&server)
            .await;

        let provider = GeminiProvider::with_base_url("test_key".to_string(), None, server.uri());
        let result = provider.chat(simple_chat_request("Hi")).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_chat_metrics_updated() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/models/gemini-pro:generateContent"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "candidates": [{
                    "content": {
                        "parts": [{"text": "Hi"}],
                        "role": "model"
                    },
                    "finishReason": "STOP"
                }],
                "usageMetadata": {"totalTokenCount": 12}
            })))
            .mount(&server)
            .await;

        let provider = GeminiProvider::with_base_url("test_key".to_string(), None, server.uri());
        provider.chat(simple_chat_request("Hi")).await.unwrap();

        let metrics = provider.metrics.lock().unwrap();
        assert_eq!(metrics.request_count, 1);
        assert_eq!(metrics.token_count, 12);
    }

    #[tokio::test]
    async fn test_chat_custom_model() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/models/gemini-2.0-flash:generateContent"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "candidates": [{
                    "content": {
                        "parts": [{"text": "Flash response"}],
                        "role": "model"
                    },
                    "finishReason": "STOP"
                }],
                "usageMetadata": {"totalTokenCount": 8}
            })))
            .mount(&server)
            .await;

        let provider = GeminiProvider::with_base_url(
            "test_key".to_string(),
            Some("gemini-2.0-flash".to_string()),
            server.uri(),
        );
        let result = provider.chat(simple_chat_request("Hi")).await.unwrap();

        assert_eq!(result.content.unwrap(), "Flash response");
    }

    #[tokio::test]
    async fn test_system_message_mapped_to_user() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/models/gemini-pro:generateContent"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "candidates": [{
                    "content": {
                        "parts": [{"text": "I'm a helpful bot."}],
                        "role": "model"
                    },
                    "finishReason": "STOP"
                }],
                "usageMetadata": {"totalTokenCount": 10}
            })))
            .mount(&server)
            .await;

        let provider = GeminiProvider::with_base_url("test_key".to_string(), None, server.uri());
        let req = ChatRequest {
            messages: vec![Message::system("You are helpful."), Message::user("Hello")],
            tools: None,
            model: None,
            max_tokens: 1024,
            temperature: 0.7,
            tool_choice: None,
        };
        let result = provider.chat(req).await.unwrap();

        assert_eq!(result.content.unwrap(), "I'm a helpful bot.");
    }
}
