use crate::errors::ProviderErrorHandler;
use crate::{API_URL_OPENAI, provider_http_client};
use anyhow::{Context, Result};
use async_trait::async_trait;
use oxicrab_core::providers::base::{ChatRequest, LLMProvider, LLMResponse, ToolCallRequest};
use reqwest::Client;
use serde_json::{Value, json};
use tracing::{debug, warn};

pub struct OpenAIProvider {
    api_key: String,
    default_model: String,
    base_url: String,
    provider_name: String,
    client: Client,
    custom_headers: std::collections::HashMap<String, String>,
}

impl OpenAIProvider {
    pub fn new(api_key: String, default_model: Option<String>) -> Self {
        Self {
            api_key,
            default_model: default_model.unwrap_or_else(|| "gpt-4o".to_string()),
            base_url: API_URL_OPENAI.to_string(),
            provider_name: "OpenAI".to_string(),
            client: provider_http_client(),
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
                    // OpenAI canonical: arguments is a JSON-encoded
                    // string. Some Azure / proxy gateways emit an
                    // object instead, so accept both — falling
                    // through to `json!({})` would silently run the
                    // tool with empty args.
                    let arguments = match function.get("arguments") {
                        Some(serde_json::Value::String(s)) => match serde_json::from_str(s) {
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
                        Some(value @ serde_json::Value::Object(_)) => value.clone(),
                        Some(serde_json::Value::Null) | None => json!({}),
                        Some(other) => {
                            warn!(
                                "skipping tool call '{}': unsupported arguments type ({:?})",
                                function["name"].as_str().unwrap_or("unknown"),
                                other
                            );
                            continue;
                        }
                    };

                    let name = function["name"].as_str().unwrap_or_default().to_string();
                    if name.is_empty() {
                        warn!("skipping tool call with empty name");
                        continue;
                    }
                    let id = tc["id"].as_str().unwrap_or_default().to_string();
                    if id.is_empty() {
                        // tool_call_id is the dedup key for reflection
                        // outcome write-back, trajectory logging, and
                        // tool_call_id linkage in `convert_messages`.
                        // Multiple `""` ids would collide and
                        // cross-credit reflection outcomes (same as
                        // the Anthropic empty-id guard).
                        warn!("skipping tool call with empty id (name={name})");
                        continue;
                    }
                    tool_calls.push(ToolCallRequest {
                        id,
                        name,
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

        // OpenAI automatic prompt caching (gpt-4o, gpt-4.1, o-series) reports
        // cache hits via `usage.prompt_tokens_details.cached_tokens`. Surface
        // it so cost tracking reflects real token consumption.
        let cache_read_input_tokens = json
            .get("usage")
            .and_then(|u| u.get("prompt_tokens_details"))
            .and_then(|d| d.get("cached_tokens"))
            .and_then(serde_json::Value::as_u64);

        // DeepSeek-R1 and similar models return reasoning in this field
        let reasoning_content = message["reasoning_content"]
            .as_str()
            .map(std::string::ToString::to_string);

        let finish_reason = choice["finish_reason"]
            .as_str()
            .map(std::string::ToString::to_string);

        Ok(LLMResponse {
            content,
            tool_calls,
            reasoning_content,
            input_tokens,
            output_tokens,
            cache_read_input_tokens,
            finish_reason,
            ..Default::default()
        })
    }
}

#[async_trait]
impl LLMProvider for OpenAIProvider {
    async fn chat(&self, req: &ChatRequest) -> Result<LLMResponse> {
        debug!(
            "{} chat: model={}",
            self.provider_name,
            req.model.as_deref().unwrap_or(&self.default_model)
        );
        let openai_messages: Vec<Value> = req
            .messages
            .iter()
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

                // Include reasoning_content for thinking models (e.g. kimi-k2.5,
                // DeepSeek-R1) that require it on assistant messages
                if let Some(ref reasoning) = msg.reasoning_content {
                    m["reasoning_content"] = json!(reasoning);
                }

                if let Some(ref tool_calls) = msg.tool_calls {
                    m["tool_calls"] = json!(
                        tool_calls
                            .iter()
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

                // Prefix tool error results so the LLM sees the error semantics
                // (OpenAI's API has no is_error field on tool messages)
                if msg.role == "tool" && msg.is_error {
                    m["content"] = json!(format!("[Error] {}", msg.content));
                }

                if let Some(ref tool_call_id) = msg.tool_call_id {
                    m["tool_call_id"] = json!(tool_call_id);
                }

                m
            })
            .collect();

        let mut payload = json!({
            "model": req.model.as_deref().unwrap_or(&self.default_model),
            "messages": openai_messages,
            "max_tokens": req.max_tokens,
        });
        if let Some(temp) = req.temperature {
            payload["temperature"] = json!(temp);
        }

        if let Some(ref format) = req.response_format {
            match format {
                oxicrab_core::providers::base::ResponseFormat::JsonObject => {
                    payload["response_format"] = json!({"type": "json_object"});
                }
                oxicrab_core::providers::base::ResponseFormat::JsonSchema { name, schema } => {
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

        if let Some(ref tools) = req.tools {
            payload["tools"] = json!(
                tools
                    .iter()
                    .map(|t| json!({
                        "type": "function",
                        "function": {
                            "name": &t.name,
                            "description": &t.description,
                            "parameters": &t.parameters
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

        let req = self
            .client
            .post(&self.base_url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json");
        let req = crate::apply_custom_headers(req, &self.custom_headers);
        let provider_name = &self.provider_name;
        let resp = req
            .json(&payload)
            .send()
            .await
            .with_context(|| format!("Failed to send request to {provider_name} API"))?;

        let json = ProviderErrorHandler::check_response(resp, &self.provider_name).await?;

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

    async fn warmup(&self) -> anyhow::Result<()> {
        let payload = json!({
            "model": self.default_model,
            "messages": [{"role": "user", "content": "hi"}],
            "max_tokens": 1,
        });
        let mut headers = vec![
            ("Authorization", format!("Bearer {}", self.api_key)),
            ("Content-Type", "application/json".to_string()),
        ];
        for (k, v) in &self.custom_headers {
            headers.push((k.as_str(), v.clone()));
        }
        crate::warmup_provider(
            &self.client,
            &self.base_url,
            headers,
            payload,
            &self.provider_name,
        )
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
