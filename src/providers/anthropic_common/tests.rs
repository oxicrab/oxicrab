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

// --- parse_response tests ---

#[test]
fn test_parse_response_text_only() {
    let json = json!({
        "content": [{"type": "text", "text": "Hello world"}],
        "usage": {"input_tokens": 10, "output_tokens": 5}
    });
    let resp = parse_response(&json);
    assert_eq!(resp.content.as_deref(), Some("Hello world"));
    assert!(resp.tool_calls.is_empty());
    assert_eq!(resp.input_tokens, Some(10));
    assert_eq!(resp.output_tokens, Some(5));
    assert!(resp.reasoning_content.is_none());
}

#[test]
fn test_parse_response_tool_use() {
    let json = json!({
        "content": [
            {"type": "tool_use", "id": "tc_1", "name": "read_file", "input": {"path": "/tmp/test.txt"}}
        ]
    });
    let resp = parse_response(&json);
    assert!(resp.content.is_none());
    assert_eq!(resp.tool_calls.len(), 1);
    assert_eq!(resp.tool_calls[0].id, "tc_1");
    assert_eq!(resp.tool_calls[0].name, "read_file");
    assert_eq!(resp.tool_calls[0].arguments["path"], "/tmp/test.txt");
}

#[test]
fn test_parse_response_text_and_tool_use() {
    let json = json!({
        "content": [
            {"type": "text", "text": "Let me read that file."},
            {"type": "tool_use", "id": "tc_2", "name": "exec", "input": {"cmd": "ls"}}
        ]
    });
    let resp = parse_response(&json);
    assert_eq!(resp.content.as_deref(), Some("Let me read that file."));
    assert_eq!(resp.tool_calls.len(), 1);
    assert_eq!(resp.tool_calls[0].name, "exec");
}

#[test]
fn test_parse_response_thinking_block() {
    let json = json!({
        "content": [
            {"type": "thinking", "text": "Let me think about this..."},
            {"type": "text", "text": "The answer is 42."}
        ]
    });
    let resp = parse_response(&json);
    assert_eq!(resp.content.as_deref(), Some("The answer is 42."));
    assert_eq!(
        resp.reasoning_content.as_deref(),
        Some("Let me think about this...")
    );
}

#[test]
fn test_parse_response_no_usage() {
    let json = json!({"content": [{"type": "text", "text": "hi"}]});
    let resp = parse_response(&json);
    assert!(resp.input_tokens.is_none());
    assert!(resp.output_tokens.is_none());
}

#[test]
fn test_parse_response_empty_content() {
    let json = json!({"content": []});
    let resp = parse_response(&json);
    assert!(resp.content.is_none());
    assert!(resp.tool_calls.is_empty());
    assert!(resp.reasoning_content.is_none());
}

#[test]
fn test_parse_response_missing_tool_use_fields() {
    let json = json!({
        "content": [{"type": "tool_use"}]
    });
    let resp = parse_response(&json);
    assert_eq!(resp.tool_calls.len(), 1);
    assert_eq!(resp.tool_calls[0].id, "");
    assert_eq!(resp.tool_calls[0].name, "");
    assert_eq!(resp.tool_calls[0].arguments, json!({}));
}

#[test]
fn test_parse_response_multiple_tool_calls() {
    let json = json!({
        "content": [
            {"type": "tool_use", "id": "t1", "name": "a", "input": {}},
            {"type": "tool_use", "id": "t2", "name": "b", "input": {"x": 1}}
        ]
    });
    let resp = parse_response(&json);
    assert_eq!(resp.tool_calls.len(), 2);
    assert_eq!(resp.tool_calls[0].id, "t1");
    assert_eq!(resp.tool_calls[1].id, "t2");
}

// --- convert_tools tests ---

#[test]
fn test_convert_tools_basic() {
    let tools = vec![ToolDefinition {
        name: "my_tool".to_string(),
        description: "Does stuff".to_string(),
        parameters: json!({"type": "object", "properties": {}}),
    }];
    let result = convert_tools(tools);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0]["name"], "my_tool");
    assert_eq!(result[0]["description"], "Does stuff");
    assert_eq!(
        result[0]["input_schema"],
        json!({"type": "object", "properties": {}})
    );
}

#[test]
fn test_convert_tools_empty() {
    let result = convert_tools(vec![]);
    assert!(result.is_empty());
}

// --- convert_messages edge cases ---

#[test]
fn test_convert_system_messages_joined() {
    let messages = vec![
        Message::system("rule 1"),
        Message::system("rule 2"),
        Message::user("hello"),
    ];
    let (system, msgs) = convert_messages(messages);
    assert_eq!(system.as_deref(), Some("rule 1\n\nrule 2"));
    assert_eq!(msgs.len(), 1);
}

#[test]
fn test_convert_tool_result_with_error() {
    let messages = vec![Message::tool_result(
        "tc1".to_string(),
        "something failed".to_string(),
        true,
    )];
    let (_, result) = convert_messages(messages);
    assert_eq!(result.len(), 1);
    let content = result[0].content.as_array().unwrap();
    assert_eq!(content[0]["is_error"], true);
}

#[test]
fn test_convert_assistant_empty_content_omitted() {
    let messages = vec![Message::assistant(
        "",
        Some(vec![ToolCallRequest {
            id: "tc1".to_string(),
            name: "tool".to_string(),
            arguments: json!({}),
        }]),
    )];
    let (_, result) = convert_messages(messages);
    let content = result[0].content.as_array().unwrap();
    // Empty text should be omitted, only tool_use block present
    assert_eq!(content.len(), 1);
    assert_eq!(content[0]["type"], "tool_use");
}

// --- prompt caching tests ---

#[test]
fn test_convert_tools_last_has_cache_control() {
    let tools = vec![
        ToolDefinition {
            name: "tool_a".to_string(),
            description: "First tool".to_string(),
            parameters: json!({"type": "object"}),
        },
        ToolDefinition {
            name: "tool_b".to_string(),
            description: "Second tool".to_string(),
            parameters: json!({"type": "object"}),
        },
    ];
    let result = convert_tools(tools);
    assert_eq!(result.len(), 2);
    // First tool should NOT have cache_control
    assert!(result[0].get("cache_control").is_none());
    // Last tool should have cache_control
    assert_eq!(result[1]["cache_control"], json!({"type": "ephemeral"}));
}

#[test]
fn test_convert_tools_single_has_cache_control() {
    let tools = vec![ToolDefinition {
        name: "only_tool".to_string(),
        description: "The only tool".to_string(),
        parameters: json!({"type": "object"}),
    }];
    let result = convert_tools(tools);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0]["cache_control"], json!({"type": "ephemeral"}));
}

#[test]
fn test_system_to_content_blocks_has_cache_control() {
    let blocks = system_to_content_blocks("You are a helpful assistant.");
    let arr = blocks.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["type"], "text");
    assert_eq!(arr[0]["text"], "You are a helpful assistant.");
    assert_eq!(arr[0]["cache_control"], json!({"type": "ephemeral"}));
}

#[test]
fn test_parse_response_cache_tokens() {
    let json = json!({
        "content": [{"type": "text", "text": "hello"}],
        "usage": {
            "input_tokens": 100,
            "output_tokens": 50,
            "cache_creation_input_tokens": 80,
            "cache_read_input_tokens": 20
        }
    });
    let resp = parse_response(&json);
    assert_eq!(resp.input_tokens, Some(100));
    assert_eq!(resp.output_tokens, Some(50));
    assert_eq!(resp.cache_creation_input_tokens, Some(80));
    assert_eq!(resp.cache_read_input_tokens, Some(20));
}

#[test]
fn test_parse_response_no_cache_tokens() {
    let json = json!({
        "content": [{"type": "text", "text": "hello"}],
        "usage": {
            "input_tokens": 100,
            "output_tokens": 50
        }
    });
    let resp = parse_response(&json);
    assert!(resp.cache_creation_input_tokens.is_none());
    assert!(resp.cache_read_input_tokens.is_none());
}
