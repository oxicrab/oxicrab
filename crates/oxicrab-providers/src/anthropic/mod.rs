use crate::anthropic_common;
use crate::errors::ProviderErrorHandler;
use crate::{API_URL_ANTHROPIC, PROVIDER_REQUEST_TIMEOUT_SECS, provider_http_client};
use anyhow::{Context, Result};
use async_trait::async_trait;
use oxicrab_core::providers::base::{ChatRequest, LLMProvider, LLMResponse};
use reqwest::Client;
use serde_json::json;
use std::time::Duration;
use tracing::debug;

pub struct AnthropicProvider {
    api_key: String,
    default_model: String,
    base_url: String,
    client: Client,
    custom_headers: std::collections::HashMap<String, String>,
}

impl AnthropicProvider {
    pub fn new(api_key: String, default_model: Option<String>) -> Self {
        Self {
            api_key,
            default_model: default_model
                .unwrap_or_else(|| "claude-sonnet-4-5-20250929".to_string()),
            base_url: API_URL_ANTHROPIC.to_string(),
            client: provider_http_client(),
            custom_headers: std::collections::HashMap::new(),
        }
    }

    pub fn with_config(
        api_key: String,
        default_model: Option<String>,
        base_url: String,
        custom_headers: std::collections::HashMap<String, String>,
    ) -> Self {
        Self {
            api_key,
            default_model: default_model
                .unwrap_or_else(|| "claude-sonnet-4-5-20250929".to_string()),
            base_url,
            client: provider_http_client(),
            custom_headers,
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
                .connect_timeout(Duration::from_secs(crate::PROVIDER_CONNECT_TIMEOUT_SECS))
                .build()
                .unwrap_or_else(|_| Client::new()),
            custom_headers: std::collections::HashMap::new(),
        }
    }
}

#[async_trait]
impl LLMProvider for AnthropicProvider {
    async fn chat(&self, req: &ChatRequest) -> Result<LLMResponse> {
        debug!(
            "anthropic chat: model={}",
            req.model.as_deref().unwrap_or(&self.default_model)
        );
        let payload = anthropic_common::build_anthropic_chat_payload(req, &self.default_model);

        let req_builder = self
            .client
            .post(&self.base_url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("anthropic-beta", "claude-code-20250219");
        let req_builder = crate::apply_custom_headers(req_builder, &self.custom_headers);
        let resp = req_builder
            .json(&payload)
            .timeout(Duration::from_secs(PROVIDER_REQUEST_TIMEOUT_SECS))
            .send()
            .await
            .context("Failed to send request to Anthropic API")?;

        let json = ProviderErrorHandler::check_response(resp, "Anthropic").await?;

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

    async fn warmup(&self) -> Result<()> {
        let payload = json!({
            "model": self.default_model,
            "messages": [{"role": "user", "content": "hi"}],
            "max_tokens": 1,
        });
        let mut headers = vec![
            ("x-api-key", self.api_key.clone()),
            ("anthropic-version", "2023-06-01".to_string()),
            ("anthropic-beta", "claude-code-20250219".to_string()),
            ("content-type", "application/json".to_string()),
        ];
        for (k, v) in &self.custom_headers {
            headers.push((k.as_str(), v.clone()));
        }
        crate::warmup_provider(&self.client, &self.base_url, headers, payload, "anthropic").await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
