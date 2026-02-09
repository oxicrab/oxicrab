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
    client: Client,
    metrics: std::sync::Arc<Mutex<ProviderMetrics>>,
}

impl OpenAIProvider {
    pub fn new(api_key: String, default_model: Option<String>) -> Self {
        Self {
            api_key,
            default_model: default_model.unwrap_or_else(|| "gpt-4o".to_string()),
            client: Client::builder()
                .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
                .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
                .build()
                .unwrap_or_else(|_| Client::new()),
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
    async fn chat(&self, req: ChatRequest<'_>) -> Result<LLMResponse> {
        let openai_messages: Vec<Value> = req
            .messages
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

        let json = ProviderErrorHandler::check_response(resp, "OpenAI", &self.metrics).await?;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_construction() {
        let provider = OpenAIProvider::new("test_key".to_string(), None);
        assert_eq!(provider.default_model(), "gpt-4o");
    }

    #[test]
    fn test_provider_custom_model() {
        let provider = OpenAIProvider::new("test_key".to_string(), Some("gpt-4-turbo".to_string()));
        assert_eq!(provider.default_model(), "gpt-4-turbo");
    }

    #[test]
    fn test_timeout_constants_are_sensible() {
        assert!(CONNECT_TIMEOUT_SECS <= 60);
        assert!(REQUEST_TIMEOUT_SECS >= 60);
        assert!(REQUEST_TIMEOUT_SECS <= 300);
    }
}
