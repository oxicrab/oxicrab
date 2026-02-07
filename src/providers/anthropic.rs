use crate::providers::base::{LLMProvider, LLMResponse, Message, ToolCallRequest, ToolDefinition, Usage};
use async_trait::async_trait;
use anyhow::{Context, Result};
use reqwest::Client;
use serde::Serialize;
use serde_json::{json, Value};

const API_URL: &str = "https://api.anthropic.com/v1/messages";

pub struct AnthropicProvider {
    api_key: String,
    default_model: String,
    client: Client,
}

impl AnthropicProvider {
    pub fn new(api_key: String, default_model: Option<String>) -> Self {
        Self {
            api_key,
            default_model: default_model.unwrap_or_else(|| "claude-sonnet-4-5-20250929".to_string()),
            client: Client::new(),
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
        tracing::debug!("Anthropic API response: {}", serde_json::to_string_pretty(&json).unwrap_or_else(|_| "failed to serialize".to_string()));
        
        let content = json["content"]
            .as_array()
            .and_then(|arr| {
                arr.iter()
                    .find_map(|block| {
                        if block["type"] == "text" {
                            block["text"].as_str().map(|s| s.to_string())
                        } else {
                            None
                        }
                    })
            });
        
        // Debug: log what content was extracted
        if content.is_none() {
            tracing::warn!("No text content found in Anthropic response. Content array: {:?}", json["content"]);
        } else {
            tracing::debug!("Extracted content length: {} chars", content.as_ref().unwrap().len());
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

        let usage_obj = json["usage"].as_object();
        let usage = Usage {
            prompt_tokens: usage_obj
                .and_then(|u| u["input_tokens"].as_u64())
                .unwrap_or(0) as u32,
            completion_tokens: usage_obj
                .and_then(|u| u["output_tokens"].as_u64())
                .unwrap_or(0) as u32,
            total_tokens: usage_obj
                .and_then(|u| {
                    u["input_tokens"].as_u64().and_then(|i| {
                        u["output_tokens"].as_u64().map(|o| i + o)
                    })
                })
                .unwrap_or(0) as u32,
        };

        Ok(LLMResponse {
            content,
            tool_calls,
            finish_reason: json["stop_reason"]
                .as_str()
                .unwrap_or("stop")
                .to_string(),
            usage,
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
            let error_text = resp.text().await.unwrap_or_else(|_| "unknown error".to_string());
            
            // Parse error JSON if possible to provide better error messages
            if let Ok(error_json) = serde_json::from_str::<Value>(&error_text) {
                if let Some(error) = error_json.get("error") {
                    let error_type = error.get("type").and_then(|v| v.as_str()).unwrap_or("unknown");
                    let error_msg = error.get("message").and_then(|v| v.as_str()).unwrap_or("Unknown error");
                    
                    // Provide helpful message for model not found errors
                    if error_type == "not_found_error" && error_msg.contains("model:") {
                        let model_name = error_msg.replace("model: ", "").trim().to_string();
                        return Err(anyhow::anyhow!(
                            "Model '{}' not found. This model may be deprecated or incorrect.\n\
                            Please update your config file (~/.nanobot/config.json) to use a valid model:\n\
                            - claude-sonnet-4-5-20250929 (recommended)\n\
                            - claude-haiku-4-5-20251001 (fastest)\n\
                            - claude-opus-4-5-20251101 (most capable)\n\
                            \n\
                            Or remove the 'model' field from your config to use the default.",
                            model_name
                        ));
                    }
                    
                    return Err(anyhow::anyhow!("Anthropic API error ({}): {}", error_type, error_msg));
                }
            }
            
            return Err(anyhow::anyhow!("Anthropic API error ({}): {}", status, error_text));
        }
        
        let json: Value = resp
            .json()
            .await
            .context("Failed to parse Anthropic API response")?;
        
        // Check for API-level errors in the JSON response
        if let Some(error) = json.get("error") {
            let error_type = error.get("type").and_then(|v| v.as_str()).unwrap_or("unknown");
            let error_msg = error.get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error");
            
            // Provide helpful message for model not found errors
            if error_type == "not_found_error" && error_msg.contains("model:") {
                let model_name = error_msg.replace("model: ", "").trim().to_string();
                return Err(anyhow::anyhow!(
                    "Model '{}' not found. This model may be deprecated or incorrect.\n\
                    Please update your config file (~/.nanobot/config.json) to use a valid model:\n\
                    - claude-sonnet-4-5-20250929 (recommended)\n\
                    - claude-haiku-4-5-20251001 (fastest)\n\
                    - claude-opus-4-5-20251101 (most capable)\n\
                    \n\
                    Or remove the 'model' field from your config to use the default.",
                    model_name
                ));
            }
            
            return Err(anyhow::anyhow!("Anthropic API error: {}", error_msg));
        }

        self.parse_response(json)
    }

    fn default_model(&self) -> &str {
        &self.default_model
    }
}
