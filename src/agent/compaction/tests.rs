use super::*;
use crate::providers::base::LLMResponse;
use async_trait::async_trait;
use proptest::prelude::*;
use serde_json::json;

// ── Helper for building test messages ───────────────────────

fn user_msg(content: &str) -> HashMap<String, Value> {
    HashMap::from([
        ("role".into(), json!("user")),
        ("content".into(), json!(content)),
    ])
}

fn assistant_msg(content: &str) -> HashMap<String, Value> {
    HashMap::from([
        ("role".into(), json!("assistant")),
        ("content".into(), json!(content)),
    ])
}

fn assistant_with_tool_calls(content: &str, tool_call_ids: &[&str]) -> HashMap<String, Value> {
    let calls: Vec<Value> = tool_call_ids
        .iter()
        .map(|id| json!({"id": id, "name": "test_tool", "input": {}}))
        .collect();
    HashMap::from([
        ("role".into(), json!("assistant")),
        ("content".into(), json!(content)),
        ("tool_calls".into(), Value::Array(calls)),
    ])
}

fn tool_result_msg(tool_call_id: &str, content: &str) -> HashMap<String, Value> {
    HashMap::from([
        ("role".into(), json!("tool")),
        ("content".into(), json!(content)),
        ("tool_call_id".into(), json!(tool_call_id)),
    ])
}

// ── Orphan cleanup tests ────────────────────────────────────

#[test]
fn strip_orphans_matched_pairs_unchanged() {
    let mut msgs = vec![
        user_msg("hello"),
        assistant_with_tool_calls("calling tool", &["tc_1"]),
        tool_result_msg("tc_1", "tool output"),
        assistant_msg("done"),
    ];
    let original_len = msgs.len();
    let (orphaned_results, orphaned_calls) = strip_orphaned_tool_messages(&mut msgs);
    assert_eq!(orphaned_results, 0);
    assert_eq!(orphaned_calls, 0);
    assert_eq!(msgs.len(), original_len);
}

#[test]
fn strip_orphans_removes_orphaned_tool_result() {
    let mut msgs = vec![
        user_msg("hello"),
        // tc_1 has no matching assistant tool_call
        tool_result_msg("tc_1", "orphaned result"),
        assistant_msg("response"),
    ];
    let (orphaned_results, _) = strip_orphaned_tool_messages(&mut msgs);
    assert_eq!(orphaned_results, 1);
    assert_eq!(msgs.len(), 2);
    // Only user + assistant remain
    assert_eq!(msgs[0]["role"], json!("user"));
    assert_eq!(msgs[1]["role"], json!("assistant"));
}

#[test]
fn strip_orphans_detects_orphaned_tool_call() {
    let mut msgs = vec![
        user_msg("hello"),
        assistant_with_tool_calls("calling", &["tc_1"]),
        // No tool_result for tc_1
        assistant_msg("continuing without result"),
    ];
    let (orphaned_results, orphaned_calls) = strip_orphaned_tool_messages(&mut msgs);
    assert_eq!(orphaned_results, 0);
    assert_eq!(orphaned_calls, 1);
    // All messages kept but tool_calls stripped from assistant
    assert_eq!(msgs.len(), 3);
    // The orphaned tool_calls key should be removed entirely
    assert!(
        !msgs[1].contains_key("tool_calls"),
        "orphaned tool_calls should be stripped from assistant message"
    );
}

#[test]
fn strip_orphans_multiple_tool_calls_partial_orphan() {
    let mut msgs = vec![
        user_msg("do two things"),
        assistant_with_tool_calls("calling tools", &["tc_1", "tc_2"]),
        tool_result_msg("tc_1", "result 1"),
        // tc_2 result is missing (orphaned call)
        assistant_msg("done"),
    ];
    let (orphaned_results, orphaned_calls) = strip_orphaned_tool_messages(&mut msgs);
    assert_eq!(orphaned_results, 0);
    assert_eq!(orphaned_calls, 1);
    assert_eq!(msgs.len(), 4);
    // tc_2 should be stripped but tc_1 should remain
    let tool_calls = msgs[1]["tool_calls"].as_array().unwrap();
    assert_eq!(tool_calls.len(), 1, "only matched tc_1 should remain");
    assert_eq!(tool_calls[0]["id"], "tc_1");
}

#[test]
fn strip_orphans_no_tool_messages() {
    let mut msgs = vec![
        user_msg("hello"),
        assistant_msg("hi there"),
        user_msg("how are you"),
        assistant_msg("great"),
    ];
    let (orphaned_results, orphaned_calls) = strip_orphaned_tool_messages(&mut msgs);
    assert_eq!(orphaned_results, 0);
    assert_eq!(orphaned_calls, 0);
    assert_eq!(msgs.len(), 4);
}

#[test]
fn strip_orphans_empty_messages() {
    let mut msgs: Vec<HashMap<String, Value>> = vec![];
    let (orphaned_results, orphaned_calls) = strip_orphaned_tool_messages(&mut msgs);
    assert_eq!(orphaned_results, 0);
    assert_eq!(orphaned_calls, 0);
}

#[test]
fn strip_orphans_openai_all_calls_orphaned_removes_key() {
    // When ALL tool_calls in an assistant message are orphaned, the key is removed entirely
    let mut msgs = vec![
        user_msg("do things"),
        assistant_with_tool_calls("calling", &["tc_a", "tc_b"]),
        // No results for either
        assistant_msg("done"),
    ];
    let (_, orphaned_calls) = strip_orphaned_tool_messages(&mut msgs);
    assert_eq!(orphaned_calls, 2);
    // tool_calls key should be removed entirely (not left as empty array)
    assert!(
        !msgs[1].contains_key("tool_calls"),
        "tool_calls key should be removed when all calls are orphaned"
    );
}

#[test]
fn strip_orphans_anthropic_tool_use_blocks_stripped() {
    // Anthropic-style: orphaned tool_use blocks in content array should be stripped
    let mut msgs = vec![
        user_msg("test"),
        HashMap::from([
            ("role".into(), json!("assistant")),
            (
                "content".into(),
                json!([
                    {"type": "text", "text": "Let me help"},
                    {"type": "tool_use", "id": "tc_orphan", "name": "read", "input": {}},
                    {"type": "tool_use", "id": "tc_matched", "name": "write", "input": {}}
                ]),
            ),
        ]),
        tool_result_msg("tc_matched", "write result"),
        assistant_msg("done"),
    ];
    let (_, orphaned_calls) = strip_orphaned_tool_messages(&mut msgs);
    assert_eq!(orphaned_calls, 1);
    // Content array should still have text block + matched tool_use, but orphaned one removed
    let content = msgs[1]["content"].as_array().unwrap();
    assert_eq!(content.len(), 2, "text + matched tool_use should remain");
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[1]["type"], "tool_use");
    assert_eq!(content[1]["id"], "tc_matched");
}

#[test]
fn strip_orphans_anthropic_all_tool_use_blocks_orphaned() {
    // When all tool_use blocks in content are orphaned, only text blocks remain
    let mut msgs = vec![
        user_msg("test"),
        HashMap::from([
            ("role".into(), json!("assistant")),
            (
                "content".into(),
                json!([
                    {"type": "text", "text": "Calling tools"},
                    {"type": "tool_use", "id": "tc_1", "name": "read", "input": {}},
                    {"type": "tool_use", "id": "tc_2", "name": "write", "input": {}}
                ]),
            ),
        ]),
        // No results for either tool_use
        assistant_msg("continuing"),
    ];
    let (_, orphaned_calls) = strip_orphaned_tool_messages(&mut msgs);
    assert_eq!(orphaned_calls, 2);
    let content = msgs[1]["content"].as_array().unwrap();
    assert_eq!(content.len(), 1, "only text block should remain");
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[0]["text"], "Calling tools");
}

#[test]
fn strip_orphans_anthropic_content_block_format() {
    // Anthropic-style content array with tool_use blocks
    let mut msgs = vec![
        user_msg("test"),
        HashMap::from([
            ("role".into(), json!("assistant")),
            (
                "content".into(),
                json!([
                    {"type": "text", "text": "Let me check"},
                    {"type": "tool_use", "id": "tc_abc", "name": "read", "input": {}}
                ]),
            ),
        ]),
        tool_result_msg("tc_abc", "file contents"),
        assistant_msg("here's what I found"),
    ];
    let (orphaned_results, orphaned_calls) = strip_orphaned_tool_messages(&mut msgs);
    assert_eq!(orphaned_results, 0);
    assert_eq!(orphaned_calls, 0);
    assert_eq!(msgs.len(), 4);
}

#[test]
fn strip_orphans_tool_result_without_id_removed() {
    let mut msgs = vec![
        user_msg("test"),
        HashMap::from([
            ("role".into(), json!("tool")),
            ("content".into(), json!("malformed result")),
            // No tool_call_id field
        ]),
        assistant_msg("response"),
    ];
    let (orphaned_results, _) = strip_orphaned_tool_messages(&mut msgs);
    assert_eq!(orphaned_results, 1);
    assert_eq!(msgs.len(), 2);
}

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
                {"type": "text", "text": "a]a]a]a]"}, // 8 chars
                {"type": "image", "url": "http://example.com"}, // → "[image]" placeholder (7 chars)
                {"type": "text", "text": "bbbb"}, // 4 chars
            ]),
        );
        m
    }];
    // After extract_message_text join: "a]a]a]a] [image] bbbb" = 21 chars = 5 tokens
    assert_eq!(estimate_messages_tokens(&msgs), 5);
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

// ── Pre-compaction flush tests ───────────────────────────

struct FlushMock {
    response: String,
}

#[async_trait]
impl LLMProvider for FlushMock {
    async fn chat(&self, _req: ChatRequest<'_>) -> anyhow::Result<LLMResponse> {
        Ok(LLMResponse {
            content: Some(self.response.clone()),
            tool_calls: vec![],
            reasoning_content: None,
            input_tokens: None,
            output_tokens: None,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
        })
    }
    fn default_model(&self) -> &'static str {
        "mock"
    }
}

#[tokio::test]
async fn flush_to_memory_extracts_facts() {
    let provider = Arc::new(FlushMock {
        response: "- User prefers dark mode\n- Project uses Rust nightly".to_string(),
    });
    let compactor = MessageCompactor::new(provider, None);

    let messages = vec![{
        let mut m = HashMap::new();
        m.insert("role".to_string(), Value::String("user".to_string()));
        m.insert(
            "content".to_string(),
            Value::String("I always use dark mode. This project runs on Rust nightly.".to_string()),
        );
        m
    }];

    let result = compactor.flush_to_memory(&messages).await.unwrap();
    assert!(result.contains("dark mode"));
    assert!(result.contains("Rust nightly"));
}

#[tokio::test]
async fn flush_to_memory_returns_empty_for_nothing() {
    let provider = Arc::new(FlushMock {
        response: "NOTHING".to_string(),
    });
    let compactor = MessageCompactor::new(provider, None);

    let messages = vec![{
        let mut m = HashMap::new();
        m.insert("role".to_string(), Value::String("user".to_string()));
        m.insert("content".to_string(), Value::String("hello".to_string()));
        m
    }];

    let result = compactor.flush_to_memory(&messages).await.unwrap();
    assert!(result.is_empty());
}

#[tokio::test]
async fn flush_to_memory_nothing_case_insensitive() {
    let provider = Arc::new(FlushMock {
        response: "Nothing worth preserving here".to_string(),
    });
    let compactor = MessageCompactor::new(provider, None);

    let messages = vec![{
        let mut m = HashMap::new();
        m.insert("role".to_string(), Value::String("user".to_string()));
        m.insert("content".to_string(), Value::String("test".to_string()));
        m
    }];

    let result = compactor.flush_to_memory(&messages).await.unwrap();
    assert!(result.is_empty());
}

#[tokio::test]
async fn flush_to_memory_empty_messages() {
    let provider = Arc::new(FlushMock {
        response: "NOTHING".to_string(),
    });
    let compactor = MessageCompactor::new(provider, None);

    let result = compactor.flush_to_memory(&[]).await.unwrap();
    assert!(result.is_empty());
}
