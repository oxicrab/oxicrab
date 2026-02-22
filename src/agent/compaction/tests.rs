use super::*;
use proptest::prelude::*;
use serde_json::json;

proptest! {
    #[test]
    fn estimate_tokens_never_panics(s in "\\PC*") {
        let _ = estimate_tokens(&s);
    }

    #[test]
    fn estimate_tokens_proportional_to_length(s in ".{0,1000}") {
        let tokens = estimate_tokens(&s);
        let char_count = s.chars().count();
        assert_eq!(tokens, char_count / CHARS_PER_TOKEN_ESTIMATE);
    }

    #[test]
    fn estimate_tokens_empty_is_zero(s in "\\s{0,10}") {
        let tokens = estimate_tokens(&s);
        // Whitespace-only strings up to 3 chars should give 0 tokens
        if s.chars().count() < CHARS_PER_TOKEN_ESTIMATE {
            assert_eq!(tokens, 0);
        }
    }
}

#[test]
fn estimate_tokens_empty() {
    assert_eq!(estimate_tokens(""), 0);
}

#[test]
fn estimate_tokens_ascii() {
    // 20 chars / 4 = 5
    assert_eq!(estimate_tokens("12345678901234567890"), 5);
}

#[test]
fn estimate_tokens_unicode() {
    // Each emoji is 1 char (but 4 bytes). 4 emoji = 4 chars / 4 = 1 token
    assert_eq!(estimate_tokens("\u{1F600}\u{1F601}\u{1F602}\u{1F603}"), 1);
}

#[test]
fn estimate_messages_tokens_empty() {
    assert_eq!(estimate_messages_tokens(&[]), 0);
}

#[test]
fn estimate_messages_tokens_string_content() {
    let msgs = vec![{
        let mut m = HashMap::new();
        m.insert(
            "content".to_string(),
            Value::String("a".repeat(40)), // 40 chars = 10 tokens
        );
        m
    }];
    assert_eq!(estimate_messages_tokens(&msgs), 10);
}

#[test]
fn estimate_messages_tokens_array_content() {
    let msgs = vec![{
        let mut m = HashMap::new();
        m.insert(
            "content".to_string(),
            serde_json::json!([
                {"type": "text", "text": "a]a]a]a]"}, // 8 chars = 2 tokens
                {"type": "image", "url": "http://example.com"},
                {"type": "text", "text": "bbbb"}, // 4 chars = 1 token
            ]),
        );
        m
    }];
    assert_eq!(estimate_messages_tokens(&msgs), 3);
}

#[test]
fn estimate_messages_tokens_missing_content() {
    let msgs = vec![{
        let mut m = HashMap::new();
        m.insert("role".to_string(), Value::String("user".to_string()));
        m
    }];
    assert_eq!(estimate_messages_tokens(&msgs), 0);
}

#[test]
fn extract_message_text_plain_string() {
    let content = Value::String("hello world".to_string());
    assert_eq!(extract_message_text(Some(&content)), "hello world");
}

#[test]
fn extract_message_text_array_with_images() {
    let content = json!([
        {"type": "text", "text": "See this"},
        {"type": "image", "url": "http://example.com/img.png"},
        {"type": "text", "text": "screenshot"}
    ]);
    assert_eq!(
        extract_message_text(Some(&content)),
        "See this [image] screenshot"
    );
}

#[test]
fn extract_message_text_none() {
    assert_eq!(extract_message_text(None), "");
}

#[test]
fn extract_message_text_non_string_non_array() {
    let content = json!(42);
    assert_eq!(extract_message_text(Some(&content)), "");
}
