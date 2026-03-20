use oxicrab_core::providers::base::{LLMResponse, Message, ToolCallRequest, ToolDefinition};
use serde::Serialize;
use serde_json::{Value, json};
use tracing::warn;

#[derive(Debug, Serialize)]
pub struct AnthropicMessage {
    pub role: String,
    pub content: Value,
}

#[derive(Debug, Serialize)]
pub struct AnthropicTool {
    pub name: String,
    pub description: String,
    #[serde(rename = "input_schema")]
    pub input_schema: Value,
}

/// Convert generic messages to Anthropic API format.
/// Returns (`system_prompt`, `anthropic_messages`).
pub fn convert_messages(messages: &[Message]) -> (Option<String>, Vec<AnthropicMessage>) {
    let mut system_parts = Vec::new();
    let mut anthropic_messages = Vec::new();

    for msg in messages {
        match msg.role.as_str() {
            "system" => {
                system_parts.push(msg.content.clone());
            }
            "user" => {
                let content = if msg.images.is_empty() {
                    Value::String(msg.content.clone())
                } else {
                    let mut parts = Vec::new();
                    if !msg.content.is_empty() {
                        parts.push(json!({
                            "type": "text",
                            "text": msg.content
                        }));
                    }
                    for img in &msg.images {
                        let block_type = if img.media_type.starts_with("image/") {
                            "image"
                        } else {
                            "document"
                        };
                        parts.push(json!({
                            "type": block_type,
                            "source": {
                                "type": "base64",
                                "media_type": img.media_type,
                                "data": img.data
                            }
                        }));
                    }
                    Value::Array(parts)
                };
                anthropic_messages.push(AnthropicMessage {
                    role: "user".to_string(),
                    content,
                });
            }
            "assistant" => {
                let mut content: Vec<Value> = Vec::new();

                // Replay thinking block before text/tool_use blocks.
                // Only emit when we have a signature — the Anthropic API rejects
                // thinking blocks without a signature.
                if let Some(ref thinking) = msg.reasoning_content
                    && !thinking.is_empty()
                    && let Some(ref sig) = msg.reasoning_signature
                {
                    content.push(json!({
                        "type": "thinking",
                        "thinking": thinking,
                        "signature": sig
                    }));
                }

                // Replay any redacted_thinking blocks (opaque, must be sent back verbatim)
                if let Some(ref blocks) = msg.redacted_thinking_blocks {
                    for data in blocks {
                        content.push(json!({
                            "type": "redacted_thinking",
                            "data": data
                        }));
                    }
                }

                // Only include text block if content is non-empty
                // (Anthropic API rejects empty text content blocks)
                if !msg.content.is_empty() {
                    content.push(json!({
                        "type": "text",
                        "text": msg.content
                    }));
                }

                if let Some(ref tool_calls) = msg.tool_calls {
                    for tc in tool_calls {
                        content.push(json!({
                            "type": "tool_use",
                            "id": tc.id,
                            "name": tc.name,
                            "input": tc.arguments
                        }));
                    }
                }

                // Skip assistant messages with no content (Anthropic rejects empty arrays)
                if !content.is_empty() {
                    anthropic_messages.push(AnthropicMessage {
                        role: "assistant".to_string(),
                        content: Value::Array(content),
                    });
                }
            }
            "tool" => {
                if let Some(ref tool_call_id) = msg.tool_call_id {
                    let mut result = json!({
                        "type": "tool_result",
                        "tool_use_id": tool_call_id,
                        "content": msg.content
                    });
                    if msg.is_error {
                        result["is_error"] = json!(true);
                    }
                    anthropic_messages.push(AnthropicMessage {
                        role: "user".to_string(),
                        content: Value::Array(vec![result]),
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

    // Merge consecutive user messages (Anthropic API rejects consecutive same-role messages).
    // This happens when multiple tool results appear in a row since each becomes role: "user".
    let mut merged: Vec<AnthropicMessage> = Vec::new();
    for msg in anthropic_messages {
        if let Some(last) = merged.last_mut()
            && last.role == "user"
            && msg.role == "user"
        {
            let existing = match &last.content {
                Value::Array(arr) => arr.clone(),
                Value::String(s) => vec![json!({"type": "text", "text": s})],
                other => vec![other.clone()],
            };
            let new_items = match &msg.content {
                Value::Array(arr) => arr.clone(),
                Value::String(s) => vec![json!({"type": "text", "text": s})],
                other => vec![other.clone()],
            };
            let mut combined = existing;
            combined.extend(new_items);
            last.content = Value::Array(combined);
            continue;
        }
        merged.push(msg);
    }

    (system, merged)
}

/// Convert generic tool definitions to Anthropic API format.
/// Adds `cache_control` to the last tool for prompt caching.
pub fn convert_tools(tools: &[ToolDefinition]) -> Vec<Value> {
    let len = tools.len();
    tools
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let mut tool = json!({
                "name": &t.name,
                "description": &t.description,
                "input_schema": &t.parameters,
            });
            // Add cache_control to the last tool definition for prompt caching
            if i == len - 1 {
                tool["cache_control"] = json!({"type": "ephemeral"});
            }
            tool
        })
        .collect()
}

/// Convert a system prompt string into Anthropic content blocks with `cache_control`
/// on the last block for prompt caching.
pub fn system_to_content_blocks(system: &str) -> Value {
    json!([{
        "type": "text",
        "text": system,
        "cache_control": {"type": "ephemeral"}
    }])
}

/// Parse an Anthropic API response into a generic [`LLMResponse`].
pub fn parse_response(json: &Value) -> LLMResponse {
    let content = json["content"].as_array().and_then(|arr| {
        let texts: Vec<&str> = arr
            .iter()
            .filter(|block| block["type"] == "text")
            .filter_map(|block| block["text"].as_str())
            .collect();
        if texts.is_empty() {
            None
        } else {
            Some(texts.join("\n\n"))
        }
    });

    let mut tool_calls = Vec::new();
    let mut reasoning_content: Option<String> = None;
    let mut reasoning_signature = None;
    let mut redacted_thinking_blocks: Vec<String> = Vec::new();

    if let Some(content_array) = json["content"].as_array() {
        for block in content_array {
            match block["type"].as_str() {
                Some("tool_use") => {
                    let name = block["name"].as_str().unwrap_or_default().to_string();
                    if name.is_empty() {
                        warn!("skipping tool_use block with empty name");
                        continue;
                    }
                    tool_calls.push(ToolCallRequest {
                        id: block["id"].as_str().unwrap_or_default().to_string(),
                        name,
                        arguments: block.get("input").cloned().unwrap_or(json!({})),
                    });
                }
                Some("thinking") => {
                    // Keep only the last thinking block — its signature is authoritative
                    // and concatenating multiple blocks creates a signature mismatch.
                    if let Some(thought) = block["thinking"]
                        .as_str()
                        .or_else(|| block["text"].as_str())
                    {
                        reasoning_content = Some(thought.to_string());
                    }
                    if let Some(sig) = block["signature"].as_str() {
                        reasoning_signature = Some(sig.to_string());
                    }
                }
                Some("redacted_thinking") => {
                    // Opaque redacted thinking blocks must be captured and replayed
                    // verbatim — the API rejects responses that drop them.
                    if let Some(data) = block["data"].as_str() {
                        redacted_thinking_blocks.push(data.to_string());
                    }
                }
                _ => {}
            }
        }
    }

    let usage = json.get("usage");

    let input_tokens = usage
        .and_then(|u| u.get("input_tokens"))
        .and_then(serde_json::Value::as_u64);

    let output_tokens = usage
        .and_then(|u| u.get("output_tokens"))
        .and_then(serde_json::Value::as_u64);

    let cache_creation_input_tokens = usage
        .and_then(|u| u.get("cache_creation_input_tokens"))
        .and_then(serde_json::Value::as_u64);

    let cache_read_input_tokens = usage
        .and_then(|u| u.get("cache_read_input_tokens"))
        .and_then(serde_json::Value::as_u64);

    let finish_reason = json["stop_reason"]
        .as_str()
        .map(std::string::ToString::to_string);

    let redacted = if redacted_thinking_blocks.is_empty() {
        None
    } else {
        Some(redacted_thinking_blocks)
    };

    LLMResponse {
        content,
        tool_calls,
        reasoning_content,
        reasoning_signature,
        redacted_thinking_blocks: redacted,
        input_tokens,
        output_tokens,
        cache_creation_input_tokens,
        cache_read_input_tokens,
        finish_reason,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests;
