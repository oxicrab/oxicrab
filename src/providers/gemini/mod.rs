use crate::providers::base::{ChatRequest, LLMProvider, LLMResponse, ToolCallRequest};
use crate::providers::errors::ProviderErrorHandler;
use crate::providers::provider_http_client;
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{Value, json};
use std::time::Duration;
use tracing::{debug, info};

const BASE_URL: &str = "https://generativelanguage.googleapis.com/v1";

pub struct GeminiProvider {
    api_key: String,
    default_model: String,
    base_url: String,
    client: Client,
    custom_headers: std::collections::HashMap<String, String>,
}

impl GeminiProvider {
    pub fn new(api_key: String, default_model: Option<String>) -> Self {
        Self {
            api_key,
            default_model: default_model.unwrap_or_else(|| "gemini-pro".to_string()),
            base_url: BASE_URL.to_string(),
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
            default_model: default_model.unwrap_or_else(|| "gemini-pro".to_string()),
            base_url,
            client: provider_http_client(),
            custom_headers,
        }
    }

    #[cfg(test)]
    fn with_base_url(api_key: String, default_model: Option<String>, base_url: String) -> Self {
        Self {
            api_key,
            default_model: default_model.unwrap_or_else(|| "gemini-pro".to_string()),
            base_url,
            client: provider_http_client(),
            custom_headers: std::collections::HashMap::new(),
        }
    }

    fn parse_response(json: &Value) -> Result<LLMResponse> {
        let candidate = json["candidates"]
            .as_array()
            .and_then(|arr| arr.first())
            .context("No candidates in Gemini response")?;

        // Detect safety-filtered or blocked responses
        if let Some(reason) = candidate["finishReason"].as_str()
            && matches!(
                reason,
                "SAFETY" | "BLOCKED" | "RECITATION" | "PROHIBITED_CONTENT" | "SPII"
            )
        {
            anyhow::bail!("Gemini response blocked (finishReason: {reason})");
        }

        let content = candidate["content"]["parts"].as_array().and_then(|parts| {
            let texts: Vec<String> = parts
                .iter()
                .filter_map(|p| p["text"].as_str().map(std::string::ToString::to_string))
                .collect();
            if texts.is_empty() {
                None
            } else {
                Some(texts.join("\n\n"))
            }
        });

        let mut tool_calls = Vec::new();
        if let Some(parts) = candidate["content"]["parts"].as_array() {
            for part in parts {
                // Gemini returns singular `functionCall` per part (not plural array)
                if let Some(fc) = part.get("functionCall") {
                    let name = fc["name"].as_str().unwrap_or_default().to_string();
                    // Generate a unique ID per tool call to avoid collisions
                    // when the same function is called multiple times
                    let id = format!("gemini_{}", &uuid::Uuid::new_v4().to_string()[..12]);
                    tool_calls.push(ToolCallRequest {
                        id,
                        name,
                        arguments: fc.get("args").cloned().unwrap_or(json!({})),
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
            input_tokens,
            output_tokens,
            ..Default::default()
        })
    }
}

#[async_trait]
impl LLMProvider for GeminiProvider {
    async fn chat(&self, req: ChatRequest) -> Result<LLMResponse> {
        debug!(
            "gemini chat: model={}",
            req.model.as_deref().unwrap_or(&self.default_model)
        );
        // Separate system messages for systemInstruction; rest go into contents
        let mut system_parts: Vec<String> = Vec::new();
        let mut gemini_contents: Vec<Value> = Vec::new();

        // Build tool_call_id → tool_name mapping from conversation history.
        // Gemini's functionResponse requires the function name (not call ID).
        let mut tool_id_to_name: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for msg in &req.messages {
            if let Some(ref tool_calls) = msg.tool_calls {
                for tc in tool_calls {
                    tool_id_to_name.insert(tc.id.clone(), tc.name.clone());
                }
            }
        }

        for msg in req.messages {
            if msg.role == "system" {
                system_parts.push(msg.content);
                continue;
            }

            if msg.role == "tool" {
                // Gemini expects tool results as functionResponse parts
                let tool_name = msg
                    .tool_call_id
                    .as_deref()
                    .and_then(|id| tool_id_to_name.get(id))
                    .map_or("unknown", String::as_str);
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

            let mut parts = Vec::new();
            if !msg.content.is_empty() {
                parts.push(json!({"text": msg.content}));
            }
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
            if msg.role == "assistant"
                && let Some(ref tool_calls) = msg.tool_calls
            {
                for tc in tool_calls {
                    parts.push(json!({
                        "functionCall": {
                            "name": tc.name,
                            "args": tc.arguments
                        }
                    }));
                }
            }
            if parts.is_empty() {
                parts.push(json!({"text": ""}));
            }

            gemini_contents.push(json!({
                "role": role,
                "parts": parts
            }));
        }

        // Merge consecutive messages with the same role (Gemini requires alternation)
        let mut merged: Vec<Value> = Vec::with_capacity(gemini_contents.len());
        for entry in gemini_contents {
            if let Some(last) = merged.last_mut()
                && last["role"] == entry["role"]
            {
                if let (Some(last_parts), Some(new_parts)) =
                    (last["parts"].as_array_mut(), entry["parts"].as_array())
                {
                    last_parts.extend(new_parts.iter().cloned());
                }
            } else {
                merged.push(entry);
            }
        }

        let mut payload = json!({
            "contents": merged,
            "generationConfig": {
                "maxOutputTokens": req.max_tokens,
            },
        });
        if let Some(temp) = req.temperature {
            payload["generationConfig"]["temperature"] = json!(temp);
        }

        if let Some(ref format) = req.response_format {
            match format {
                crate::providers::base::ResponseFormat::JsonObject => {
                    payload["generationConfig"]["responseMimeType"] = json!("application/json");
                }
                crate::providers::base::ResponseFormat::JsonSchema { schema, .. } => {
                    payload["generationConfig"]["responseMimeType"] = json!("application/json");
                    payload["generationConfig"]["responseSchema"] = schema.clone();
                }
            }
        }

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

        let model_name = req.model.as_deref().unwrap_or(&self.default_model);
        // URL-encode model name to prevent path injection
        let encoded_model = urlencoding::encode(model_name);
        let url = format!("{}/models/{}:generateContent", self.base_url, encoded_model);

        let mut req_builder = self
            .client
            .post(&url)
            .header("x-goog-api-key", &self.api_key);
        for (k, v) in &self.custom_headers {
            req_builder = req_builder.header(k.as_str(), v.as_str());
        }
        let resp = req_builder
            .json(&payload)
            .send()
            .await
            .context("Failed to send request to Gemini API")?;

        let json = ProviderErrorHandler::check_response(resp, "Gemini").await?;

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

    async fn warmup(&self) -> anyhow::Result<()> {
        use tracing::warn;
        let start = std::time::Instant::now();
        let encoded_model = urlencoding::encode(&self.default_model);
        let url = format!("{}/models/{}:generateContent", self.base_url, encoded_model);
        let payload = json!({
            "contents": [{"parts": [{"text": "hi"}]}],
            "generationConfig": {"maxOutputTokens": 1}
        });
        let mut req_builder = self
            .client
            .post(&url)
            .header("x-goog-api-key", &self.api_key)
            .header("Content-Type", "application/json")
            .timeout(Duration::from_secs(15));
        for (k, v) in &self.custom_headers {
            req_builder = req_builder.header(k.as_str(), v.as_str());
        }
        let result = req_builder.json(&payload).send().await;
        match result {
            Ok(resp) if !resp.status().is_success() => {
                warn!("gemini warmup got HTTP {} (non-fatal)", resp.status());
            }
            Ok(_) => info!(
                "gemini provider warmed up in {}ms",
                start.elapsed().as_millis()
            ),
            Err(e) => warn!("gemini warmup request failed (non-fatal): {}", e),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
