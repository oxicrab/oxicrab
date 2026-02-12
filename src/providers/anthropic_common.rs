use crate::providers::base::{LLMResponse, Message, ToolCallRequest, ToolDefinition};
use serde::Serialize;
use serde_json::{json, Value};

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
                anthropic_messages.push(AnthropicMessage {
                    role: "user".to_string(),
                    content: Value::String(msg.content),
                });
            }
            "assistant" => {
                let mut content: Vec<Value> = Vec::new();

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

    (system, anthropic_messages)
}

/// Convert generic tool definitions to Anthropic API format.
pub fn convert_tools(tools: Vec<ToolDefinition>) -> Vec<AnthropicTool> {
    tools
        .into_iter()
        .map(|t| AnthropicTool {
            name: t.name,
            description: t.description,
            input_schema: t.parameters,
        })
        .collect()
}

/// Parse an Anthropic API response into a generic [`LLMResponse`].
pub fn parse_response(json: &Value) -> LLMResponse {
    let content = json["content"].as_array().and_then(|arr| {
        arr.iter().find_map(|block| {
            if block["type"] == "text" {
                block["text"].as_str().map(|s| s.to_string())
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
                    reasoning_content = block["thinking"].as_str().map(|s| s.to_string());
                }
                _ => {}
            }
        }
    }

    let input_tokens = json
        .get("usage")
        .and_then(|u| u.get("input_tokens"))
        .and_then(|t| t.as_u64());

    LLMResponse {
        content,
        tool_calls,
        reasoning_content,
        input_tokens,
    }
}
