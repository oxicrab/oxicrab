use super::*;
use crate::providers::base::Message;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn simple_chat_request(content: &str) -> ChatRequest<'_> {
    ChatRequest {
        messages: vec![Message::user(content)],
        tools: None,
        model: None,
        max_tokens: 1024,
        temperature: 0.7,
        tool_choice: None,
        response_format: None,
    }
}

#[tokio::test]
async fn test_chat_success() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/"))
        .and(header("x-api-key", "test_key"))
        .and(header("anthropic-version", "2023-06-01"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "content": [{"type": "text", "text": "Hello! How can I help?"}],
            "model": "claude-sonnet-4-5-20250929",
            "role": "assistant",
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 10, "output_tokens": 8}
        })))
        .mount(&server)
        .await;

    let provider = AnthropicProvider::with_base_url("test_key".to_string(), None, server.uri());
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
            "content": [
                {"type": "tool_use", "id": "tc_1", "name": "weather", "input": {"city": "NYC"}}
            ],
            "model": "claude-sonnet-4-5-20250929",
            "role": "assistant",
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 20, "output_tokens": 15}
        })))
        .mount(&server)
        .await;

    let provider = AnthropicProvider::with_base_url("test_key".to_string(), None, server.uri());
    let result = provider
        .chat(simple_chat_request("What's the weather in NYC?"))
        .await
        .unwrap();

    assert!(result.has_tool_calls());
    assert_eq!(result.tool_calls[0].name, "weather");
    assert_eq!(result.tool_calls[0].id, "tc_1");
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

    let provider = AnthropicProvider::with_base_url("bad_key".to_string(), None, server.uri());
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
                .insert_header("retry-after", "30")
                .set_body_json(json!({
                    "error": {"type": "rate_limit_error", "message": "Rate limit exceeded"}
                })),
        )
        .mount(&server)
        .await;

    let provider = AnthropicProvider::with_base_url("test_key".to_string(), None, server.uri());
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
            "error": {"type": "api_error", "message": "Internal server error"}
        })))
        .mount(&server)
        .await;

    let provider = AnthropicProvider::with_base_url("test_key".to_string(), None, server.uri());
    let result = provider.chat(simple_chat_request("Hi")).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn test_chat_with_system_message() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "content": [{"type": "text", "text": "I am a helpful assistant."}],
            "model": "claude-sonnet-4-5-20250929",
            "role": "assistant",
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 25, "output_tokens": 10}
        })))
        .mount(&server)
        .await;

    let provider = AnthropicProvider::with_base_url("test_key".to_string(), None, server.uri());
    let req = ChatRequest {
        messages: vec![
            Message::system("You are a helpful assistant."),
            Message::user("Hello"),
        ],
        tools: None,
        model: None,
        max_tokens: 1024,
        temperature: 0.7,
        tool_choice: None,
        response_format: None,
    };
    let result = provider.chat(req).await.unwrap();

    assert_eq!(result.content.unwrap(), "I am a helpful assistant.");
}

#[tokio::test]
async fn test_chat_metrics_updated() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "content": [{"type": "text", "text": "Hi"}],
            "model": "claude-sonnet-4-5-20250929",
            "role": "assistant",
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 5, "output_tokens": 3}
        })))
        .mount(&server)
        .await;

    let provider = AnthropicProvider::with_base_url("test_key".to_string(), None, server.uri());
    provider.chat(simple_chat_request("Hi")).await.unwrap();

    let metrics = provider.metrics.lock().unwrap();
    assert_eq!(metrics.request_count, 1);
    assert_eq!(metrics.token_count, 8); // 5 input + 3 output
}
