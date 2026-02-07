use crate::providers::base::{
    LLMProvider, LLMResponse, Message, ToolCallRequest, ToolDefinition, Usage,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};

pub struct GeminiProvider {
    api_key: String,
    default_model: String,
    client: Client,
}

impl GeminiProvider {
    pub fn new(api_key: String, default_model: Option<String>) -> Self {
        Self {
            api_key,
            default_model: default_model.unwrap_or_else(|| "gemini-pro".to_string()),
            client: Client::new(),
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

        let usage_obj = json["usageMetadata"].as_object();
        let usage = Usage {
            prompt_tokens: usage_obj
                .and_then(|u| u["promptTokenCount"].as_u64())
                .unwrap_or(0) as u32,
            completion_tokens: usage_obj
                .and_then(|u| u["candidatesTokenCount"].as_u64())
                .unwrap_or(0) as u32,
            total_tokens: usage_obj
                .and_then(|u| {
                    u["promptTokenCount"]
                        .as_u64()
                        .and_then(|p| u["candidatesTokenCount"].as_u64().map(|c| p + c))
                })
                .unwrap_or(0) as u32,
        };

        Ok(LLMResponse {
            content,
            tool_calls,
            finish_reason: candidate["finishReason"]
                .as_str()
                .unwrap_or("STOP")
                .to_string(),
            usage,
            reasoning_content: None,
        })
    }
}

#[async_trait]
impl LLMProvider for GeminiProvider {
    async fn chat(
        &self,
        messages: Vec<Message>,
        tools: Option<Vec<ToolDefinition>>,
        model: Option<&str>,
        max_tokens: u32,
        temperature: f32,
    ) -> Result<LLMResponse> {
        let gemini_contents: Vec<Value> = messages
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
                "maxOutputTokens": max_tokens,
                "temperature": temperature,
            },
        });

        if let Some(tools) = tools {
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

        let model_name = model.unwrap_or(&self.default_model);
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

        let json: Value = resp
            .json()
            .await
            .context("Failed to parse Gemini API response")?;

        self.parse_response(json)
    }

    fn default_model(&self) -> &str {
        &self.default_model
    }
}
