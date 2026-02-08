use crate::providers::anthropic_common;
use crate::providers::base::{
    LLMProvider, LLMResponse, Message, ProviderMetrics, ToolDefinition,
};
use crate::providers::errors::ProviderErrorHandler;
use anyhow::{Context, Result};
use async_trait::async_trait;
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
    async fn chat(
        &self,
        messages: Vec<Message>,
        tools: Option<Vec<ToolDefinition>>,
        model: Option<&str>,
        max_tokens: u32,
        temperature: f32,
    ) -> Result<LLMResponse> {
        let (system, anthropic_messages) = anthropic_common::convert_messages(messages);

        let mut payload = json!({
            "model": model.unwrap_or(&self.default_model),
            "messages": anthropic_messages,
            "max_tokens": max_tokens,
            "temperature": temperature,
        });

        if let Some(system) = system {
            payload["system"] = json!(system);
        }

        if let Some(tools) = tools {
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

        // Check for HTTP errors first
        let status = resp.status();
        tracing::debug!("Anthropic API response status: {}", status);
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
                    "Anthropic",
                    "chat",
                );
                return Err(ProviderErrorHandler::handle_rate_limit(status.as_u16(), retry_after)
                    .unwrap_err());
            }

            // Handle authentication errors
            if status == 401 || status == 403 {
                ProviderErrorHandler::log_and_handle_error(
                    &anyhow::anyhow!("Authentication failed"),
                    "Anthropic",
                    "chat",
                );
                return Err(ProviderErrorHandler::handle_auth_error(status.as_u16(), &error_text)
                    .unwrap_err());
            }

            // Use shared error handler for other errors
            ProviderErrorHandler::log_and_handle_error(
                &anyhow::anyhow!("API error"),
                "Anthropic",
                "chat",
            );
            return Err(ProviderErrorHandler::parse_api_error(status.as_u16(), &error_text)
                .unwrap_err());
        }

        let json: Value = resp
            .json()
            .await
            .context("Failed to parse Anthropic API response")?;

        // Check for API-level errors in the JSON response
        if let Some(error) = json.get("error") {
            // Update error metrics
            {
                if let Ok(mut metrics) = self.metrics.lock() {
                    metrics.error_count += 1;
                }
            }

            let error_text = serde_json::to_string(error)
                .unwrap_or_else(|_| "Unknown error".to_string());
            ProviderErrorHandler::log_and_handle_error(
                &anyhow::anyhow!("API error in response"),
                "Anthropic",
                "chat",
            );
            return Err(ProviderErrorHandler::parse_api_error(200, &error_text)
                .unwrap_err());
        }

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

    fn default_model(&self) -> &str {
        &self.default_model
    }
}
