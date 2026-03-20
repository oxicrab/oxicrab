use super::*;

#[test]
fn message_assistant_with_tool_calls() {
    let tc = vec![ToolCallRequest {
        id: "tc1".into(),
        name: "weather".into(),
        arguments: serde_json::json!({"city": "NYC"}),
    }];
    let msg = Message::assistant("thinking", Some(tc));
    assert_eq!(msg.role, "assistant");
    assert_eq!(msg.content, "thinking");
    assert_eq!(msg.tool_calls.as_ref().unwrap().len(), 1);
}

#[test]
fn message_tool_result() {
    let msg = Message::tool_result("tc1", "result data", false);
    assert_eq!(msg.role, "tool");
    assert_eq!(msg.content, "result data");
    assert_eq!(msg.tool_call_id.as_deref(), Some("tc1"));
    assert!(!msg.is_error);
}

#[test]
fn llm_response_has_tool_calls() {
    let empty = LLMResponse {
        content: Some("hi".into()),
        ..Default::default()
    };
    assert!(!empty.has_tool_calls());

    let with_tools = LLMResponse {
        tool_calls: vec![ToolCallRequest {
            id: "1".into(),
            name: "test".into(),
            arguments: Value::Null,
        }],
        ..Default::default()
    };
    assert!(with_tools.has_tool_calls());
}

#[test]
fn message_assistant_with_thinking() {
    let msg = Message::assistant_with_thinking(
        "answer",
        None,
        Some("reasoning...".to_string()),
        Some("sig123".to_string()),
    );
    assert_eq!(msg.role, "assistant");
    assert_eq!(msg.content, "answer");
    assert_eq!(msg.reasoning_content.as_deref(), Some("reasoning..."));
    assert_eq!(msg.reasoning_signature.as_deref(), Some("sig123"));
    assert!(msg.tool_calls.is_none());
}

#[test]
fn message_assistant_with_thinking_none() {
    let msg = Message::assistant_with_thinking("answer", None, None, None);
    assert!(msg.reasoning_content.is_none());
}

#[test]
fn test_chat_request_builder_all_setters() {
    let tools = vec![ToolDefinition {
        name: "test".into(),
        description: "desc".into(),
        parameters: serde_json::json!({}),
    }];
    let req = ChatRequest::builder(vec![Message::user("hi")], 200)
        .model("gpt-4")
        .temperature(0.5)
        .tools(tools)
        .tool_choice("auto")
        .response_format(ResponseFormat::JsonObject)
        .build();
    assert_eq!(req.model.as_deref(), Some("gpt-4"));
    assert_eq!(req.temperature, Some(0.5));
    assert_eq!(req.tools.as_ref().unwrap().len(), 1);
    assert_eq!(req.tool_choice.as_deref(), Some("auto"));
    assert!(matches!(
        req.response_format,
        Some(ResponseFormat::JsonObject)
    ));
}
