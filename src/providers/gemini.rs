use crate::providers::base::{
    LLMProvider, LLMResponse, Message, ProviderMetrics, ToolCallRequest, ToolDefinition,
};
use crate::providers::errors::ProviderErrorHandler;
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};
use std::sync::Mutex;

pub struct GeminiProvider {
    api_key: String,
    default_model: String,
    client: Client,
    metrics: std::sync::Arc<Mutex<ProviderMetrics>>,
}

impl GeminiProvider {
    pub fn new(api_key: String, default_model: Option<String>) -> Self {
        Self {
            api_key,
            default_model: default_model.unwrap_or_else(|| "gemini-pro".to_string()),
            client: Client::new(),
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
        })
    }
}

#[async_trait]
impl LLMProvider for GeminiProvider {
    async fn chat(
        &self,
        messages: Vec<Message>,
        tools: Option<Vec<ToolDefinition>>,
        model: Option<&str>,
        max_tokens: u32,
        temperature: f32,
    ) -> Result<LLMResponse> {
        let gemini_contents: Vec<Value> = messages
            .into_iter()
            .map(|msg| {
                let role = match msg.role.as_str() {
                    "system" => "user", // Gemini doesn't have system role
                    "user" => "user",
                    "assistant" => "model",
                    "tool" => "function",
                    _ => "user",
                };

                json!({
                    "role": role,
                    "parts": [{"text": msg.content}]
                })
            })
            .collect();

        let mut payload = json!({
            "contents": gemini_contents,
            "generationConfig": {
                "maxOutputTokens": max_tokens,
                "temperature": temperature,
            },
        });

        if let Some(tools) = tools {
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

        let model_name = model.unwrap_or(&self.default_model);
        let url = format!(
            "https://generativelanguage.googleapis.com/v1/models/{}:generateContent?key={}",
            model_name, self.api_key
        );

        let resp = self
            .client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .context("Failed to send request to Gemini API")?;

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
                    "Gemini",
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
                    "Gemini",
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
                "Gemini",
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
            .context("Failed to parse Gemini API response")?;

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
                "Gemini",
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
