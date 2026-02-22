use super::*;
use crate::providers::base::{ChatRequest, LLMResponse, Message, ToolCallRequest, ToolDefinition};
use serde_json::json;

fn web_search_tool() -> ToolDefinition {
    ToolDefinition {
        name: "web_search".into(),
        description: "Search the web for information".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of results"
                }
            },
            "required": ["query"]
        }),
    }
}

fn read_file_tool() -> ToolDefinition {
    ToolDefinition {
        name: "read_file".into(),
        description: "Read a file from the filesystem".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute path to the file"
                }
            },
            "required": ["path"]
        }),
    }
}

// --- render_tool_definitions tests ---

#[test]
fn render_tool_definitions_format() {
    let tools = vec![web_search_tool(), read_file_tool()];
    let rendered = render_tool_definitions(&tools);

    assert!(rendered.contains("## Available Tools"));
    assert!(rendered.contains("<tool_call>"));
    assert!(rendered.contains("**web_search** - Search the web for information"));
    assert!(rendered.contains("**read_file** - Read a file from the filesystem"));
    assert!(rendered.contains("- query (string, required): The search query"));
    assert!(rendered.contains("- max_results (integer, optional): Maximum number of results"));
    assert!(rendered.contains("- path (string, required): Absolute path to the file"));
}

#[test]
fn render_parameters_types_and_enum() {
    let schema = json!({
        "type": "object",
        "properties": {
            "name": { "type": "string", "description": "The name" },
            "count": { "type": "integer" },
            "enabled": { "type": "boolean" },
            "tags": { "type": "array", "description": "Tag list" },
            "mode": {
                "type": "string",
                "description": "The mode",
                "enum": ["fast", "slow"]
            }
        },
        "required": ["name"]
    });

    let rendered = render_parameters(&schema);
    assert!(rendered.contains("- name (string, required): The name"));
    assert!(rendered.contains("- count (integer, optional)"));
    assert!(rendered.contains("- enabled (boolean, optional)"));
    assert!(rendered.contains("- tags (array, optional): Tag list"));
    assert!(rendered.contains("- mode (string, optional): The mode [fast, slow]"));
}

// --- parse_tool_calls_from_text tests ---

#[test]
fn parse_xml_single_tool_call() {
    let text = r#"<tool_call>
{"name": "web_search", "arguments": {"query": "rust async"}}
</tool_call>"#;

    let (calls, remaining) = parse_tool_calls_from_text(text);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "web_search");
    assert_eq!(calls[0].arguments["query"], "rust async");
    assert_eq!(calls[0].id, "prompt_tc_1");
    assert!(remaining.is_none());
}

#[test]
fn parse_xml_multiple_tool_calls() {
    let text = r#"Let me search for that.

<tool_call>
{"name": "web_search", "arguments": {"query": "rust async"}}
</tool_call>

<tool_call>
{"name": "read_file", "arguments": {"path": "/tmp/test.rs"}}
</tool_call>"#;

    let (calls, remaining) = parse_tool_calls_from_text(text);
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].name, "web_search");
    assert_eq!(calls[0].id, "prompt_tc_1");
    assert_eq!(calls[1].name, "read_file");
    assert_eq!(calls[1].id, "prompt_tc_2");
    assert_eq!(remaining.as_deref(), Some("Let me search for that."));
}

#[test]
fn parse_json_block_tool_calls() {
    let text = "Here's what I'll do:\n\n```json\n{\"name\": \"web_search\", \"arguments\": {\"query\": \"test\"}}\n```";

    let (calls, remaining) = parse_tool_calls_from_text(text);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "web_search");
    assert_eq!(remaining.as_deref(), Some("Here's what I'll do:"));
}

#[test]
fn parse_mixed_text_and_calls() {
    let text = "I'll search for that.\n\n<tool_call>\n{\"name\": \"web_search\", \"arguments\": {\"query\": \"hello\"}}\n</tool_call>\n\nDone.";

    let (calls, remaining) = parse_tool_calls_from_text(text);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "web_search");
    let rem = remaining.unwrap();
    assert!(rem.contains("I'll search for that."));
    assert!(rem.contains("Done."));
}

#[test]
fn parse_malformed_json_skipped() {
    let text = "<tool_call>\n{not valid json}\n</tool_call>\n\n<tool_call>\n{\"name\": \"web_search\", \"arguments\": {}}\n</tool_call>";

    let (calls, _remaining) = parse_tool_calls_from_text(text);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "web_search");
}

#[test]
fn parse_no_tool_calls_returns_empty() {
    let text = "Just a regular response with no tool calls.";

    let (calls, remaining) = parse_tool_calls_from_text(text);
    assert!(calls.is_empty());
    assert_eq!(remaining.as_deref(), Some(text));
}

// --- rewrite_request tests ---

#[test]
fn rewrite_request_moves_tools_to_system_prompt() {
    let tools = vec![web_search_tool()];
    let req = ChatRequest {
        messages: vec![Message::system("You are a helpful assistant.")],
        tools: Some(tools),
        model: None,
        max_tokens: 1024,
        temperature: 0.7,
        tool_choice: None,
        response_format: None,
    };

    let rewritten = PromptGuidedToolsProvider::rewrite_request(req);
    assert!(rewritten.tools.is_none());
    assert!(rewritten.tool_choice.is_none());
    assert!(rewritten.messages[0].content.contains("## Available Tools"));
    assert!(rewritten.messages[0].content.contains("**web_search**"));
}

#[test]
fn rewrite_request_tool_result_to_user_message() {
    let req = ChatRequest {
        messages: vec![
            Message::system("You are helpful."),
            Message::user("search for rust"),
            Message::tool_result("tc_1", "Found 10 results", false),
        ],
        tools: Some(vec![web_search_tool()]),
        model: None,
        max_tokens: 1024,
        temperature: 0.7,
        tool_choice: None,
        response_format: None,
    };

    let rewritten = PromptGuidedToolsProvider::rewrite_request(req);
    // tool result should be converted to user message
    let tool_msg = &rewritten.messages[2];
    assert_eq!(tool_msg.role, "user");
    assert!(tool_msg.content.contains("[Tool result for tc_1]"));
    assert!(tool_msg.content.contains("Found 10 results"));
}

#[test]
fn rewrite_request_assistant_tool_calls_to_inline_text() {
    let tool_calls = vec![ToolCallRequest {
        id: "tc_1".into(),
        name: "web_search".into(),
        arguments: json!({"query": "rust"}),
    }];

    let req = ChatRequest {
        messages: vec![
            Message::system("You are helpful."),
            Message::assistant("Let me search.", Some(tool_calls)),
        ],
        tools: Some(vec![web_search_tool()]),
        model: None,
        max_tokens: 1024,
        temperature: 0.7,
        tool_choice: None,
        response_format: None,
    };

    let rewritten = PromptGuidedToolsProvider::rewrite_request(req);
    let assistant_msg = &rewritten.messages[1];
    assert_eq!(assistant_msg.role, "assistant");
    assert!(assistant_msg.content.contains("<tool_call>"));
    assert!(assistant_msg.content.contains("web_search"));
    assert!(assistant_msg.tool_calls.is_none());
}

#[test]
fn rewrite_request_tool_choice_any_adds_force_instruction() {
    let req = ChatRequest {
        messages: vec![Message::system("You are helpful.")],
        tools: Some(vec![web_search_tool()]),
        model: None,
        max_tokens: 1024,
        temperature: 0.7,
        tool_choice: Some("any".into()),
        response_format: None,
    };

    let rewritten = PromptGuidedToolsProvider::rewrite_request(req);
    assert!(
        rewritten.messages[0]
            .content
            .contains("You MUST respond by calling at least one tool")
    );
}

// --- Integration test with MockProvider ---

#[tokio::test]
async fn integration_text_tool_call_parsed() {
    let inner_response = LLMResponse {
        content: Some(
            "I'll search for that.\n\n<tool_call>\n\
             {\"name\": \"web_search\", \"arguments\": {\"query\": \"rust async\"}}\n\
             </tool_call>"
                .into(),
        ),
        tool_calls: vec![],
        reasoning_content: None,
        input_tokens: None,
        output_tokens: None,
        cache_creation_input_tokens: None,
        cache_read_input_tokens: None,
    };

    let inner = Arc::new(MockProvider::with_response(inner_response));
    let provider = PromptGuidedToolsProvider::wrap(inner);

    let req = ChatRequest {
        messages: vec![
            Message::system("You are helpful."),
            Message::user("search for rust async"),
        ],
        tools: Some(vec![web_search_tool()]),
        model: None,
        max_tokens: 1024,
        temperature: 0.7,
        tool_choice: None,
        response_format: None,
    };

    let response = provider.chat(req).await.unwrap();
    assert_eq!(response.tool_calls.len(), 1);
    assert_eq!(response.tool_calls[0].name, "web_search");
    assert_eq!(response.tool_calls[0].arguments["query"], "rust async");
    // Remaining text preserved
    assert_eq!(response.content.as_deref(), Some("I'll search for that."));
}

#[tokio::test]
async fn passthrough_when_no_tools() {
    let inner = Arc::new(MockProvider::text("just text"));
    let provider = PromptGuidedToolsProvider::wrap(inner);

    let req = ChatRequest {
        messages: vec![Message::user("hello")],
        tools: None,
        model: None,
        max_tokens: 1024,
        temperature: 0.7,
        tool_choice: None,
        response_format: None,
    };

    let response = provider.chat(req).await.unwrap();
    assert_eq!(response.content.as_deref(), Some("just text"));
    assert!(response.tool_calls.is_empty());
}

// --- Mock provider ---

struct MockProvider {
    response: LLMResponse,
}

impl MockProvider {
    fn text(text: &str) -> Self {
        Self {
            response: LLMResponse {
                content: Some(text.into()),
                tool_calls: vec![],
                reasoning_content: None,
                input_tokens: None,
                output_tokens: None,
                cache_creation_input_tokens: None,
                cache_read_input_tokens: None,
            },
        }
    }

    fn with_response(response: LLMResponse) -> Self {
        Self { response }
    }
}

#[async_trait]
impl LLMProvider for MockProvider {
    async fn chat(&self, _req: ChatRequest<'_>) -> anyhow::Result<LLMResponse> {
        Ok(self.response.clone())
    }

    fn default_model(&self) -> &'static str {
        "mock-model"
    }
}
