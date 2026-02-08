use crate::providers::base::{
    LLMProvider, LLMResponse, Message, ProviderMetrics, ToolCallRequest, ToolDefinition,
};
use crate::providers::errors::ProviderErrorHandler;
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};
use std::sync::Mutex;

const API_URL: &str = "https://api.openai.com/v1/chat/completions";

pub struct OpenAIProvider {
    api_key: String,
    default_model: String,
    client: Client,
    metrics: std::sync::Arc<Mutex<ProviderMetrics>>,
}

impl OpenAIProvider {
    pub fn new(api_key: String, default_model: Option<String>) -> Self {
        Self {
            api_key,
            default_model: default_model.unwrap_or_else(|| "gpt-4o".to_string()),
            client: Client::new(),
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
            reasoning_content: None, // OpenAI doesn't expose reasoning content separately
        })
    }
}

#[async_trait]
impl LLMProvider for OpenAIProvider {
    async fn chat(
        &self,
        messages: Vec<Message>,
        tools: Option<Vec<ToolDefinition>>,
        model: Option<&str>,
        max_tokens: u32,
        temperature: f32,
    ) -> Result<LLMResponse> {
        let openai_messages: Vec<Value> = messages
            .into_iter()
            .map(|msg| {
                let mut m = json!({
                    "role": msg.role,
                    "content": msg.content,
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
            "model": model.unwrap_or(&self.default_model),
            "messages": openai_messages,
            "max_tokens": max_tokens,
            "temperature": temperature,
        });

        if let Some(tools) = tools {
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
        }

        let resp = self
            .client
            .post(API_URL)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .context("Failed to send request to OpenAI API")?;

        // Check for HTTP errors
        let status = resp.status();
        if !status.is_success() {
            // Extract headers before consuming response
            let retry_after = resp
                .headers()
                .get("retry-after")
                .and_then(|h| h.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok());

            let error_text = resp
                .text()
                .await
                .unwrap_or_else(|_| "unknown error".to_string());

            // Handle rate limiting
            if status == 429 {
                ProviderErrorHandler::log_and_handle_error(
                    &anyhow::anyhow!("Rate limit exceeded"),
                    "OpenAI",
                    "chat",
                );
                return Err(
                    ProviderErrorHandler::handle_rate_limit(status.as_u16(), retry_after)
                        .unwrap_err()
                        .into(),
                );
            }

            // Handle authentication errors
            if status == 401 || status == 403 {
                ProviderErrorHandler::log_and_handle_error(
                    &anyhow::anyhow!("Authentication failed"),
                    "OpenAI",
                    "chat",
                );
                return Err(
                    ProviderErrorHandler::handle_auth_error(status.as_u16(), &error_text)
                        .unwrap_err()
                        .into(),
                );
            }

            // Use shared error handler for other errors
            ProviderErrorHandler::log_and_handle_error(
                &anyhow::anyhow!("API error"),
                "OpenAI",
                "chat",
            );
            return Err(
                ProviderErrorHandler::parse_api_error(status.as_u16(), &error_text)
                    .unwrap_err()
                    .into(),
            );
        }

        let json: Value = resp
            .json()
            .await
            .context("Failed to parse OpenAI API response")?;

        // Check for API-level errors in the JSON response
        if let Some(error) = json.get("error") {
            // Update error metrics
            {
                if let Ok(mut metrics) = self.metrics.lock() {
                    metrics.error_count += 1;
                }
            }

            let error_text =
                serde_json::to_string(error).unwrap_or_else(|_| "Unknown error".to_string());
            ProviderErrorHandler::log_and_handle_error(
                &anyhow::anyhow!("API error in response"),
                "OpenAI",
                "chat",
            );
            return Err(ProviderErrorHandler::parse_api_error(200, &error_text)
                .unwrap_err()
                .into());
        }

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
