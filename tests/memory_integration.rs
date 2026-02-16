mod common;

use common::{
    create_test_agent_with, text_response, tool_call, tool_response, MockLLMProvider,
    TestAgentOverrides,
};
use oxicrab::agent::memory::{MemoryDB, MemoryStore};
use serde_json::json;
use tempfile::TempDir;

#[tokio::test]
async fn test_memory_store_daily_note_creation() {
    let tmp = TempDir::new().unwrap();
    let store = MemoryStore::new(tmp.path()).unwrap();

    store.append_today("Test fact: user likes Rust").unwrap();

    let today_file = store.get_today_file();
    assert!(today_file.exists());
    let content = std::fs::read_to_string(&today_file).unwrap();
    assert!(content.contains("Test fact: user likes Rust"));
}

#[tokio::test]
async fn test_memory_store_daily_note_append_multiple() {
    let tmp = TempDir::new().unwrap();
    let store = MemoryStore::new(tmp.path()).unwrap();

    store.append_today("First fact").unwrap();
    store.append_today("Second fact").unwrap();

    let content = std::fs::read_to_string(store.get_today_file()).unwrap();
    assert!(content.contains("First fact"));
    assert!(content.contains("Second fact"));
}

#[tokio::test]
async fn test_memory_store_context_includes_memory_md() {
    let tmp = TempDir::new().unwrap();
    let memory_dir = tmp.path().join("memory");
    std::fs::create_dir_all(&memory_dir).unwrap();
    std::fs::write(
        memory_dir.join("MEMORY.md"),
        "# Long-term Memory\n\nUser prefers dark mode.",
    )
    .unwrap();

    let store = MemoryStore::new(tmp.path()).unwrap();
    let context = store.get_memory_context(None).unwrap();

    assert!(
        context.contains("dark mode"),
        "Context should include MEMORY.md content: {}",
        context
    );
}

#[test]
fn test_memory_db_index_and_search_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test_memory.sqlite3");
    let db = MemoryDB::new(&db_path).unwrap();

    // Create files to index
    let file1 = tmp.path().join("notes1.md");
    let file2 = tmp.path().join("notes2.md");
    let file3 = tmp.path().join("notes3.md");
    std::fs::write(&file1, "Rust programming language is great for systems").unwrap();
    std::fs::write(&file2, "Python is popular for machine learning").unwrap();
    std::fs::write(&file3, "JavaScript dominates web development").unwrap();

    db.index_file("notes1.md", &file1).unwrap();
    db.index_file("notes2.md", &file2).unwrap();
    db.index_file("notes3.md", &file3).unwrap();

    // Search for Rust-related content
    let results = db.search("Rust programming", 10, None).unwrap();
    assert!(
        !results.is_empty(),
        "Should find results for 'Rust programming'"
    );
    assert!(results.iter().any(|r| r.content.contains("Rust")));
}

#[test]
fn test_memory_db_search_empty_query() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test_memory.sqlite3");
    let db = MemoryDB::new(&db_path).unwrap();

    let results = db.search("", 10, None).unwrap();
    assert!(results.is_empty(), "Empty query should return no results");
}

#[tokio::test]
async fn test_memory_search_tool_through_agent_loop() {
    let tmp = TempDir::new().unwrap();

    // Pre-write memory content so the tool can find it
    let memory_dir = tmp.path().join("memory");
    std::fs::create_dir_all(&memory_dir).unwrap();
    std::fs::write(
        memory_dir.join("MEMORY.md"),
        "# Memory\n\nUser's favorite color is blue.\nUser works at Acme Corp.",
    )
    .unwrap();

    let provider = MockLLMProvider::with_responses(vec![
        tool_response(vec![tool_call(
            "tc1",
            "memory_search",
            json!({"query": "favorite color"}),
        )]),
        text_response("Your favorite color is blue."),
    ]);
    let calls = provider.calls.clone();

    let agent = create_test_agent_with(provider, &tmp, TestAgentOverrides::default()).await;
    let response = agent
        .process_direct("What's my favorite color?", "test:mem1", "telegram", "mem1")
        .await
        .unwrap();

    assert_eq!(response, "Your favorite color is blue.");

    // Verify the memory search tool returned content
    let recorded = calls.lock().unwrap();
    let second_msgs = &recorded[1].messages;
    let tool_msg = second_msgs.iter().find(|m| m.role == "tool").unwrap();
    assert!(!tool_msg.is_error);
    // Should contain either search results or the MEMORY.md content
    assert!(
        tool_msg.content.contains("blue") || tool_msg.content.contains("color"),
        "Memory search should return relevant content: {}",
        tool_msg.content
    );
}

#[tokio::test]
async fn test_memory_search_empty_query_error() {
    let tmp = TempDir::new().unwrap();

    let provider = MockLLMProvider::with_responses(vec![
        tool_response(vec![tool_call(
            "tc1",
            "memory_search",
            json!({"query": ""}),
        )]),
        text_response("No query given."),
    ]);
    let calls = provider.calls.clone();

    let agent = create_test_agent_with(provider, &tmp, TestAgentOverrides::default()).await;
    agent
        .process_direct("Search with empty", "test:mem2", "telegram", "mem2")
        .await
        .unwrap();

    let recorded = calls.lock().unwrap();
    let second_msgs = &recorded[1].messages;
    let tool_msg = second_msgs.iter().find(|m| m.role == "tool").unwrap();
    assert!(tool_msg.is_error);
    assert!(
        tool_msg.content.contains("empty") || tool_msg.content.contains("Missing"),
        "Should error on empty query: {}",
        tool_msg.content
    );
}
