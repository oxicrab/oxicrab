use crate::providers::anthropic_common;
use crate::providers::base::{ChatRequest, LLMProvider, LLMResponse, ProviderMetrics};
use crate::providers::errors::ProviderErrorHandler;
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::json;
use std::time::Duration;
use tracing::{debug, info};

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
                .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
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
        debug!(
            "anthropic chat: model={}",
            req.model.unwrap_or(&self.default_model)
        );
        let (system, anthropic_messages) = anthropic_common::convert_messages(req.messages);

        let mut payload = json!({
            "model": req.model.unwrap_or(&self.default_model),
            "messages": anthropic_messages,
            "max_tokens": req.max_tokens,
            "temperature": req.temperature,
        });

        if let Some(system) = system {
            payload["system"] = anthropic_common::system_to_content_blocks(&system);
        }

        if let Some(tools) = req.tools {
            payload["tools"] = serde_json::Value::Array(anthropic_common::convert_tools(tools));
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
                    if let Some(tokens) = usage
                        .get("input_tokens")
                        .and_then(serde_json::Value::as_u64)
                    {
                        metrics.token_count += tokens;
                    }
                    if let Some(tokens) = usage
                        .get("output_tokens")
                        .and_then(serde_json::Value::as_u64)
                    {
                        metrics.token_count += tokens;
                    }
                }
            }
        }

        let response = anthropic_common::parse_response(&json);
        debug!(
            "anthropic chat complete: input_tokens={:?}, output_tokens={:?}",
            response.input_tokens, response.output_tokens
        );
        Ok(response)
    }

    fn default_model(&self) -> &str {
        &self.default_model
    }

    fn metrics(&self) -> ProviderMetrics {
        self.metrics
            .lock()
            .map_or_else(|_| ProviderMetrics::default(), |m| m.clone())
    }

    async fn warmup(&self) -> Result<()> {
        let start = std::time::Instant::now();
        let payload = json!({
            "model": self.default_model,
            "messages": [{"role": "user", "content": "hi"}],
            "max_tokens": 1,
        });
        let result = self
            .client
            .post(&self.base_url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .timeout(Duration::from_secs(15))
            .json(&payload)
            .send()
            .await;
        match result {
            Ok(_) => info!(
                "anthropic provider warmed up in {}ms",
                start.elapsed().as_millis()
            ),
            Err(e) => tracing::warn!("anthropic warmup request failed (non-fatal): {}", e),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
