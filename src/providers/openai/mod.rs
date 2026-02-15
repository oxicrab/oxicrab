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
    provider_name: String,
    client: Client,
    metrics: std::sync::Arc<Mutex<ProviderMetrics>>,
}

impl OpenAIProvider {
    pub fn new(api_key: String, default_model: Option<String>) -> Self {
        Self {
            api_key,
            default_model: default_model.unwrap_or_else(|| "gpt-4o".to_string()),
            base_url: API_URL.to_string(),
            provider_name: "OpenAI".to_string(),
            client: Client::builder()
                .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
                .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
                .build()
                .unwrap_or_else(|_| Client::new()),
            metrics: std::sync::Arc::new(Mutex::new(ProviderMetrics::default())),
        }
    }

    pub fn with_config(
        api_key: String,
        default_model: String,
        base_url: String,
        provider_name: String,
    ) -> Self {
        Self {
            api_key,
            default_model,
            base_url,
            provider_name,
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
            provider_name: "OpenAI".to_string(),
            client: Client::builder()
                .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
                .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
                .build()
                .unwrap_or_else(|_| Client::new()),
            metrics: std::sync::Arc::new(Mutex::new(ProviderMetrics::default())),
        }
    }

    fn parse_response(json: &Value) -> Result<LLMResponse> {
        let choice = json["choices"]
            .as_array()
            .and_then(|arr| arr.first())
            .context("No choices in OpenAI response")?;

        let message = &choice["message"];
        let content = message["content"]
            .as_str()
            .map(std::string::ToString::to_string);

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
                        .map(|tc| {
                            let args_str = serde_json::to_string(&tc.arguments)
                                .unwrap_or_else(|_| "{}".to_string());
                            json!({
                                "id": tc.id,
                                "type": "function",
                                "function": {
                                    "name": tc.name,
                                    "arguments": args_str
                                }
                            })
                        })
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
            .context(format!(
                "Failed to send request to {} API",
                self.provider_name
            ))?;

        let json =
            ProviderErrorHandler::check_response(resp, &self.provider_name, &self.metrics).await?;

        // Update metrics on success
        {
            if let Ok(mut metrics) = self.metrics.lock() {
                metrics.request_count += 1;
                if let Some(usage) = json.get("usage").and_then(|u| u.as_object()) {
                    if let Some(tokens) = usage
                        .get("total_tokens")
                        .and_then(serde_json::Value::as_u64)
                    {
                        metrics.token_count += tokens;
                    }
                }
            }
        }

        Self::parse_response(&json)
    }

    fn default_model(&self) -> &str {
        &self.default_model
    }

    async fn warmup(&self) -> anyhow::Result<()> {
        use tracing::info;
        let start = std::time::Instant::now();
        let payload = json!({
            "model": self.default_model,
            "messages": [{"role": "user", "content": "hi"}],
            "max_tokens": 1,
        });
        let result = self
            .client
            .post(&self.base_url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .timeout(Duration::from_secs(15))
            .json(&payload)
            .send()
            .await;
        match result {
            Ok(_) => info!(
                "{} provider warmed up in {}ms",
                self.provider_name,
                start.elapsed().as_millis()
            ),
            Err(e) => tracing::warn!(
                "{} warmup request failed (non-fatal): {}",
                self.provider_name,
                e
            ),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
