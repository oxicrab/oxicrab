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
                    reasoning_content =
                        block["text"].as_str().map(std::string::ToString::to_string);
                }
                _ => {}
            }
        }
    }

    let input_tokens = json
        .get("usage")
        .and_then(|u| u.get("input_tokens"))
        .and_then(serde_json::Value::as_u64);

    let output_tokens = json
        .get("usage")
        .and_then(|u| u.get("output_tokens"))
        .and_then(serde_json::Value::as_u64);

    LLMResponse {
        content,
        tool_calls,
        reasoning_content,
        input_tokens,
        output_tokens,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::base::ImageData;

    #[test]
    fn test_convert_user_message_text_only() {
        let messages = vec![Message::user("hello")];
        let (_, result) = convert_messages(messages);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].role, "user");
        // Text-only should be a plain string, not an array
        assert!(result[0].content.is_string());
        assert_eq!(result[0].content.as_str().unwrap(), "hello");
    }

    #[test]
    fn test_convert_user_message_with_images() {
        let msg = Message::user_with_images(
            "describe this",
            vec![ImageData {
                media_type: "image/jpeg".to_string(),
                data: "base64data".to_string(),
            }],
        );
        let (_, result) = convert_messages(vec![msg]);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].role, "user");
        // Should be an array with text + image blocks
        let content = result[0].content.as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "describe this");
        assert_eq!(content[1]["type"], "image");
        assert_eq!(content[1]["source"]["type"], "base64");
        assert_eq!(content[1]["source"]["media_type"], "image/jpeg");
        assert_eq!(content[1]["source"]["data"], "base64data");
    }

    #[test]
    fn test_convert_user_message_with_multiple_images() {
        let msg = Message::user_with_images(
            "compare these",
            vec![
                ImageData {
                    media_type: "image/jpeg".to_string(),
                    data: "jpg_data".to_string(),
                },
                ImageData {
                    media_type: "image/png".to_string(),
                    data: "png_data".to_string(),
                },
            ],
        );
        let (_, result) = convert_messages(vec![msg]);

        let content = result[0].content.as_array().unwrap();
        assert_eq!(content.len(), 3); // 1 text + 2 images
        assert_eq!(content[1]["source"]["media_type"], "image/jpeg");
        assert_eq!(content[2]["source"]["media_type"], "image/png");
    }

    #[test]
    fn test_convert_user_message_image_with_empty_text() {
        let msg = Message::user_with_images(
            "",
            vec![ImageData {
                media_type: "image/png".to_string(),
                data: "data".to_string(),
            }],
        );
        let (_, result) = convert_messages(vec![msg]);

        let content = result[0].content.as_array().unwrap();
        // Empty text should be omitted, only image block
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "image");
    }

    #[test]
    fn test_convert_mixed_messages_images_only_on_user() {
        let messages = vec![
            Message::system("system prompt"),
            Message::user_with_images(
                "look at this",
                vec![ImageData {
                    media_type: "image/jpeg".to_string(),
                    data: "img".to_string(),
                }],
            ),
            Message::assistant("I see an image", None),
            Message::user("follow up with no image"),
        ];
        let (system, result) = convert_messages(messages);

        assert!(system.is_some());
        assert_eq!(result.len(), 3); // user, assistant, user
        // First user has image (array)
        assert!(result[0].content.is_array());
        // Second user is text-only (string)
        assert!(result[2].content.is_string());
    }

    #[test]
    fn test_consecutive_tool_results_merged() {
        // Two tool results (both become role: "user") should be merged into one user message
        let messages = vec![
            Message::user("do things"),
            Message::assistant(
                "",
                Some(vec![
                    crate::providers::base::ToolCallRequest {
                        id: "tc1".to_string(),
                        name: "tool_a".to_string(),
                        arguments: json!({}),
                    },
                    crate::providers::base::ToolCallRequest {
                        id: "tc2".to_string(),
                        name: "tool_b".to_string(),
                        arguments: json!({}),
                    },
                ]),
            ),
            Message::tool_result("tc1".to_string(), "result1".to_string(), false),
            Message::tool_result("tc2".to_string(), "result2".to_string(), false),
        ];
        let (_, result) = convert_messages(messages);

        // Should be: user, assistant, user (merged tool results)
        assert_eq!(
            result.len(),
            3,
            "consecutive user messages should be merged"
        );
        assert_eq!(result[2].role, "user");
        // Merged content should have 2 tool_result blocks
        let content = result[2].content.as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "tool_result");
        assert_eq!(content[1]["type"], "tool_result");
    }

    #[test]
    fn test_user_after_tool_result_merged() {
        // A user message following a tool result should be merged
        let messages = vec![
            Message::user("first"),
            Message::assistant("ok", None),
            Message::tool_result("tc1".to_string(), "result".to_string(), false),
            Message::user("follow up"),
        ];
        let (_, result) = convert_messages(messages);

        // Should be: user, assistant, user (merged tool_result + text)
        assert_eq!(result.len(), 3);
        let content = result[2].content.as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "tool_result");
        assert_eq!(content[1]["type"], "text");
        assert_eq!(content[1]["text"], "follow up");
    }
}
