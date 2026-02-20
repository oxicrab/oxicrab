use super::*;
use crate::providers::base::{LLMProvider, Message};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// --- Wiremock tests ---

fn simple_chat_request(content: &str) -> ChatRequest<'_> {
    ChatRequest {
        messages: vec![Message::user(content)],
        tools: None,
        model: None,
        max_tokens: 1024,
        temperature: 0.7,
        tool_choice: None,
    }
}

#[tokio::test]
async fn test_chat_success() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/"))
        .and(header("Authorization", "Bearer test_key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "Hello! How can I help?"
                },
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 8, "total_tokens": 18}
        })))
        .mount(&server)
        .await;

    let provider = OpenAIProvider::with_base_url("test_key".to_string(), None, server.uri());
    let result = provider.chat(simple_chat_request("Hi")).await.unwrap();

    assert_eq!(result.content.unwrap(), "Hello! How can I help?");
    assert!(result.tool_calls.is_empty());
}

#[tokio::test]
async fn test_chat_with_tool_calls() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_123",
                        "type": "function",
                        "function": {
                            "name": "weather",
                            "arguments": "{\"city\": \"NYC\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 15, "completion_tokens": 20, "total_tokens": 35}
        })))
        .mount(&server)
        .await;

    let provider = OpenAIProvider::with_base_url("test_key".to_string(), None, server.uri());
    let result = provider
        .chat(simple_chat_request("What's the weather?"))
        .await
        .unwrap();

    assert!(result.has_tool_calls());
    assert_eq!(result.tool_calls[0].name, "weather");
    assert_eq!(result.tool_calls[0].id, "call_123");
    assert_eq!(result.tool_calls[0].arguments["city"], "NYC");
}

#[tokio::test]
async fn test_chat_unauthorized() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": {"type": "authentication_error", "message": "Invalid API key"}
        })))
        .mount(&server)
        .await;

    let provider = OpenAIProvider::with_base_url("bad_key".to_string(), None, server.uri());
    let result = provider.chat(simple_chat_request("Hi")).await;

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("Authentication"), "Error: {}", err);
}

#[tokio::test]
async fn test_chat_rate_limit() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "60")
                .set_body_json(json!({
                    "error": {"type": "rate_limit", "message": "Too many requests"}
                })),
        )
        .mount(&server)
        .await;

    let provider = OpenAIProvider::with_base_url("test_key".to_string(), None, server.uri());
    let result = provider.chat(simple_chat_request("Hi")).await;

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("Rate limit") || err.contains("rate limit"),
        "Error: {}",
        err
    );
}

#[tokio::test]
async fn test_chat_server_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({
            "error": {"type": "server_error", "message": "Internal server error"}
        })))
        .mount(&server)
        .await;

    let provider = OpenAIProvider::with_base_url("test_key".to_string(), None, server.uri());
    let result = provider.chat(simple_chat_request("Hi")).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn test_chat_metrics_updated() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{
                "message": {"role": "assistant", "content": "Hi"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7}
        })))
        .mount(&server)
        .await;

    let provider = OpenAIProvider::with_base_url("test_key".to_string(), None, server.uri());
    provider.chat(simple_chat_request("Hi")).await.unwrap();

    let metrics = provider.metrics.lock().unwrap();
    assert_eq!(metrics.request_count, 1);
    assert_eq!(metrics.token_count, 7);
}

#[tokio::test]
async fn test_chat_custom_model() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{
                "message": {"role": "assistant", "content": "Response from custom model"},
                "finish_reason": "stop"
            }],
            "usage": {"total_tokens": 10}
        })))
        .mount(&server)
        .await;

    let provider = OpenAIProvider::with_base_url(
        "test_key".to_string(),
        Some("gpt-4-turbo".to_string()),
        server.uri(),
    );
    let result = provider.chat(simple_chat_request("Hi")).await.unwrap();

    assert_eq!(result.content.unwrap(), "Response from custom model");
}

#[tokio::test]
async fn test_with_config_custom_provider() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/"))
        .and(header("Authorization", "Bearer deepseek_key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{
                "message": {"role": "assistant", "content": "DeepSeek response"},
                "finish_reason": "stop"
            }],
            "usage": {"total_tokens": 12}
        })))
        .mount(&server)
        .await;

    let provider = OpenAIProvider::with_config(
        "deepseek_key".to_string(),
        "deepseek-chat".to_string(),
        server.uri(),
        "DeepSeek".to_string(),
    );

    assert_eq!(provider.default_model(), "deepseek-chat");

    let result = provider.chat(simple_chat_request("Hi")).await.unwrap();
    assert_eq!(result.content.unwrap(), "DeepSeek response");
}

// --- parse_response unit tests (no network) ---

#[test]
fn test_parse_response_text_only() {
    let json = json!({
        "choices": [{"message": {"content": "hello"}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 5, "completion_tokens": 1}
    });
    let resp = OpenAIProvider::parse_response(&json).unwrap();
    assert_eq!(resp.content.as_deref(), Some("hello"));
    assert!(resp.tool_calls.is_empty());
    assert_eq!(resp.input_tokens, Some(5));
    assert_eq!(resp.output_tokens, Some(1));
}

#[test]
fn test_parse_response_tool_calls() {
    let json = json!({
        "choices": [{"message": {
            "content": null,
            "tool_calls": [{
                "id": "call_1",
                "type": "function",
                "function": {"name": "get_weather", "arguments": "{\"city\":\"London\"}"}
            }]
        }}],
        "usage": {}
    });
    let resp = OpenAIProvider::parse_response(&json).unwrap();
    assert!(resp.content.is_none());
    assert_eq!(resp.tool_calls.len(), 1);
    assert_eq!(resp.tool_calls[0].name, "get_weather");
    assert_eq!(resp.tool_calls[0].arguments["city"], "London");
}

#[test]
fn test_parse_response_no_choices() {
    let json = json!({"choices": []});
    let result = OpenAIProvider::parse_response(&json);
    assert!(result.is_err());
}

#[test]
fn test_parse_response_malformed_tool_arguments() {
    let json = json!({
        "choices": [{"message": {
            "tool_calls": [{
                "id": "call_2",
                "function": {"name": "broken", "arguments": "not-json"}
            }]
        }}],
        "usage": {}
    });
    let resp = OpenAIProvider::parse_response(&json).unwrap();
    // Should fall back to empty object
    assert_eq!(resp.tool_calls[0].arguments, json!({}));
}

#[test]
fn test_parse_response_multiple_tool_calls() {
    let json = json!({
        "choices": [{"message": {
            "tool_calls": [
                {"id": "c1", "function": {"name": "a", "arguments": "{}"}},
                {"id": "c2", "function": {"name": "b", "arguments": "{}"}}
            ]
        }}]
    });
    let resp = OpenAIProvider::parse_response(&json).unwrap();
    assert_eq!(resp.tool_calls.len(), 2);
    assert_eq!(resp.tool_calls[0].id, "c1");
    assert_eq!(resp.tool_calls[1].id, "c2");
}

#[test]
fn test_parse_response_no_usage() {
    let json = json!({
        "choices": [{"message": {"content": "hi"}}]
    });
    let resp = OpenAIProvider::parse_response(&json).unwrap();
    assert!(resp.input_tokens.is_none());
    assert!(resp.output_tokens.is_none());
}
