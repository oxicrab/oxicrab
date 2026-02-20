mod common;

use common::{MockLLMProvider, TestAgentOverrides, create_test_agent_with, text_response};
use oxicrab::agent::compaction::{MessageCompactor, estimate_tokens};
use oxicrab::config::CompactionConfig;
use oxicrab::providers::base::LLMResponse;
use std::sync::Arc;
use tempfile::TempDir;

#[test]
fn test_estimate_tokens_basic() {
    // 4 chars per token
    assert_eq!(estimate_tokens("12345678"), 2);
    assert_eq!(estimate_tokens(""), 0);
}

#[tokio::test]
async fn test_compaction_disabled_preserves_full_history() {
    let tmp = TempDir::new().expect("create temp dir");

    // Send many messages to build up history; compaction is disabled
    let mut responses = Vec::new();
    for _ in 0..5 {
        responses.push(text_response("Response."));
    }
    let provider = MockLLMProvider::with_responses(responses);
    let calls = provider.calls.clone();

    let agent = create_test_agent_with(
        provider,
        &tmp,
        TestAgentOverrides {
            compaction_config: Some(CompactionConfig {
                enabled: false,
                threshold_tokens: 100,
                keep_recent: 2,
                extraction_enabled: false,
                model: None,
                checkpoint: Default::default(),
            }),
            ..Default::default()
        },
    )
    .await;

    let session = "test:no_compact";
    for i in 0..5 {
        agent
            .process_direct(
                &format!("Message number {}", i),
                session,
                "telegram",
                "no_compact",
            )
            .await
            .expect("process message");
    }

    // All 5 messages should be present (no compaction)
    let recorded = calls.lock().expect("lock recorded calls");
    // The last call should have all previous history
    let last_msgs = &recorded.last().unwrap().messages;
    // Should have system + 4 pairs of history + current = at least 10 messages
    assert!(
        last_msgs.len() >= 9,
        "With compaction disabled, all history should be present. Got {} messages",
        last_msgs.len()
    );
}

#[tokio::test]
async fn test_compact_produces_summary() {
    // Test MessageCompactor directly
    let provider = MockLLMProvider::with_responses(vec![text_response(
        "User discussed Rust programming and file management.",
    )]);

    let compactor = MessageCompactor::new(Arc::new(provider), None);

    let messages = vec![
        {
            let mut m = std::collections::HashMap::new();
            m.insert(
                "role".to_string(),
                serde_json::Value::String("user".to_string()),
            );
            m.insert(
                "content".to_string(),
                serde_json::Value::String("I want to learn Rust".to_string()),
            );
            m
        },
        {
            let mut m = std::collections::HashMap::new();
            m.insert(
                "role".to_string(),
                serde_json::Value::String("assistant".to_string()),
            );
            m.insert(
                "content".to_string(),
                serde_json::Value::String("Rust is a systems programming language.".to_string()),
            );
            m
        },
    ];

    let summary = compactor.compact(&messages, "").await.unwrap();
    assert!(!summary.is_empty());
    assert!(summary.contains("Rust"));
}

#[tokio::test]
async fn test_extract_facts_returns_nothing() {
    let provider = MockLLMProvider::with_responses(vec![text_response("NOTHING")]);

    let compactor = MessageCompactor::new(Arc::new(provider), None);

    let facts = compactor
        .extract_facts("What time is it?", "It's 3pm.")
        .await
        .unwrap();

    assert!(
        facts.is_empty(),
        "NOTHING response should produce empty facts, got: {}",
        facts
    );
}

#[tokio::test]
async fn test_extract_facts_returns_bullets() {
    let provider = MockLLMProvider::with_responses(vec![text_response(
        "- User prefers dark mode\n- User's name is Alice",
    )]);

    let compactor = MessageCompactor::new(Arc::new(provider), None);

    let facts = compactor
        .extract_facts("Call me Alice, and I like dark mode.", "Got it, Alice!")
        .await
        .unwrap();

    assert!(facts.contains("dark mode"));
    assert!(facts.contains("Alice"));
}

#[tokio::test]
async fn test_compaction_triggers_at_threshold() {
    let tmp = TempDir::new().expect("create temp dir");

    // Use a very low threshold so compaction triggers
    // We need enough responses: 5 conversation turns + possible compaction calls
    let mut responses: Vec<LLMResponse> = Vec::new();
    // Conversation responses
    for _ in 0..5 {
        responses.push(text_response(
            &"x".repeat(400), // ~100 tokens each
        ));
    }
    // Compaction summary call(s)
    responses.push(text_response("Compacted summary of conversation."));
    // Extra responses in case of additional turns
    responses.push(text_response("Final response."));
    responses.push(text_response("Extra."));

    let provider = MockLLMProvider::with_responses(responses);
    let calls = provider.calls.clone();

    let agent = create_test_agent_with(
        provider,
        &tmp,
        TestAgentOverrides {
            compaction_config: Some(CompactionConfig {
                enabled: true,
                threshold_tokens: 100, // Very low threshold
                keep_recent: 2,
                extraction_enabled: false,
                model: None,
                checkpoint: Default::default(),
            }),
            ..Default::default()
        },
    )
    .await;

    let session = "test:compact";
    // Send multiple messages to exceed the threshold
    for i in 0..5 {
        let _ = agent
            .process_direct(
                &format!(
                    "Long message {} with lots of content to exceed the compaction threshold: {}",
                    i,
                    "padding ".repeat(25)
                ),
                session,
                "telegram",
                "compact",
            )
            .await;
    }

    // Verify that compaction occurred by checking that the LLM was called
    // with a compaction prompt (contains "Summarize" in the messages)
    let recorded = calls.lock().expect("lock recorded calls");
    // Compaction may or may not trigger depending on exact token counting,
    // but we verify no panics and the system handles it gracefully
    assert!(
        recorded.len() >= 5,
        "Should have processed all messages, got {} calls",
        recorded.len()
    );

    // Verify that a compaction call included "Summarize" in a system/user message
    let has_compaction_call = recorded.iter().any(|call| {
        call.messages.iter().any(|m| {
            (m.role == "system" || m.role == "user")
                && m.content.to_lowercase().contains("summarize")
        })
    });
    assert!(
        has_compaction_call,
        "compaction should trigger a 'Summarize' LLM call at threshold_tokens=100; got {} LLM calls",
        recorded.len()
    );
}
