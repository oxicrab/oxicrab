use crate::providers::base::{
    LLMProvider, LLMResponse, Message, ProviderMetrics, ToolCallRequest, ToolDefinition,
};
use crate::providers::errors::ProviderErrorHandler;
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::Serialize;
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

    fn convert_messages(&self, messages: Vec<Message>) -> (Option<String>, Vec<AnthropicMessage>) {
        let mut system_parts = Vec::new();
        let mut anthropic_messages = Vec::new();

        for msg in messages {
            match msg.role.as_str() {
                "system" => {
                    system_parts.push(msg.content);
                }
                "user" => {
                    // Anthropic API accepts content as either a string or array of blocks
                    // For simple text messages, use a string
                    anthropic_messages.push(AnthropicMessage {
                        role: "user".to_string(),
                        content: Value::String(msg.content),
                    });
                }
                "assistant" => {
                    let mut content: Vec<Value> = vec![json!({
                        "type": "text",
                        "text": msg.content
                    })];

                    if let Some(tool_calls) = msg.tool_calls {
                        for tc in tool_calls {
                            content.push(json!({
                                "type": "tool_use",
                                "id": tc.id,
                                "name": tc.name,
                                "input": tc.arguments
                            }));
                        }
                    }

                    anthropic_messages.push(AnthropicMessage {
                        role: "assistant".to_string(),
                        content: Value::Array(content),
                    });
                }
                "tool" => {
                    if let Some(tool_call_id) = msg.tool_call_id {
                        anthropic_messages.push(AnthropicMessage {
                            role: "user".to_string(),
                            content: Value::Array(vec![json!({
                                "type": "tool_result",
                                "tool_use_id": tool_call_id,
                                "content": msg.content
                            })]),
                        });
                    }
                }
                _ => {}
            }
        }

        let system = if system_parts.is_empty() {
            None
        } else {
            Some(system_parts.join("\n\n"))
        };

        (system, anthropic_messages)
    }

    fn convert_tools(&self, tools: Vec<ToolDefinition>) -> Vec<AnthropicTool> {
        tools
            .into_iter()
            .map(|t| AnthropicTool {
                name: t.name,
                description: t.description,
                input_schema: t.parameters,
            })
            .collect()
    }

    fn parse_response(&self, json: Value) -> Result<LLMResponse> {
        // Debug: log the raw response structure
        tracing::debug!(
            "Anthropic API response: {}",
            serde_json::to_string_pretty(&json)
                .unwrap_or_else(|_| "failed to serialize".to_string())
        );

        let content = json["content"].as_array().and_then(|arr| {
            arr.iter().find_map(|block| {
                if block["type"] == "text" {
                    block["text"].as_str().map(|s| s.to_string())
                } else {
                    None
                }
            })
        });

        // Debug: log what content was extracted
        if content.is_none() {
            tracing::warn!(
                "No text content found in Anthropic response. Content array: {:?}",
                json["content"]
            );
        } else {
            tracing::debug!(
                "Extracted content length: {} chars",
                content.as_ref().map(|c| c.len()).unwrap_or(0)
            );
        }

        let mut tool_calls = Vec::new();
        let mut reasoning_content = None;

        if let Some(content_array) = json["content"].as_array() {
            for block in content_array {
                if block["type"] == "tool_use" {
                    tool_calls.push(ToolCallRequest {
                        id: block["id"].as_str().unwrap_or("").to_string(),
                        name: block["name"].as_str().unwrap_or("").to_string(),
                        arguments: block["input"].clone(),
                    });
                } else if block["type"] == "thinking" {
                    reasoning_content = block["thinking"].as_str().map(|s| s.to_string());
                }
            }
        }

        Ok(LLMResponse {
            content,
            tool_calls,
            reasoning_content,
        })
    }
}

#[derive(Debug, Serialize)]
struct AnthropicMessage {
    role: String,
    content: Value,
}

#[derive(Debug, Serialize)]
struct AnthropicTool {
    name: String,
    description: String,
    #[serde(rename = "input_schema")]
    input_schema: Value,
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
        let (system, anthropic_messages) = self.convert_messages(messages);

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
            payload["tools"] = json!(self.convert_tools(tools));
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

        self.parse_response(json)
    }

    fn default_model(&self) -> &str {
        &self.default_model
    }
}
