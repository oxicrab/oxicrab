use super::*;
use crate::providers::base::Message;
use wiremock::matchers::{method, path};
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
        .and(path("/models/gemini-pro:generateContent"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "candidates": [{
                "content": {
                    "parts": [{"text": "Hello! How can I help you?"}],
                    "role": "model"
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {"totalTokenCount": 15}
        })))
        .mount(&server)
        .await;

    let provider = GeminiProvider::with_base_url("test_key".to_string(), None, server.uri());
    let result = provider.chat(simple_chat_request("Hi")).await.unwrap();

    assert_eq!(result.content.unwrap(), "Hello! How can I help you?");
    assert!(result.tool_calls.is_empty());
}

#[tokio::test]
async fn test_chat_with_tool_calls() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-pro:generateContent"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "candidates": [{
                "content": {
                    "parts": [{
                        "functionCall": {
                            "name": "weather",
                            "args": {"city": "London"}
                        }
                    }],
                    "role": "model"
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {"totalTokenCount": 20}
        })))
        .mount(&server)
        .await;

    let provider = GeminiProvider::with_base_url("test_key".to_string(), None, server.uri());
    let result = provider
        .chat(simple_chat_request("Weather in London?"))
        .await
        .unwrap();

    assert!(result.has_tool_calls());
    assert_eq!(result.tool_calls[0].name, "weather");
    assert_eq!(result.tool_calls[0].arguments["city"], "London");
}

#[tokio::test]
async fn test_chat_unauthorized() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-pro:generateContent"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": {"type": "auth_error", "message": "API key not valid"}
        })))
        .mount(&server)
        .await;

    let provider = GeminiProvider::with_base_url("bad_key".to_string(), None, server.uri());
    let result = provider.chat(simple_chat_request("Hi")).await;

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("Authentication"), "Error: {}", err);
}

#[tokio::test]
async fn test_chat_rate_limit() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-pro:generateContent"))
        .respond_with(ResponseTemplate::new(429).set_body_json(json!({
            "error": {"type": "rate_limit", "message": "Quota exceeded"}
        })))
        .mount(&server)
        .await;

    let provider = GeminiProvider::with_base_url("test_key".to_string(), None, server.uri());
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
        .and(path("/models/gemini-pro:generateContent"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({
            "error": {"type": "server_error", "message": "Internal error"}
        })))
        .mount(&server)
        .await;

    let provider = GeminiProvider::with_base_url("test_key".to_string(), None, server.uri());
    let result = provider.chat(simple_chat_request("Hi")).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn test_chat_metrics_updated() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-pro:generateContent"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "candidates": [{
                "content": {
                    "parts": [{"text": "Hi"}],
                    "role": "model"
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {"totalTokenCount": 12}
        })))
        .mount(&server)
        .await;

    let provider = GeminiProvider::with_base_url("test_key".to_string(), None, server.uri());
    provider.chat(simple_chat_request("Hi")).await.unwrap();

    let metrics = provider.metrics.lock().unwrap();
    assert_eq!(metrics.request_count, 1);
    assert_eq!(metrics.token_count, 12);
}

#[tokio::test]
async fn test_chat_custom_model() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-2.0-flash:generateContent"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "candidates": [{
                "content": {
                    "parts": [{"text": "Flash response"}],
                    "role": "model"
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {"totalTokenCount": 8}
        })))
        .mount(&server)
        .await;

    let provider = GeminiProvider::with_base_url(
        "test_key".to_string(),
        Some("gemini-2.0-flash".to_string()),
        server.uri(),
    );
    let result = provider.chat(simple_chat_request("Hi")).await.unwrap();

    assert_eq!(result.content.unwrap(), "Flash response");
}

#[tokio::test]
async fn test_system_message_as_system_instruction() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-pro:generateContent"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "candidates": [{
                "content": {
                    "parts": [{"text": "I'm a helpful bot."}],
                    "role": "model"
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {"totalTokenCount": 10}
        })))
        .mount(&server)
        .await;

    let provider = GeminiProvider::with_base_url("test_key".to_string(), None, server.uri());
    let req = ChatRequest {
        messages: vec![Message::system("You are helpful."), Message::user("Hello")],
        tools: None,
        model: None,
        max_tokens: 1024,
        temperature: 0.7,
        tool_choice: None,
    };
    let result = provider.chat(req).await.unwrap();

    assert_eq!(result.content.unwrap(), "I'm a helpful bot.");
}

// --- parse_response unit tests (no network) ---

#[test]
fn test_parse_response_text_only() {
    let json = json!({
        "candidates": [{
            "content": {"parts": [{"text": "Hello"}], "role": "model"}
        }],
        "usageMetadata": {"promptTokenCount": 5, "candidatesTokenCount": 3}
    });
    let resp = GeminiProvider::parse_response(&json).unwrap();
    assert_eq!(resp.content.as_deref(), Some("Hello"));
    assert!(resp.tool_calls.is_empty());
    assert_eq!(resp.input_tokens, Some(5));
    assert_eq!(resp.output_tokens, Some(3));
}

#[test]
fn test_parse_response_function_call() {
    let json = json!({
        "candidates": [{
            "content": {
                "parts": [{
                    "functionCall": {"name": "get_weather", "args": {"city": "Paris"}}
                }],
                "role": "model"
            }
        }]
    });
    let resp = GeminiProvider::parse_response(&json).unwrap();
    assert!(resp.content.is_none());
    assert_eq!(resp.tool_calls.len(), 1);
    assert_eq!(resp.tool_calls[0].name, "get_weather");
    // Gemini uses function name as ID
    assert_eq!(resp.tool_calls[0].id, "get_weather");
    assert_eq!(resp.tool_calls[0].arguments["city"], "Paris");
}

#[test]
fn test_parse_response_text_and_function_call() {
    let json = json!({
        "candidates": [{
            "content": {
                "parts": [
                    {"text": "Let me check the weather."},
                    {"functionCall": {"name": "weather", "args": {}}}
                ]
            }
        }]
    });
    let resp = GeminiProvider::parse_response(&json).unwrap();
    assert_eq!(resp.content.as_deref(), Some("Let me check the weather."));
    assert_eq!(resp.tool_calls.len(), 1);
}

#[test]
fn test_parse_response_no_candidates() {
    let json = json!({"candidates": []});
    let result = GeminiProvider::parse_response(&json);
    assert!(result.is_err());
}

#[test]
fn test_parse_response_no_usage_metadata() {
    let json = json!({
        "candidates": [{"content": {"parts": [{"text": "hi"}]}}]
    });
    let resp = GeminiProvider::parse_response(&json).unwrap();
    assert!(resp.input_tokens.is_none());
    assert!(resp.output_tokens.is_none());
}

#[test]
fn test_parse_response_multiple_function_calls() {
    let json = json!({
        "candidates": [{
            "content": {
                "parts": [
                    {"functionCall": {"name": "tool_a", "args": {}}},
                    {"functionCall": {"name": "tool_b", "args": {"x": 1}}}
                ]
            }
        }]
    });
    let resp = GeminiProvider::parse_response(&json).unwrap();
    assert_eq!(resp.tool_calls.len(), 2);
    assert_eq!(resp.tool_calls[0].name, "tool_a");
    assert_eq!(resp.tool_calls[1].name, "tool_b");
}

#[test]
fn test_parse_response_function_call_no_name_fallback_id() {
    let json = json!({
        "candidates": [{
            "content": {
                "parts": [{"functionCall": {"args": {}}}]
            }
        }]
    });
    let resp = GeminiProvider::parse_response(&json).unwrap();
    assert_eq!(resp.tool_calls.len(), 1);
    // Without a name, ID should be the fallback pattern
    assert!(resp.tool_calls[0].id.starts_with("gemini_tc_"));
}
