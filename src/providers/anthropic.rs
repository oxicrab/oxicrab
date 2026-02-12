use crate::providers::anthropic_common;
use crate::providers::base::{
    ChatRequest, LLMProvider, LLMResponse, ProviderMetrics, StreamCallback, ToolCallRequest,
};
use crate::providers::errors::ProviderErrorHandler;
use crate::providers::sse::parse_sse_chunk;
use anyhow::{Context, Result};
use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const CONNECT_TIMEOUT_SECS: u64 = 30;
const STREAM_CHUNK_TIMEOUT_SECS: u64 = 120;

pub struct AnthropicProvider {
    api_key: String,
    default_model: String,
    base_url: String,
    client: Client,
    metrics: std::sync::Arc<std::sync::Mutex<ProviderMetrics>>,
}

impl AnthropicProvider {
    pub fn new(api_key: String, default_model: Option<String>) -> Self {
        Self {
            api_key,
            default_model: default_model
                .unwrap_or_else(|| "claude-sonnet-4-5-20250929".to_string()),
            base_url: API_URL.to_string(),
            client: Client::builder()
                .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
                .build()
                .unwrap_or_else(|_| Client::new()),
            metrics: std::sync::Arc::new(std::sync::Mutex::new(ProviderMetrics::default())),
        }
    }

    #[cfg(test)]
    fn with_base_url(api_key: String, default_model: Option<String>, base_url: String) -> Self {
        Self {
            api_key,
            default_model: default_model
                .unwrap_or_else(|| "claude-sonnet-4-5-20250929".to_string()),
            base_url,
            client: Client::builder()
                .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
                .build()
                .unwrap_or_else(|_| Client::new()),
            metrics: std::sync::Arc::new(std::sync::Mutex::new(ProviderMetrics::default())),
        }
    }
}

#[async_trait]
impl LLMProvider for AnthropicProvider {
    async fn chat(&self, req: ChatRequest<'_>) -> Result<LLMResponse> {
        let (system, anthropic_messages) = anthropic_common::convert_messages(req.messages);

        let mut payload = json!({
            "model": req.model.unwrap_or(&self.default_model),
            "messages": anthropic_messages,
            "max_tokens": req.max_tokens,
            "temperature": req.temperature,
        });

        if let Some(system) = system {
            payload["system"] = json!(system);
        }

        if let Some(tools) = req.tools {
            payload["tools"] = json!(anthropic_common::convert_tools(tools));
            let choice = req.tool_choice.as_deref().unwrap_or("auto");
            payload["tool_choice"] = json!({"type": choice});
        }

        let resp = self
            .client
            .post(&self.base_url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&payload)
            .timeout(Duration::from_secs(STREAM_CHUNK_TIMEOUT_SECS))
            .send()
            .await
            .context("Failed to send request to Anthropic API")?;

        let json = ProviderErrorHandler::check_response(resp, "Anthropic", &self.metrics).await?;

        // Update metrics on success
        {
            if let Ok(mut metrics) = self.metrics.lock() {
                metrics.request_count += 1;
                if let Some(usage) = json.get("usage").and_then(|u| u.as_object()) {
                    if let Some(tokens) = usage.get("input_tokens").and_then(|t| t.as_u64()) {
                        metrics.token_count += tokens;
                    }
                    if let Some(tokens) = usage.get("output_tokens").and_then(|t| t.as_u64()) {
                        metrics.token_count += tokens;
                    }
                }
            }
        }

        Ok(anthropic_common::parse_response(&json))
    }

    async fn chat_stream(&self, req: ChatRequest<'_>) -> Result<LLMResponse> {
        self.chat_stream_with_callback(req, None).await
    }

    async fn chat_stream_with_callback(
        &self,
        req: ChatRequest<'_>,
        callback: Option<StreamCallback>,
    ) -> Result<LLMResponse> {
        let (system, anthropic_messages) = anthropic_common::convert_messages(req.messages);

        let mut payload = json!({
            "model": req.model.unwrap_or(&self.default_model),
            "messages": anthropic_messages,
            "max_tokens": req.max_tokens,
            "temperature": req.temperature,
            "stream": true,
        });

        if let Some(system) = system {
            payload["system"] = json!(system);
        }

        if let Some(tools) = req.tools {
            payload["tools"] = json!(anthropic_common::convert_tools(tools));
            let choice = req.tool_choice.as_deref().unwrap_or("auto");
            payload["tool_choice"] = json!({"type": choice});
        }

        let resp = self
            .client
            .post(&self.base_url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&payload)
            .send()
            .await
            .context("Failed to send streaming request to Anthropic API")?;

        let resp = ProviderErrorHandler::check_http_status(resp, "Anthropic").await?;

        // Process SSE stream
        let mut content_text = String::new();
        let mut tool_calls: Vec<ToolCallRequest> = Vec::new();
        let mut current_tool_id = String::new();
        let mut current_tool_name = String::new();
        let mut current_tool_json = String::new();
        let mut buf = String::new();
        let mut stream_input_tokens: Option<u64> = None;

        let mut stream = resp.bytes_stream();
        loop {
            let chunk = tokio::time::timeout(
                Duration::from_secs(STREAM_CHUNK_TIMEOUT_SECS),
                stream.next(),
            )
            .await
            .context("Anthropic stream timed out waiting for next chunk")?;

            let Some(chunk) = chunk else { break };
            let chunk = chunk.context("Error reading stream chunk")?;
            let text = String::from_utf8_lossy(&chunk);
            buf.push_str(&text);

            // Process complete SSE events from buffer
            let events = parse_sse_chunk(&buf);
            // Keep any incomplete event data (last line without trailing \n\n)
            if let Some(last_double_newline) = buf.rfind("\n\n") {
                buf = buf[last_double_newline + 2..].to_string();
            }

            for event in events {
                let Some(data) = event.data else { continue };
                let event_type = data["type"].as_str().unwrap_or("");

                match event_type {
                    "content_block_start" => {
                        let block = &data["content_block"];
                        if block["type"].as_str() == Some("tool_use") {
                            current_tool_id = block["id"].as_str().unwrap_or("").to_string();
                            current_tool_name = block["name"].as_str().unwrap_or("").to_string();
                            current_tool_json.clear();
                        }
                    }
                    "content_block_delta" => {
                        let delta = &data["delta"];
                        match delta["type"].as_str() {
                            Some("text_delta") => {
                                if let Some(text) = delta["text"].as_str() {
                                    content_text.push_str(text);
                                    if let Some(ref cb) = callback {
                                        cb(text);
                                    }
                                }
                            }
                            Some("input_json_delta") => {
                                if let Some(json_str) = delta["partial_json"].as_str() {
                                    current_tool_json.push_str(json_str);
                                }
                            }
                            _ => {}
                        }
                    }
                    "content_block_stop" => {
                        if !current_tool_id.is_empty() {
                            let arguments: Value =
                                serde_json::from_str(&current_tool_json).unwrap_or(Value::Null);
                            tool_calls.push(ToolCallRequest {
                                id: current_tool_id.clone(),
                                name: current_tool_name.clone(),
                                arguments,
                            });
                            current_tool_id.clear();
                            current_tool_name.clear();
                            current_tool_json.clear();
                        }
                    }
                    "message_delta" => {
                        // Could extract stop_reason and usage here if needed
                        if let Some(usage) = data.get("usage").and_then(|u| u.as_object()) {
                            if let Ok(mut metrics) = self.metrics.lock() {
                                if let Some(tokens) =
                                    usage.get("output_tokens").and_then(|t| t.as_u64())
                                {
                                    metrics.token_count += tokens;
                                }
                            }
                        }
                    }
                    "message_start" => {
                        if let Some(usage) = data
                            .get("message")
                            .and_then(|m| m.get("usage"))
                            .and_then(|u| u.as_object())
                        {
                            if let Some(tokens) = usage.get("input_tokens").and_then(|t| t.as_u64())
                            {
                                stream_input_tokens = Some(tokens);
                                if let Ok(mut metrics) = self.metrics.lock() {
                                    metrics.token_count += tokens;
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // Update metrics
        {
            if let Ok(mut metrics) = self.metrics.lock() {
                metrics.request_count += 1;
            }
        }

        Ok(LLMResponse {
            content: if content_text.is_empty() {
                None
            } else {
                Some(content_text)
            },
            tool_calls,
            reasoning_content: None,
            input_tokens: stream_input_tokens,
        })
    }

    fn default_model(&self) -> &str {
        &self.default_model
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::base::Message;
    use futures_util::stream;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_stream_chunk_timeout_fires() {
        // Simulate a stream that stalls forever
        let mut stalled: futures_util::stream::Pending<Option<String>> = stream::pending();

        let result = tokio::time::timeout(
            Duration::from_millis(50),
            futures_util::StreamExt::next(&mut stalled),
        )
        .await;

        assert!(result.is_err(), "Timeout should fire on stalled stream");
    }

    #[tokio::test]
    async fn test_stream_chunk_timeout_does_not_fire_on_data() {
        let items: Vec<String> = vec!["chunk1".to_string()];
        let mut ready_stream = stream::iter(items);

        let result = tokio::time::timeout(
            Duration::from_millis(500),
            futures_util::StreamExt::next(&mut ready_stream),
        )
        .await;

        assert!(result.is_ok(), "Timeout should not fire when data arrives");
        assert_eq!(result.unwrap().unwrap(), "chunk1");
    }

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
            .and(header("x-api-key", "test_key"))
            .and(header("anthropic-version", "2023-06-01"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "content": [{"type": "text", "text": "Hello! How can I help?"}],
                "model": "claude-sonnet-4-5-20250929",
                "role": "assistant",
                "stop_reason": "end_turn",
                "usage": {"input_tokens": 10, "output_tokens": 8}
            })))
            .mount(&server)
            .await;

        let provider = AnthropicProvider::with_base_url("test_key".to_string(), None, server.uri());
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
                "content": [
                    {"type": "tool_use", "id": "tc_1", "name": "weather", "input": {"city": "NYC"}}
                ],
                "model": "claude-sonnet-4-5-20250929",
                "role": "assistant",
                "stop_reason": "tool_use",
                "usage": {"input_tokens": 20, "output_tokens": 15}
            })))
            .mount(&server)
            .await;

        let provider = AnthropicProvider::with_base_url("test_key".to_string(), None, server.uri());
        let result = provider
            .chat(simple_chat_request("What's the weather in NYC?"))
            .await
            .unwrap();

        // Non-streaming chat() uses anthropic_common::parse_response which extracts tool calls
        assert!(result.has_tool_calls());
        assert_eq!(result.tool_calls[0].name, "weather");
        assert_eq!(result.tool_calls[0].id, "tc_1");
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

        let provider = AnthropicProvider::with_base_url("bad_key".to_string(), None, server.uri());
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
                    .insert_header("retry-after", "30")
                    .set_body_json(json!({
                        "error": {"type": "rate_limit_error", "message": "Rate limit exceeded"}
                    })),
            )
            .mount(&server)
            .await;

        let provider = AnthropicProvider::with_base_url("test_key".to_string(), None, server.uri());
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
                "error": {"type": "api_error", "message": "Internal server error"}
            })))
            .mount(&server)
            .await;

        let provider = AnthropicProvider::with_base_url("test_key".to_string(), None, server.uri());
        let result = provider.chat(simple_chat_request("Hi")).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_chat_with_system_message() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "content": [{"type": "text", "text": "I am a helpful assistant."}],
                "model": "claude-sonnet-4-5-20250929",
                "role": "assistant",
                "stop_reason": "end_turn",
                "usage": {"input_tokens": 25, "output_tokens": 10}
            })))
            .mount(&server)
            .await;

        let provider = AnthropicProvider::with_base_url("test_key".to_string(), None, server.uri());
        let req = ChatRequest {
            messages: vec![
                Message::system("You are a helpful assistant."),
                Message::user("Hello"),
            ],
            tools: None,
            model: None,
            max_tokens: 1024,
            temperature: 0.7,
            tool_choice: None,
        };
        let result = provider.chat(req).await.unwrap();

        assert_eq!(result.content.unwrap(), "I am a helpful assistant.");
    }

    #[tokio::test]
    async fn test_chat_metrics_updated() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "content": [{"type": "text", "text": "Hi"}],
                "model": "claude-sonnet-4-5-20250929",
                "role": "assistant",
                "stop_reason": "end_turn",
                "usage": {"input_tokens": 5, "output_tokens": 3}
            })))
            .mount(&server)
            .await;

        let provider = AnthropicProvider::with_base_url("test_key".to_string(), None, server.uri());
        provider.chat(simple_chat_request("Hi")).await.unwrap();

        let metrics = provider.metrics.lock().unwrap();
        assert_eq!(metrics.request_count, 1);
        assert_eq!(metrics.token_count, 8); // 5 input + 3 output
    }
}
