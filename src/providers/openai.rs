use crate::providers::base::{LLMProvider, LLMResponse, Message, ToolCallRequest, ToolDefinition, Usage};
use async_trait::async_trait;
use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::{json, Value};

const API_URL: &str = "https://api.openai.com/v1/chat/completions";

pub struct OpenAIProvider {
    api_key: String,
    default_model: String,
    client: Client,
}

impl OpenAIProvider {
    pub fn new(api_key: String, default_model: Option<String>) -> Self {
        Self {
            api_key,
            default_model: default_model.unwrap_or_else(|| "gpt-4o".to_string()),
            client: Client::new(),
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

        let usage_obj = json["usage"].as_object();
        let usage = Usage {
            prompt_tokens: usage_obj
                .and_then(|u| u["prompt_tokens"].as_u64())
                .unwrap_or(0) as u32,
            completion_tokens: usage_obj
                .and_then(|u| u["completion_tokens"].as_u64())
                .unwrap_or(0) as u32,
            total_tokens: usage_obj
                .and_then(|u| u["total_tokens"].as_u64())
                .unwrap_or(0) as u32,
        };

        Ok(LLMResponse {
            content,
            tool_calls,
            finish_reason: choice["finish_reason"]
                .as_str()
                .unwrap_or("stop")
                .to_string(),
            usage,
            reasoning_content: None, // OpenAI doesn't expose reasoning content separately
        })
    }
}

#[async_trait]
impl LLMProvider for OpenAIProvider {
    async fn chat(
        &self,
        messages: Vec<Message>,
        tools: Option<Vec<ToolDefinition>>,
        model: Option<&str>,
        max_tokens: u32,
        temperature: f32,
    ) -> Result<LLMResponse> {
        let openai_messages: Vec<Value> = messages
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
            "model": model.unwrap_or(&self.default_model),
            "messages": openai_messages,
            "max_tokens": max_tokens,
            "temperature": temperature,
        });

        if let Some(tools) = tools {
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

        let json: Value = resp
            .json()
            .await
            .context("Failed to parse OpenAI API response")?;

        self.parse_response(json)
    }

    fn default_model(&self) -> &str {
        &self.default_model
    }
}
