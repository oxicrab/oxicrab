use super::*;
use crate::providers::base::{Message, ToolCallRequest};
use serde_json::json;

fn chat_req(messages: Vec<Message>) -> ChatRequest {
    ChatRequest {
        messages,
        max_tokens: 1024,
        ..Default::default()
    }
}

#[tokio::test]
async fn sequential_responses() {
    let json = serde_json::to_string(&vec![
        FixtureEntry {
            hint: None,
            min_messages: None,
            response: LLMResponse {
                content: Some("first".into()),
                ..Default::default()
            },
        },
        FixtureEntry {
            hint: None,
            min_messages: None,
            response: LLMResponse {
                content: Some("second".into()),
                ..Default::default()
            },
        },
    ])
    .expect("serialize");

    let provider = JsonFixtureLLMProvider::from_json(&json).expect("parse");
    let req = chat_req(vec![Message::user("hello")]);

    let r1 = provider.chat(&req).await.expect("chat");
    assert_eq!(r1.content.as_deref(), Some("first"));

    let r2 = provider.chat(&req).await.expect("chat");
    assert_eq!(r2.content.as_deref(), Some("second"));

    // Clamps to last entry when exhausted
    let r3 = provider.chat(&req).await.expect("chat");
    assert_eq!(r3.content.as_deref(), Some("second"));
}

#[tokio::test]
async fn hint_matching() {
    let json = serde_json::to_string(&vec![
        FixtureEntry {
            hint: Some("weather".into()),
            min_messages: None,
            response: LLMResponse {
                content: Some("sunny".into()),
                ..Default::default()
            },
        },
        FixtureEntry {
            hint: Some("time".into()),
            min_messages: None,
            response: LLMResponse {
                content: Some("3pm".into()),
                ..Default::default()
            },
        },
        FixtureEntry {
            hint: None,
            min_messages: None,
            response: LLMResponse {
                content: Some("fallback".into()),
                ..Default::default()
            },
        },
    ])
    .expect("serialize");

    let provider = JsonFixtureLLMProvider::from_json(&json).expect("parse");

    // Hint match on "time"
    let req = chat_req(vec![Message::user("what time is it?")]);
    let r = provider.chat(&req).await.expect("chat");
    assert_eq!(r.content.as_deref(), Some("3pm"));

    // Hint match on "weather"
    let req = chat_req(vec![Message::user("how's the weather?")]);
    let r = provider.chat(&req).await.expect("chat");
    assert_eq!(r.content.as_deref(), Some("sunny"));

    // No hint match, falls back to sequential
    let req = chat_req(vec![Message::user("tell me a joke")]);
    let r = provider.chat(&req).await.expect("chat");
    assert_eq!(r.content.as_deref(), Some("fallback"));
}

#[tokio::test]
async fn hint_with_min_messages() {
    let json = serde_json::to_string(&vec![
        FixtureEntry {
            hint: Some("hello".into()),
            min_messages: Some(3),
            response: LLMResponse {
                content: Some("deep conversation".into()),
                ..Default::default()
            },
        },
        FixtureEntry {
            hint: None,
            min_messages: None,
            response: LLMResponse {
                content: Some("early".into()),
                ..Default::default()
            },
        },
    ])
    .expect("serialize");

    let provider = JsonFixtureLLMProvider::from_json(&json).expect("parse");

    // Only 1 message: min_messages=3 not met, falls to sequential
    let req = chat_req(vec![Message::user("hello")]);
    let r = provider.chat(&req).await.expect("chat");
    assert_eq!(r.content.as_deref(), Some("early"));

    // 3 messages: hint matches
    let req = chat_req(vec![
        Message::user("hello"),
        Message::assistant("hi", None),
        Message::user("hello again"),
    ]);
    let r = provider.chat(&req).await.expect("chat");
    assert_eq!(r.content.as_deref(), Some("deep conversation"));
}

#[tokio::test]
async fn tool_call_fixture() {
    let json = serde_json::to_string(&vec![FixtureEntry {
        hint: None,
        min_messages: None,
        response: LLMResponse {
            tool_calls: vec![ToolCallRequest {
                id: "call_1".into(),
                name: "get_weather".into(),
                arguments: json!({"city": "London"}),
            }],
            ..Default::default()
        },
    }])
    .expect("serialize");

    let provider = JsonFixtureLLMProvider::from_json(&json).expect("parse");
    let req = chat_req(vec![Message::user("weather in London")]);
    let r = provider.chat(&req).await.expect("chat");
    assert_eq!(r.tool_calls.len(), 1);
    assert_eq!(r.tool_calls[0].name, "get_weather");
}

#[tokio::test]
async fn from_file_round_trip() {
    let entries = vec![FixtureEntry {
        hint: Some("greet".into()),
        min_messages: None,
        response: LLMResponse {
            content: Some("hello!".into()),
            ..Default::default()
        },
    }];

    let tmp = tempfile::NamedTempFile::new().expect("create temp file");
    std::fs::write(
        tmp.path(),
        serde_json::to_string_pretty(&entries).expect("serialize"),
    )
    .expect("write");

    let provider = JsonFixtureLLMProvider::from_file(tmp.path()).expect("from_file");
    let req = chat_req(vec![Message::user("greet me")]);
    let r = provider.chat(&req).await.expect("chat");
    assert_eq!(r.content.as_deref(), Some("hello!"));
}

#[test]
fn empty_fixture_rejected() {
    let result = JsonFixtureLLMProvider::from_json("[]");
    assert!(result.is_err());
}
