use super::*;

#[test]
fn test_hash_text_deterministic() {
    let h1 = hash_text("hello world");
    let h2 = hash_text("hello world");
    assert_eq!(h1, h2);
}

#[test]
fn test_hash_text_different_inputs() {
    let h1 = hash_text("hello");
    let h2 = hash_text("world");
    assert_ne!(h1, h2);
}

#[test]
fn test_split_into_chunks_skips_short() {
    let text = "short\n\nalso short\n\nthis is long enough to be a chunk";
    let chunks = split_into_chunks(text);
    // "short" and "also short" are < MIN_CHUNK_SIZE (12), should be skipped
    assert!(!chunks.iter().any(|c| c == "short"));
    assert!(!chunks.iter().any(|c| c == "also short"));
}

#[test]
fn test_split_into_chunks_truncates_long() {
    let long_paragraph = "a".repeat(2000);
    let chunks = split_into_chunks(&long_paragraph);
    assert_eq!(chunks.len(), 1);
    assert!(chunks[0].len() <= MAX_CHUNK_SIZE);
}

#[test]
fn test_split_into_chunks_utf8_safe_truncation() {
    // Create a string of multi-byte chars longer than MAX_CHUNK_SIZE
    let text = "\u{1F600}".repeat(500); // 500 * 4 = 2000 bytes
    let chunks = split_into_chunks(&text);
    // Each chunk should be valid UTF-8
    for chunk in &chunks {
        for c in chunk.chars() {
            assert_eq!(c, '\u{1F600}');
        }
    }
}

#[test]
fn test_split_into_chunks_normal_paragraphs() {
    let text = "This is paragraph one with enough text.\n\nThis is paragraph two with enough text.";
    let chunks = split_into_chunks(text);
    assert_eq!(chunks.len(), 2);
}

#[test]
fn test_split_into_chunks_empty_input() {
    let chunks = split_into_chunks("");
    assert!(chunks.is_empty());
}

#[test]
fn test_fts_query_simple() {
    let q = fts_query("hello world");
    assert!(q.contains("hello"));
    assert!(q.contains("world"));
    assert!(q.contains(" OR "));
}

#[test]
fn test_fts_query_empty() {
    assert_eq!(fts_query(""), "");
}

#[test]
fn test_fts_query_deduplicates() {
    let q = fts_query("hello hello hello");
    // Should only have "hello" once
    assert_eq!(q.matches("hello").count(), 1);
}

#[test]
fn test_fts_query_case_insensitive() {
    let q = fts_query("Hello HELLO hello");
    assert_eq!(q.matches("hello").count(), 1);
}

#[test]
fn test_fts_query_max_terms() {
    let terms: Vec<String> = (0..30).map(|i| format!("word{}", i)).collect();
    let q = fts_query(&terms.join(" "));
    let count = q.split(" OR ").count();
    assert!(count <= MAX_FTS_TERMS);
}

#[test]
fn test_fts_query_symbols_stripped() {
    let q = fts_query("!!! ??? ...");
    assert_eq!(q, "");
}

#[test]
fn test_memory_db_new_creates_schema() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_memory.db");
    let db = MemoryDB::new(&db_path).unwrap();
    // Should be able to search without error
    let results = db.search("anything", 10, None).unwrap();
    assert!(results.is_empty());
}

#[test]
fn test_memory_db_index_and_search() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_memory.db");

    // Write a test file
    let test_file = dir.path().join("notes.md");
    std::fs::write(
        &test_file,
        "This is a test document about Rust programming.\n\n\
         Another paragraph about async runtime and tokio.",
    )
    .unwrap();

    let db = MemoryDB::new(&db_path).unwrap();
    db.index_file("notes.md", &test_file).unwrap();

    let results = db.search("Rust programming", 10, None).unwrap();
    assert!(!results.is_empty());
    assert!(results[0].content.contains("Rust"));
}

#[test]
fn test_memory_db_search_no_results() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_memory.db");

    let test_file = dir.path().join("notes.md");
    std::fs::write(
        &test_file,
        "This document is about cooking recipes.\n\nAnother paragraph about food.",
    )
    .unwrap();

    let db = MemoryDB::new(&db_path).unwrap();
    db.index_file("notes.md", &test_file).unwrap();

    let results = db.search("quantum physics", 10, None).unwrap();
    assert!(results.is_empty());
}

#[test]
fn test_memory_db_exclude_sources() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_memory.db");

    let file1 = dir.path().join("notes1.md");
    std::fs::write(&file1, "This is about Rust programming language.").unwrap();
    let file2 = dir.path().join("notes2.md");
    std::fs::write(&file2, "This is also about Rust async patterns.").unwrap();

    let db = MemoryDB::new(&db_path).unwrap();
    db.index_file("notes1.md", &file1).unwrap();
    db.index_file("notes2.md", &file2).unwrap();

    let mut exclude = std::collections::HashSet::new();
    exclude.insert("notes1.md".to_string());

    let results = db.search("Rust", 10, Some(&exclude)).unwrap();
    // notes1.md should be excluded
    for hit in &results {
        assert_ne!(hit.source_key, "notes1.md");
    }
}

#[test]
fn test_memory_db_reindex_unchanged_file_is_noop() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_memory.db");

    let test_file = dir.path().join("notes.md");
    std::fs::write(
        &test_file,
        "Content about database indexing and memory systems.",
    )
    .unwrap();

    let db = MemoryDB::new(&db_path).unwrap();
    db.index_file("notes.md", &test_file).unwrap();
    // Indexing the same unchanged file again should be a no-op
    db.index_file("notes.md", &test_file).unwrap();

    let results = db.search("database", 10, None).unwrap();
    assert!(!results.is_empty());
}

#[test]
fn test_memory_db_index_directory() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_memory.db");
    let memory_dir = dir.path().join("memory");
    std::fs::create_dir(&memory_dir).unwrap();

    std::fs::write(
        memory_dir.join("file1.md"),
        "This file is about artificial intelligence and machine learning.",
    )
    .unwrap();
    std::fs::write(
        memory_dir.join("file2.md"),
        "This file is about web development and JavaScript frameworks.",
    )
    .unwrap();
    // Non-md file should be ignored
    std::fs::write(memory_dir.join("file3.txt"), "This should be ignored.").unwrap();

    let db = MemoryDB::new(&db_path).unwrap();
    db.index_directory(&memory_dir).unwrap();

    let results = db.search("artificial intelligence", 10, None).unwrap();
    assert!(!results.is_empty());
}

#[test]
fn test_memory_db_clone_works() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_memory.db");
    let db = MemoryDB::new(&db_path).unwrap();

    let test_file = dir.path().join("notes.md");
    std::fs::write(
        &test_file,
        "Content about cloning and testing database connections.",
    )
    .unwrap();
    db.index_file("notes.md", &test_file).unwrap();

    let db2 = db.clone();
    let results = db2.search("cloning", 10, None).unwrap();
    assert!(!results.is_empty());
}

#[test]
fn test_search_logging_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_memory.db");

    let test_file = dir.path().join("notes.md");
    std::fs::write(
        &test_file,
        "This is a document about Rust programming and memory safety.",
    )
    .unwrap();

    let db = MemoryDB::new(&db_path).unwrap();
    db.index_file("notes.md", &test_file).unwrap();

    // search() internally calls log_search()
    let results = db.search("Rust programming", 10, None).unwrap();
    assert!(!results.is_empty());

    let stats = db.get_search_stats().unwrap();
    assert_eq!(stats.total_searches, 1);
    assert!(stats.total_hits > 0);
}

#[test]
fn test_source_hit_count() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_memory.db");

    let test_file = dir.path().join("notes.md");
    std::fs::write(
        &test_file,
        "This is a document about Rust programming and concurrency.",
    )
    .unwrap();

    let db = MemoryDB::new(&db_path).unwrap();
    db.index_file("notes.md", &test_file).unwrap();

    // Before any search, hit count should be 0
    assert_eq!(db.get_source_hit_count("notes.md").unwrap(), 0);

    // Search triggers a log
    let _ = db.search("Rust", 10, None).unwrap();

    // Now notes.md should have hits
    let count = db.get_source_hit_count("notes.md").unwrap();
    assert!(count > 0);

    // Search again
    let _ = db.search("programming", 10, None).unwrap();
    let count2 = db.get_source_hit_count("notes.md").unwrap();
    assert!(count2 >= count);
}

#[test]
fn test_cost_record_and_query() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_memory.db");
    let db = MemoryDB::new(&db_path).unwrap();

    db.record_cost("claude-sonnet-4", 1000, 500, 0, 0, 4.5, "main")
        .unwrap();
    db.record_cost("gpt-4o", 2000, 1000, 100, 200, 3.2, "subagent")
        .unwrap();

    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let daily = db.get_daily_cost(&today).unwrap();
    assert!((daily - 7.7).abs() < 0.01, "expected 7.7, got {}", daily);
}

#[test]
fn test_cost_summary() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_memory.db");
    let db = MemoryDB::new(&db_path).unwrap();

    db.record_cost("claude-sonnet-4", 1000, 500, 0, 0, 4.5, "main")
        .unwrap();
    db.record_cost("claude-sonnet-4", 2000, 1000, 0, 0, 9.0, "main")
        .unwrap();
    db.record_cost("gpt-4o", 500, 200, 0, 0, 1.0, "main")
        .unwrap();

    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let summary = db.get_cost_summary(&today).unwrap();
    assert!(!summary.is_empty());
    // Should have two groups: claude-sonnet-4 and gpt-4o
    assert_eq!(summary.len(), 2);
}

#[test]
fn test_entries_missing_embeddings() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_memory.db");

    let test_file = dir.path().join("notes.md");
    std::fs::write(
        &test_file,
        "This is about embeddings and vector search.\n\nAnother paragraph about neural networks.",
    )
    .unwrap();

    let db = MemoryDB::new(&db_path).unwrap();
    db.index_file("notes.md", &test_file).unwrap();

    let missing = db.get_entries_missing_embeddings().unwrap();
    assert!(!missing.is_empty());

    // Store an embedding for the first entry
    let (entry_id, _, _) = &missing[0];
    let fake_embedding = vec![0u8; 128];
    db.store_embedding(*entry_id, &fake_embedding).unwrap();

    // Now one fewer should be missing
    let missing_after = db.get_entries_missing_embeddings().unwrap();
    assert_eq!(missing_after.len(), missing.len() - 1);
}

#[test]
fn test_fusion_strategy_default_is_weighted_score() {
    assert_eq!(
        crate::config::FusionStrategy::default(),
        crate::config::FusionStrategy::WeightedScore
    );
}

#[test]
fn test_fusion_strategy_serde_roundtrip() {
    let rrf = crate::config::FusionStrategy::Rrf;
    let json = serde_json::to_string(&rrf).unwrap();
    assert_eq!(json, "\"rrf\"");
    let ws = crate::config::FusionStrategy::WeightedScore;
    let json = serde_json::to_string(&ws).unwrap();
    assert_eq!(json, "\"weighted_score\"");

    let parsed: crate::config::FusionStrategy = serde_json::from_str("\"rrf\"").unwrap();
    assert_eq!(parsed, crate::config::FusionStrategy::Rrf);
}
