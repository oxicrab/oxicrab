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
    finish_reason: Option<String>,
}

#[async_trait]
impl LLMProvider for FlushMock {
    async fn chat(&self, _req: &ChatRequest) -> anyhow::Result<LLMResponse> {
        Ok(LLMResponse {
            content: Some(self.response.clone()),
            finish_reason: self.finish_reason.clone(),
            ..Default::default()
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
        finish_reason: None,
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
        finish_reason: None,
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
        finish_reason: None,
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
        finish_reason: None,
    });
    let compactor = MessageCompactor::new(provider, None);

    let result = compactor.flush_to_memory(&[]).await.unwrap();
    assert!(result.is_empty());
}

// ── finish_reason guard tests ────────────────────────────

fn sample_messages() -> Vec<HashMap<String, Value>> {
    vec![{
        let mut m = HashMap::new();
        m.insert("role".to_string(), Value::String("user".to_string()));
        m.insert(
            "content".to_string(),
            Value::String("important project details here".to_string()),
        );
        m
    }]
}

#[tokio::test]
async fn flush_to_memory_discards_truncated_output_finish_length() {
    // finish_reason = "length" means output was truncated — should be discarded
    let provider = Arc::new(FlushMock {
        response: "- Important fact that got cut off mid-sen".to_string(),
        finish_reason: Some("length".to_string()),
    });
    let compactor = MessageCompactor::new(provider, None);

    let result = compactor.flush_to_memory(&sample_messages()).await.unwrap();
    assert!(
        result.is_empty(),
        "truncated output (finish_reason=length) should be discarded"
    );
}

#[tokio::test]
async fn flush_to_memory_discards_truncated_output_finish_max_tokens() {
    // finish_reason = "max_tokens" (Anthropic) means output was truncated
    let provider = Arc::new(FlushMock {
        response: "- Partial extraction that was cut".to_string(),
        finish_reason: Some("max_tokens".to_string()),
    });
    let compactor = MessageCompactor::new(provider, None);

    let result = compactor.flush_to_memory(&sample_messages()).await.unwrap();
    assert!(
        result.is_empty(),
        "truncated output (finish_reason=max_tokens) should be discarded"
    );
}

#[tokio::test]
async fn flush_to_memory_discards_truncated_output_finish_max_tokens_upper() {
    // finish_reason = "MAX_TOKENS" (Gemini) means output was truncated
    let provider = Arc::new(FlushMock {
        response: "- Gemini partial output".to_string(),
        finish_reason: Some("MAX_TOKENS".to_string()),
    });
    let compactor = MessageCompactor::new(provider, None);

    let result = compactor.flush_to_memory(&sample_messages()).await.unwrap();
    assert!(
        result.is_empty(),
        "truncated output (finish_reason=MAX_TOKENS) should be discarded"
    );
}

#[tokio::test]
async fn flush_to_memory_preserves_output_with_normal_finish_reason() {
    // finish_reason = "stop" or "end_turn" means normal completion — should be preserved
    let provider = Arc::new(FlushMock {
        response: "- User prefers vim keybindings".to_string(),
        finish_reason: Some("stop".to_string()),
    });
    let compactor = MessageCompactor::new(provider, None);

    let result = compactor.flush_to_memory(&sample_messages()).await.unwrap();
    assert!(
        result.contains("vim keybindings"),
        "normal finish_reason=stop should preserve output"
    );
}

// ── split_at_turn_boundary tests ─────────────────────────

fn role_msg(role: &str) -> HashMap<String, Value> {
    HashMap::from([
        ("role".into(), json!(role)),
        ("content".into(), json!("test")),
    ])
}

#[test]
fn split_at_turn_boundary_basic_two_turns() {
    // 2 turns: [user, assistant] + [user, assistant, tool, assistant]
    let msgs = vec![
        role_msg("user"),
        role_msg("assistant"),
        role_msg("user"),
        role_msg("assistant"),
        role_msg("tool"),
        role_msg("assistant"),
    ];
    // Keep 1 turn → split at index 2 (start of second user message)
    assert_eq!(split_at_turn_boundary(&msgs, 1), 2);
    // Keep 2 turns → split at index 0 (keep everything)
    assert_eq!(split_at_turn_boundary(&msgs, 2), 0);
}

#[test]
fn split_at_turn_boundary_no_user_messages() {
    // No user messages — no turns to find
    let msgs = vec![
        role_msg("assistant"),
        role_msg("tool"),
        role_msg("assistant"),
    ];
    assert_eq!(split_at_turn_boundary(&msgs, 1), 0);
}

#[test]
fn split_at_turn_boundary_single_turn() {
    let msgs = vec![role_msg("user"), role_msg("assistant")];
    // Keep 1 turn → everything is one turn, keep all
    assert_eq!(split_at_turn_boundary(&msgs, 1), 0);
}

#[test]
fn split_at_turn_boundary_empty() {
    let msgs: Vec<HashMap<String, Value>> = vec![];
    assert_eq!(split_at_turn_boundary(&msgs, 1), 0);
}

#[test]
fn split_at_turn_boundary_keep_zero() {
    // keep_turns=0 returns 0 (nothing to keep)
    let msgs = vec![role_msg("user"), role_msg("assistant")];
    assert_eq!(split_at_turn_boundary(&msgs, 0), 0);
}

#[test]
fn split_at_turn_boundary_more_turns_requested_than_available() {
    // Only 2 turns but requesting 5 → keep all (split at earliest user)
    let msgs = vec![
        role_msg("user"),
        role_msg("assistant"),
        role_msg("user"),
        role_msg("assistant"),
    ];
    assert_eq!(split_at_turn_boundary(&msgs, 5), 0);
}

#[test]
fn split_at_turn_boundary_three_turns() {
    // 3 turns: [user, asst] [user, tool, asst] [user, asst]
    let msgs = vec![
        role_msg("user"),      // 0 - turn 1 start
        role_msg("assistant"), // 1
        role_msg("user"),      // 2 - turn 2 start
        role_msg("tool"),      // 3
        role_msg("assistant"), // 4
        role_msg("user"),      // 5 - turn 3 start
        role_msg("assistant"), // 6
    ];
    // Keep 1 → split at 5 (only last turn)
    assert_eq!(split_at_turn_boundary(&msgs, 1), 5);
    // Keep 2 → split at 2 (turns 2 and 3)
    assert_eq!(split_at_turn_boundary(&msgs, 2), 2);
    // Keep 3 → split at 0 (all turns)
    assert_eq!(split_at_turn_boundary(&msgs, 3), 0);
}

#[test]
fn split_at_turn_boundary_consecutive_user_messages() {
    // Each user message starts its own turn even if consecutive
    let msgs = vec![
        role_msg("user"),      // 0 - turn 1
        role_msg("user"),      // 1 - turn 2
        role_msg("assistant"), // 2
    ];
    // Keep 1 → split at 1 (second user message starts last turn)
    assert_eq!(split_at_turn_boundary(&msgs, 1), 1);
    // Keep 2 → split at 0
    assert_eq!(split_at_turn_boundary(&msgs, 2), 0);
}

#[test]
fn split_at_turn_boundary_tool_heavy_single_turn() {
    // Single turn with many tool calls
    let msgs = vec![
        role_msg("user"),
        role_msg("assistant"),
        role_msg("tool"),
        role_msg("assistant"),
        role_msg("tool"),
        role_msg("assistant"),
        role_msg("tool"),
        role_msg("assistant"),
    ];
    // Keep 1 → only one turn, keep all
    assert_eq!(split_at_turn_boundary(&msgs, 1), 0);
}

// ── Anthropic-style tool_use with mixed orphans ──────────

#[test]
fn strip_orphans_anthropic_mixed_tool_use_partial_results() {
    // Anthropic-style assistant with two tool_use blocks in content,
    // but only one has a matching result — the orphaned one should be stripped.
    let mut msgs = vec![
        user_msg("analyze these"),
        HashMap::from([
            ("role".into(), json!("assistant")),
            (
                "content".into(),
                json!([
                    {"type": "text", "text": "Let me check that."},
                    {"type": "tool_use", "id": "tu_1", "name": "read_file", "input": {"path": "test.txt"}},
                    {"type": "tool_use", "id": "tu_2", "name": "list_dir", "input": {"path": "."}},
                ]),
            ),
        ]),
        // Only tu_1 has a result — tu_2 is orphaned
        tool_result_msg("tu_1", "file contents here"),
    ];

    let (orphaned_results, orphaned_calls) = strip_orphaned_tool_messages(&mut msgs);
    assert_eq!(orphaned_results, 0, "no orphaned results");
    assert_eq!(orphaned_calls, 1, "tu_2 has no result");

    // Verify tu_2 was stripped from the content array
    let content = msgs[1]["content"].as_array().unwrap();
    assert_eq!(content.len(), 2, "text + tu_1 should remain");
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[1]["type"], "tool_use");
    assert_eq!(content[1]["id"], "tu_1");
}

#[test]
fn strip_orphans_mixed_openai_and_anthropic_styles() {
    // Message with both OpenAI-style tool_calls AND Anthropic-style content blocks.
    // Both orphans should be detected and stripped.
    let mut msgs = vec![
        user_msg("test hybrid"),
        HashMap::from([
            ("role".into(), json!("assistant")),
            (
                "content".into(),
                json!([
                    {"type": "text", "text": "processing"},
                    {"type": "tool_use", "id": "ant_1", "name": "read", "input": {}},
                ]),
            ),
            (
                "tool_calls".into(),
                json!([
                    {"id": "oai_1", "name": "write", "input": {}}
                ]),
            ),
        ]),
        // Both tool calls have results
        tool_result_msg("ant_1", "read result"),
        tool_result_msg("oai_1", "write result"),
    ];

    let (orphaned_results, orphaned_calls) = strip_orphaned_tool_messages(&mut msgs);
    assert_eq!(orphaned_results, 0);
    assert_eq!(orphaned_calls, 0);
    assert_eq!(msgs.len(), 4);

    // Now remove one result to create an orphan
    msgs.remove(3); // remove oai_1 result
    let (orphaned_results, orphaned_calls) = strip_orphaned_tool_messages(&mut msgs);
    assert_eq!(orphaned_results, 0);
    assert_eq!(orphaned_calls, 1, "oai_1 should be orphaned");
    // oai_1 should be stripped from tool_calls
    assert!(
        !msgs[1].contains_key("tool_calls"),
        "empty tool_calls key should be removed"
    );
    // Anthropic tool_use should still be present
    let content = msgs[1]["content"].as_array().unwrap();
    assert_eq!(content.len(), 2);
    assert_eq!(content[1]["id"], "ant_1");
}

#[test]
fn strip_orphans_tool_result_with_null_id() {
    // A tool result with a null tool_call_id is malformed and should be removed
    let mut msgs = vec![
        user_msg("test"),
        HashMap::from([
            ("role".into(), json!("tool")),
            ("content".into(), json!("malformed")),
            ("tool_call_id".into(), Value::Null),
        ]),
        assistant_msg("ok"),
    ];
    let (orphaned_results, _) = strip_orphaned_tool_messages(&mut msgs);
    // Null ID means as_str() returns None → removed as malformed
    assert_eq!(orphaned_results, 1);
    assert_eq!(msgs.len(), 2);
}
