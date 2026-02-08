use crate::providers::anthropic_common;
use crate::providers::base::{
    ChatRequest, LLMProvider, LLMResponse, ProviderMetrics, ToolCallRequest,
};
use crate::providers::errors::ProviderErrorHandler;
use crate::providers::sse::parse_sse_chunk;
use anyhow::{Context, Result};
use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};

const API_URL: &str = "https://api.anthropic.com/v1/messages";

pub struct AnthropicProvider {
    api_key: String,
    default_model: String,
    client: Client,
    metrics: std::sync::Arc<std::sync::Mutex<ProviderMetrics>>,
}

impl AnthropicProvider {
    pub fn new(api_key: String, default_model: Option<String>) -> Self {
        Self {
            api_key,
            default_model: default_model
                .unwrap_or_else(|| "claude-sonnet-4-5-20250929".to_string()),
            client: Client::new(),
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
            payload["tool_choice"] = json!({"type": "auto"});
        }

        let resp = self
            .client
            .post(API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&payload)
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
            payload["tool_choice"] = json!({"type": "auto"});
        }

        let resp = self
            .client
            .post(API_URL)
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

        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
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
                            if let Ok(mut metrics) = self.metrics.lock() {
                                if let Some(tokens) =
                                    usage.get("input_tokens").and_then(|t| t.as_u64())
                                {
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
        })
    }

    fn default_model(&self) -> &str {
        &self.default_model
    }
}
