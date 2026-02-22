use crate::providers::base::{LLMResponse, Message, ToolCallRequest, ToolDefinition};
use serde::Serialize;
use serde_json::{Value, json};

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
pub fn convert_messages(messages: Vec<Message>) -> (Option<String>, Vec<AnthropicMessage>) {
    let mut system_parts = Vec::new();
    let mut anthropic_messages = Vec::new();

    for msg in messages {
        match msg.role.as_str() {
            "system" => {
                system_parts.push(msg.content);
            }
            "user" => {
                let content = if msg.images.is_empty() {
                    Value::String(msg.content)
                } else {
                    let mut parts = Vec::new();
                    if !msg.content.is_empty() {
                        parts.push(json!({
                            "type": "text",
                            "text": msg.content
                        }));
                    }
                    for img in &msg.images {
                        parts.push(json!({
                            "type": "image",
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

                // Replay thinking block before text/tool_use blocks
                if let Some(ref thinking) = msg.reasoning_content
                    && !thinking.is_empty()
                {
                    content.push(json!({
                        "type": "thinking",
                        "thinking": thinking
                    }));
                }

                // Only include text block if content is non-empty
                // (Anthropic API rejects empty text content blocks)
                if !msg.content.is_empty() {
                    content.push(json!({
                        "type": "text",
                        "text": msg.content
                    }));
                }

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
pub fn convert_tools(tools: Vec<ToolDefinition>) -> Vec<Value> {
    let len = tools.len();
    tools
        .into_iter()
        .enumerate()
        .map(|(i, t)| {
            let mut tool = json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.parameters,
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
        arr.iter().find_map(|block| {
            if block["type"] == "text" {
                block["text"].as_str().map(std::string::ToString::to_string)
            } else {
                None
            }
        })
    });

    let mut tool_calls = Vec::new();
    let mut reasoning_content = None;

    if let Some(content_array) = json["content"].as_array() {
        for block in content_array {
            match block["type"].as_str() {
                Some("tool_use") => {
                    tool_calls.push(ToolCallRequest {
                        id: block["id"].as_str().unwrap_or("").to_string(),
                        name: block["name"].as_str().unwrap_or("").to_string(),
                        arguments: block.get("input").cloned().unwrap_or(json!({})),
                    });
                }
                Some("thinking") => {
                    // Anthropic API uses "thinking" key; some versions use "text"
                    reasoning_content = block["thinking"]
                        .as_str()
                        .or_else(|| block["text"].as_str())
                        .map(std::string::ToString::to_string);
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

    LLMResponse {
        content,
        tool_calls,
        reasoning_content,
        input_tokens,
        output_tokens,
        cache_creation_input_tokens,
        cache_read_input_tokens,
    }
}

#[cfg(test)]
mod tests;
