use crate::providers::base::{
    ChatRequest, LLMProvider, LLMResponse, ProviderMetrics, ToolCallRequest,
};
use crate::providers::errors::ProviderErrorHandler;
use crate::providers::provider_http_client;
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{Value, json};
use std::sync::Mutex;
use std::time::Duration;
use tracing::{debug, info, warn};

const API_URL: &str = "https://api.openai.com/v1/chat/completions";

pub struct OpenAIProvider {
    api_key: String,
    default_model: String,
    base_url: String,
    provider_name: String,
    client: Client,
    metrics: std::sync::Arc<Mutex<ProviderMetrics>>,
    custom_headers: std::collections::HashMap<String, String>,
}

impl OpenAIProvider {
    pub fn new(api_key: String, default_model: Option<String>) -> Self {
        Self {
            api_key,
            default_model: default_model.unwrap_or_else(|| "gpt-4o".to_string()),
            base_url: API_URL.to_string(),
            provider_name: "OpenAI".to_string(),
            client: provider_http_client(),
            metrics: std::sync::Arc::new(Mutex::new(ProviderMetrics::default())),
            custom_headers: std::collections::HashMap::new(),
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
            client: provider_http_client(),
            metrics: std::sync::Arc::new(Mutex::new(ProviderMetrics::default())),
            custom_headers: std::collections::HashMap::new(),
        }
    }

    /// Create a provider with custom headers injected into every request.
    pub fn with_config_and_headers(
        api_key: String,
        default_model: String,
        base_url: String,
        provider_name: String,
        custom_headers: std::collections::HashMap<String, String>,
    ) -> Self {
        Self {
            api_key,
            default_model,
            base_url,
            provider_name,
            client: provider_http_client(),
            metrics: std::sync::Arc::new(Mutex::new(ProviderMetrics::default())),
            custom_headers,
        }
    }

    #[cfg(test)]
    fn with_base_url(api_key: String, default_model: Option<String>, base_url: String) -> Self {
        Self {
            api_key,
            default_model: default_model.unwrap_or_else(|| "gpt-4o".to_string()),
            base_url,
            provider_name: "OpenAI".to_string(),
            client: provider_http_client(),
            metrics: std::sync::Arc::new(Mutex::new(ProviderMetrics::default())),
            custom_headers: std::collections::HashMap::new(),
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
                    let arguments = match function["arguments"].as_str() {
                        Some(s) => match serde_json::from_str(s) {
                            Ok(v) => v,
                            Err(e) => {
                                warn!(
                                    "skipping tool call '{}': failed to parse arguments: {}",
                                    function["name"].as_str().unwrap_or("unknown"),
                                    e
                                );
                                continue;
                            }
                        },
                        None => json!({}),
                    };

                    tool_calls.push(ToolCallRequest {
                        id: tc["id"].as_str().unwrap_or("").to_string(),
                        name: function["name"].as_str().unwrap_or("").to_string(),
                        arguments,
                    });
                }
            }
        }

        let input_tokens = json
            .get("usage")
            .and_then(|u| u.get("prompt_tokens"))
            .and_then(serde_json::Value::as_u64);

        let output_tokens = json
            .get("usage")
            .and_then(|u| u.get("completion_tokens"))
            .and_then(serde_json::Value::as_u64);

        // DeepSeek-R1 and similar models return reasoning in this field
        let reasoning_content = message["reasoning_content"]
            .as_str()
            .map(std::string::ToString::to_string);

        Ok(LLMResponse {
            content,
            tool_calls,
            reasoning_content,
            input_tokens,
            output_tokens,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
        })
    }
}

#[async_trait]
impl LLMProvider for OpenAIProvider {
    async fn chat(&self, req: ChatRequest<'_>) -> Result<LLMResponse> {
        debug!(
            "{} chat: model={}",
            self.provider_name,
            req.model.unwrap_or(&self.default_model)
        );
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
                        if img.media_type.starts_with("image/") {
                            parts.push(json!({
                                "type": "image_url",
                                "image_url": {
                                    "url": format!("data:{};base64,{}", img.media_type, img.data)
                                }
                            }));
                        } else {
                            // Documents (PDFs, etc.) use the file content block
                            parts.push(json!({
                                "type": "file",
                                "file": {
                                    "filename": "document",
                                    "file_data": format!("data:{};base64,{}", img.media_type, img.data)
                                }
                            }));
                        }
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
                    m["tool_calls"] = json!(
                        tool_calls
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
                            .collect::<Vec<_>>()
                    );
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

        if let Some(ref format) = req.response_format {
            match format {
                crate::providers::base::ResponseFormat::JsonObject => {
                    payload["response_format"] = json!({"type": "json_object"});
                }
                crate::providers::base::ResponseFormat::JsonSchema { name, schema } => {
                    payload["response_format"] = json!({
                        "type": "json_schema",
                        "json_schema": {
                            "name": name,
                            "schema": schema,
                            "strict": true
                        }
                    });
                }
            }
        }

        if let Some(tools) = req.tools {
            payload["tools"] = json!(
                tools
                    .into_iter()
                    .map(|t| json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.parameters
                        }
                    }))
                    .collect::<Vec<_>>()
            );
            if let Some(ref choice) = req.tool_choice {
                // Map Anthropic-style "any" to OpenAI's "required"
                let mapped = match choice.as_str() {
                    "any" => "required",
                    other => other,
                };
                payload["tool_choice"] = json!(mapped);
            }
        }

        let mut req = self
            .client
            .post(&self.base_url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json");
        for (k, v) in &self.custom_headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let provider_name = &self.provider_name;
        let resp = req
            .json(&payload)
            .send()
            .await
            .with_context(|| format!("Failed to send request to {} API", provider_name))?;

        let json =
            ProviderErrorHandler::check_response(resp, &self.provider_name, &self.metrics).await?;

        // Update metrics on success
        {
            if let Ok(mut metrics) = self.metrics.lock() {
                metrics.request_count += 1;
                if let Some(usage) = json.get("usage").and_then(|u| u.as_object())
                    && let Some(tokens) = usage
                        .get("total_tokens")
                        .and_then(serde_json::Value::as_u64)
                {
                    metrics.token_count += tokens;
                }
            }
        }

        let response = Self::parse_response(&json)?;
        debug!(
            "{} chat complete: input_tokens={:?}, output_tokens={:?}",
            self.provider_name, response.input_tokens, response.output_tokens
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
        let payload = json!({
            "model": self.default_model,
            "messages": [{"role": "user", "content": "hi"}],
            "max_tokens": 1,
        });
        let mut req = self
            .client
            .post(&self.base_url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .timeout(Duration::from_secs(15));
        for (k, v) in &self.custom_headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let result = req.json(&payload).send().await;
        match result {
            Ok(resp) if !resp.status().is_success() => {
                warn!(
                    "{} warmup got HTTP {} (non-fatal)",
                    self.provider_name,
                    resp.status()
                );
            }
            Ok(_) => info!(
                "{} provider warmed up in {}ms",
                self.provider_name,
                start.elapsed().as_millis()
            ),
            Err(e) => warn!(
                "{} warmup request failed (non-fatal): {}",
                self.provider_name, e
            ),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
