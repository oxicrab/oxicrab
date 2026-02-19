use crate::providers::base::{
    ChatRequest, LLMProvider, LLMResponse, ProviderMetrics, ToolCallRequest,
};
use crate::providers::errors::ProviderErrorHandler;
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{Value, json};
use std::sync::Mutex;
use std::time::Duration;
use tracing::{debug, info};

const CONNECT_TIMEOUT_SECS: u64 = 30;
const REQUEST_TIMEOUT_SECS: u64 = 120;

const BASE_URL: &str = "https://generativelanguage.googleapis.com/v1";

pub struct GeminiProvider {
    api_key: String,
    default_model: String,
    base_url: String,
    client: Client,
    metrics: std::sync::Arc<Mutex<ProviderMetrics>>,
}

impl GeminiProvider {
    pub fn new(api_key: String, default_model: Option<String>) -> Self {
        Self {
            api_key,
            default_model: default_model.unwrap_or_else(|| "gemini-pro".to_string()),
            base_url: BASE_URL.to_string(),
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
            default_model: default_model.unwrap_or_else(|| "gemini-pro".to_string()),
            base_url,
            client: Client::builder()
                .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
                .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
                .build()
                .unwrap_or_else(|_| Client::new()),
            metrics: std::sync::Arc::new(Mutex::new(ProviderMetrics::default())),
        }
    }

    fn parse_response(json: &Value) -> Result<LLMResponse> {
        let candidate = json["candidates"]
            .as_array()
            .and_then(|arr| arr.first())
            .context("No candidates in Gemini response")?;

        let content = candidate["content"]["parts"].as_array().and_then(|parts| {
            parts.iter().find_map(|p| {
                if p["text"].is_string() {
                    p["text"].as_str().map(std::string::ToString::to_string)
                } else {
                    None
                }
            })
        });

        let mut tool_calls = Vec::new();
        if let Some(parts) = candidate["content"]["parts"].as_array() {
            for (i, part) in parts.iter().enumerate() {
                // Gemini returns singular `functionCall` per part (not plural array)
                if let Some(fc) = part.get("functionCall") {
                    // Gemini doesn't return tool call IDs; use function name as ID
                    // since Gemini's functionResponse.name needs the actual function name
                    let id = fc["name"]
                        .as_str()
                        .map_or_else(|| format!("gemini_tc_{}", i), str::to_string);
                    tool_calls.push(ToolCallRequest {
                        id,
                        name: fc["name"].as_str().unwrap_or("").to_string(),
                        arguments: fc["args"].clone(),
                    });
                }
            }
        }

        let input_tokens = json
            .get("usageMetadata")
            .and_then(|u| u.get("promptTokenCount"))
            .and_then(serde_json::Value::as_u64);

        let output_tokens = json
            .get("usageMetadata")
            .and_then(|u| u.get("candidatesTokenCount"))
            .and_then(serde_json::Value::as_u64);

        Ok(LLMResponse {
            content,
            tool_calls,
            reasoning_content: None,
            input_tokens,
            output_tokens,
        })
    }
}

#[async_trait]
impl LLMProvider for GeminiProvider {
    async fn chat(&self, req: ChatRequest<'_>) -> Result<LLMResponse> {
        debug!(
            "gemini chat: model={}",
            req.model.unwrap_or(&self.default_model)
        );
        // Separate system messages for systemInstruction; rest go into contents
        let mut system_parts: Vec<String> = Vec::new();
        let mut gemini_contents: Vec<Value> = Vec::new();

        for msg in req.messages {
            if msg.role == "system" {
                system_parts.push(msg.content);
                continue;
            }

            if msg.role == "tool" {
                // Gemini expects tool results as functionResponse parts
                let tool_name = msg.tool_call_id.as_deref().unwrap_or("unknown");
                let response_value: Value = serde_json::from_str(&msg.content)
                    .unwrap_or_else(|_| json!({"result": msg.content}));
                gemini_contents.push(json!({
                    "role": "function",
                    "parts": [{
                        "functionResponse": {
                            "name": tool_name,
                            "response": response_value
                        }
                    }]
                }));
                continue;
            }

            let role = match msg.role.as_str() {
                "assistant" => "model",
                _ => "user",
            };

            let mut parts = vec![json!({"text": msg.content})];
            if msg.role == "user" {
                for img in &msg.images {
                    parts.push(json!({
                        "inline_data": {
                            "mime_type": img.media_type,
                            "data": img.data
                        }
                    }));
                }
            }

            gemini_contents.push(json!({
                "role": role,
                "parts": parts
            }));
        }

        let mut payload = json!({
            "contents": gemini_contents,
            "generationConfig": {
                "maxOutputTokens": req.max_tokens,
                "temperature": req.temperature,
            },
        });

        // Use Gemini's native systemInstruction field for system messages
        if !system_parts.is_empty() {
            payload["systemInstruction"] = json!({
                "parts": [{"text": system_parts.join("\n\n")}]
            });
        }

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
            // Map tool_choice to Gemini's functionCallingConfig
            if let Some(ref choice) = req.tool_choice {
                let mode = match choice.as_str() {
                    "any" => "ANY",
                    "none" => "NONE",
                    _ => "AUTO",
                };
                payload["toolConfig"] = json!({
                    "functionCallingConfig": { "mode": mode }
                });
            }
        }

        let model_name = req.model.unwrap_or(&self.default_model);
        let url = format!("{}/models/{}:generateContent", self.base_url, model_name);

        let resp = self
            .client
            .post(&url)
            .header("x-goog-api-key", &self.api_key)
            .json(&payload)
            .send()
            .await
            .context("Failed to send request to Gemini API")?;

        let json = ProviderErrorHandler::check_response(resp, "Gemini", &self.metrics).await?;

        // Update metrics on success
        {
            if let Ok(mut metrics) = self.metrics.lock() {
                metrics.request_count += 1;
                if let Some(usage) = json.get("usageMetadata").and_then(|u| u.as_object())
                    && let Some(tokens) = usage
                        .get("totalTokenCount")
                        .and_then(serde_json::Value::as_u64)
                {
                    metrics.token_count += tokens;
                }
            }
        }

        let response = Self::parse_response(&json)?;
        debug!(
            "gemini chat complete: input_tokens={:?}, output_tokens={:?}",
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

    async fn warmup(&self) -> anyhow::Result<()> {
        let start = std::time::Instant::now();
        let url = format!(
            "{}/models/{}:generateContent",
            self.base_url, self.default_model
        );
        let payload = json!({
            "contents": [{"parts": [{"text": "hi"}]}],
            "generationConfig": {"maxOutputTokens": 1}
        });
        let result = self
            .client
            .post(&url)
            .header("x-goog-api-key", &self.api_key)
            .header("Content-Type", "application/json")
            .timeout(Duration::from_secs(15))
            .json(&payload)
            .send()
            .await;
        match result {
            Ok(_) => info!(
                "gemini provider warmed up in {}ms",
                start.elapsed().as_millis()
            ),
            Err(e) => tracing::warn!("gemini warmup request failed (non-fatal): {}", e),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
