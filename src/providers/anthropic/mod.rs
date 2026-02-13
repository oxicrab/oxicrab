use crate::providers::anthropic_common;
use crate::providers::base::{ChatRequest, LLMProvider, LLMResponse, ProviderMetrics};
use crate::providers::errors::ProviderErrorHandler;
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::json;
use std::time::Duration;

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const CONNECT_TIMEOUT_SECS: u64 = 30;
const REQUEST_TIMEOUT_SECS: u64 = 120;

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
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
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

    fn default_model(&self) -> &str {
        &self.default_model
    }
}

#[cfg(test)]
mod tests;
