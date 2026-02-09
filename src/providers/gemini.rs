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

const CONNECT_TIMEOUT_SECS: u64 = 30;
const REQUEST_TIMEOUT_SECS: u64 = 120;

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
            client: Client::builder()
                .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
                .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
                .build()
                .unwrap_or_else(|_| Client::new()),
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
    async fn chat(&self, req: ChatRequest<'_>) -> Result<LLMResponse> {
        let gemini_contents: Vec<Value> = req
            .messages
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
                "maxOutputTokens": req.max_tokens,
                "temperature": req.temperature,
            },
        });

        if let Some(tools) = req.tools {
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

        let model_name = req.model.unwrap_or(&self.default_model);
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

        let json = ProviderErrorHandler::check_response(resp, "Gemini", &self.metrics).await?;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_construction() {
        let provider = GeminiProvider::new("test_key".to_string(), None);
        assert_eq!(provider.default_model(), "gemini-pro");
    }

    #[test]
    fn test_provider_custom_model() {
        let provider =
            GeminiProvider::new("test_key".to_string(), Some("gemini-2.0-flash".to_string()));
        assert_eq!(provider.default_model(), "gemini-2.0-flash");
    }

    #[test]
    fn test_timeout_constants_are_sensible() {
        const {
            assert!(CONNECT_TIMEOUT_SECS <= 60);
            assert!(REQUEST_TIMEOUT_SECS >= 60);
            assert!(REQUEST_TIMEOUT_SECS <= 300);
        }
    }
}
