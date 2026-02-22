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
        tool_calls: vec![],
        reasoning_content: None,
        input_tokens: None,
        output_tokens: None,
        cache_creation_input_tokens: None,
        cache_read_input_tokens: None,
    };
    assert!(!empty.has_tool_calls());

    let with_tools = LLMResponse {
        content: None,
        tool_calls: vec![ToolCallRequest {
            id: "1".into(),
            name: "test".into(),
            arguments: Value::Null,
        }],
        reasoning_content: None,
        input_tokens: None,
        output_tokens: None,
        cache_creation_input_tokens: None,
        cache_read_input_tokens: None,
    };
    assert!(with_tools.has_tool_calls());
}

struct NoopProvider;

#[async_trait]
impl LLMProvider for NoopProvider {
    async fn chat(&self, _req: ChatRequest<'_>) -> anyhow::Result<LLMResponse> {
        Ok(LLMResponse {
            content: Some("ok".into()),
            tool_calls: vec![],
            reasoning_content: None,
            input_tokens: None,
            output_tokens: None,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
        })
    }
    fn default_model(&self) -> &'static str {
        "noop"
    }
}

#[tokio::test]
async fn default_warmup_returns_ok() {
    let provider = NoopProvider;
    assert!(provider.warmup().await.is_ok());
}

#[test]
fn message_assistant_with_thinking() {
    let msg = Message::assistant_with_thinking("answer", None, Some("reasoning...".to_string()));
    assert_eq!(msg.role, "assistant");
    assert_eq!(msg.content, "answer");
    assert_eq!(msg.reasoning_content.as_deref(), Some("reasoning..."));
    assert!(msg.tool_calls.is_none());
}

#[test]
fn message_assistant_with_thinking_none() {
    let msg = Message::assistant_with_thinking("answer", None, None);
    assert!(msg.reasoning_content.is_none());
}

#[test]
fn message_assistant_default_has_no_reasoning() {
    let msg = Message::assistant("answer", None);
    assert!(msg.reasoning_content.is_none());
}

#[test]
fn response_format_json_object() {
    let fmt = ResponseFormat::JsonObject;
    assert!(matches!(fmt, ResponseFormat::JsonObject));
}

#[test]
fn response_format_json_schema() {
    let fmt = ResponseFormat::JsonSchema {
        name: "person".into(),
        schema: serde_json::json!({"type": "object"}),
    };
    match fmt {
        ResponseFormat::JsonSchema { name, schema } => {
            assert_eq!(name, "person");
            assert_eq!(schema["type"], "object");
        }
        ResponseFormat::JsonObject => panic!("expected JsonSchema"),
    }
}
