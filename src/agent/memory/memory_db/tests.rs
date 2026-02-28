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

#[test]
fn test_strip_html_tags() {
    let html = "<html><body><h1>Title</h1><p>Some <b>bold</b> text.</p></body></html>";
    let text = super::strip_html_tags(html);
    assert!(text.contains("Title"));
    assert!(text.contains("Some"));
    assert!(text.contains("bold"));
    assert!(text.contains("text."));
    assert!(!text.contains("<h1>"));
    assert!(!text.contains("<p>"));
}

#[test]
fn test_strip_html_tags_empty() {
    assert!(super::strip_html_tags("").is_empty());
}

#[test]
fn test_index_knowledge_directory() {
    let tmp = tempfile::TempDir::new().unwrap();
    let knowledge_dir = tmp.path().join("knowledge");
    std::fs::create_dir_all(&knowledge_dir).unwrap();

    // Create test files
    std::fs::write(
        knowledge_dir.join("faq.md"),
        "## FAQ\n\nQ: What is oxicrab?\n\nA: A multi-channel AI assistant.",
    )
    .unwrap();
    std::fs::write(
        knowledge_dir.join("notes.txt"),
        "Important notes about the project.\n\nSecond paragraph with details.",
    )
    .unwrap();
    std::fs::write(
        knowledge_dir.join("page.html"),
        "<html><body><h1>Reference</h1><p>HTML reference content here.</p></body></html>",
    )
    .unwrap();
    // Non-supported file should be ignored
    std::fs::write(knowledge_dir.join("data.json"), "{}").unwrap();

    let db_path = tmp.path().join("memory.sqlite3");
    let db = super::MemoryDB::new(db_path).unwrap();

    db.index_knowledge_directory(&knowledge_dir).unwrap();

    // Search should find content from all three files
    let results = db.search("oxicrab", 10, None).unwrap();
    assert!(!results.is_empty());
    assert!(results.iter().any(|h| h.source_key == "knowledge:faq.md"));

    let results = db.search("notes project", 10, None).unwrap();
    assert!(
        results
            .iter()
            .any(|h| h.source_key == "knowledge:notes.txt")
    );

    let results = db.search("reference content", 10, None).unwrap();
    assert!(
        results
            .iter()
            .any(|h| h.source_key == "knowledge:page.html")
    );

    // JSON file should not be indexed
    let all_results = db.search("json", 10, None).unwrap();
    assert!(
        all_results
            .iter()
            .all(|h| h.source_key != "knowledge:data.json")
    );
}

#[test]
fn test_index_knowledge_directory_nonexistent() {
    let tmp = tempfile::TempDir::new().unwrap();
    let db_path = tmp.path().join("memory.sqlite3");
    let db = super::MemoryDB::new(db_path).unwrap();

    // Should not error on nonexistent directory
    let result = db.index_knowledge_directory(&tmp.path().join("nonexistent"));
    assert!(result.is_ok());
}

// ── DLQ tests ────────────────────────────────────────────

#[test]
fn test_dlq_insert_and_list() {
    let dir = tempfile::tempdir().unwrap();
    let db = MemoryDB::new(dir.path().join("test.db")).unwrap();

    let id = db
        .insert_dlq_entry("job-1", "daily-report", "{}", "connection timeout")
        .unwrap();
    assert!(id > 0);

    let entries = db.list_dlq_entries(None).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].job_id, "job-1");
    assert_eq!(entries[0].job_name, "daily-report");
    assert_eq!(entries[0].error_message, "connection timeout");
    assert_eq!(entries[0].retry_count, 0);
    assert_eq!(entries[0].status, "pending_retry");
}

#[test]
fn test_dlq_list_with_status_filter() {
    let dir = tempfile::tempdir().unwrap();
    let db = MemoryDB::new(dir.path().join("test.db")).unwrap();

    let id1 = db.insert_dlq_entry("j1", "a", "{}", "err1").unwrap();
    db.insert_dlq_entry("j2", "b", "{}", "err2").unwrap();

    db.update_dlq_status(id1, "replayed").unwrap();

    let pending = db.list_dlq_entries(Some("pending_retry")).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].job_id, "j2");

    let replayed = db.list_dlq_entries(Some("replayed")).unwrap();
    assert_eq!(replayed.len(), 1);
    assert_eq!(replayed[0].job_id, "j1");
}

#[test]
fn test_dlq_update_status() {
    let dir = tempfile::tempdir().unwrap();
    let db = MemoryDB::new(dir.path().join("test.db")).unwrap();

    let id = db.insert_dlq_entry("j1", "test", "{}", "err").unwrap();
    assert!(db.update_dlq_status(id, "failed_replay").unwrap());

    let entries = db.list_dlq_entries(None).unwrap();
    assert_eq!(entries[0].status, "failed_replay");

    // Non-existent ID returns false
    assert!(!db.update_dlq_status(9999, "replayed").unwrap());
}

#[test]
fn test_dlq_increment_retry() {
    let dir = tempfile::tempdir().unwrap();
    let db = MemoryDB::new(dir.path().join("test.db")).unwrap();

    let id = db.insert_dlq_entry("j1", "test", "{}", "err").unwrap();
    assert_eq!(db.list_dlq_entries(None).unwrap()[0].retry_count, 0);

    db.increment_dlq_retry(id).unwrap();
    assert_eq!(db.list_dlq_entries(None).unwrap()[0].retry_count, 1);

    db.increment_dlq_retry(id).unwrap();
    db.increment_dlq_retry(id).unwrap();
    assert_eq!(db.list_dlq_entries(None).unwrap()[0].retry_count, 3);

    // Non-existent ID returns false
    assert!(!db.increment_dlq_retry(9999).unwrap());
}

#[test]
fn test_dlq_clear_all() {
    let dir = tempfile::tempdir().unwrap();
    let db = MemoryDB::new(dir.path().join("test.db")).unwrap();

    db.insert_dlq_entry("j1", "a", "{}", "e1").unwrap();
    db.insert_dlq_entry("j2", "b", "{}", "e2").unwrap();
    db.insert_dlq_entry("j3", "c", "{}", "e3").unwrap();

    let deleted = db.clear_dlq(None).unwrap();
    assert_eq!(deleted, 3);
    assert!(db.list_dlq_entries(None).unwrap().is_empty());
}

#[test]
fn test_dlq_clear_by_status() {
    let dir = tempfile::tempdir().unwrap();
    let db = MemoryDB::new(dir.path().join("test.db")).unwrap();

    let id1 = db.insert_dlq_entry("j1", "a", "{}", "e1").unwrap();
    db.insert_dlq_entry("j2", "b", "{}", "e2").unwrap();

    db.update_dlq_status(id1, "replayed").unwrap();

    let deleted = db.clear_dlq(Some("replayed")).unwrap();
    assert_eq!(deleted, 1);

    let remaining = db.list_dlq_entries(None).unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].job_id, "j2");
}

#[test]
fn test_dlq_retention_cap() {
    let dir = tempfile::tempdir().unwrap();
    let db = MemoryDB::new(dir.path().join("test.db")).unwrap();

    // Insert 105 entries — only 100 most recent should survive
    for i in 0..105 {
        db.insert_dlq_entry(&format!("j{i}"), &format!("name{i}"), "{}", "err")
            .unwrap();
    }

    let entries = db.list_dlq_entries(None).unwrap();
    assert_eq!(entries.len(), 100);

    // Oldest entries (j0-j4) should have been purged
    let ids: Vec<&str> = entries.iter().map(|e| e.job_id.as_str()).collect();
    assert!(!ids.contains(&"j0"));
    assert!(!ids.contains(&"j4"));
    // Most recent should still be present
    assert!(ids.contains(&"j104"));
}

// ── Recency decay tests ───────────────────────────────────

#[test]
fn test_recency_decay_fresh_entry() {
    // Age 0 → multiplier is 1.0
    let decay = recency_decay(0.0, 90);
    assert!((decay - 1.0).abs() < 1e-6);
}

#[test]
fn test_recency_decay_one_half_life() {
    // At exactly one half-life (90 days), multiplier should be 0.5
    let decay = recency_decay(90.0, 90);
    assert!((decay - 0.5).abs() < 1e-6);
}

#[test]
fn test_recency_decay_two_half_lives() {
    // At two half-lives (180 days), multiplier should be 0.25
    let decay = recency_decay(180.0, 90);
    assert!((decay - 0.25).abs() < 1e-6);
}

#[test]
fn test_recency_decay_disabled_when_zero() {
    // half_life_days = 0 disables decay
    let decay = recency_decay(365.0, 0);
    assert!((decay - 1.0).abs() < 1e-6);
}

#[test]
fn test_recency_decay_negative_age() {
    // Negative age (future timestamp) treated as fresh
    let decay = recency_decay(-10.0, 90);
    assert!((decay - 1.0).abs() < 1e-6);
}

#[test]
fn test_recency_decay_very_old_entry() {
    // 360 days (4 half-lives) → 0.5^4 = 0.0625
    let decay = recency_decay(360.0, 90);
    assert!((decay - 0.0625).abs() < 1e-5);
}

#[test]
fn test_recency_decay_short_half_life() {
    // 7-day half-life: at 7 days → 0.5, at 14 days → 0.25
    assert!((recency_decay(7.0, 7) - 0.5).abs() < 1e-6);
    assert!((recency_decay(14.0, 7) - 0.25).abs() < 1e-6);
}

#[test]
fn test_recency_decay_preserves_relevance_ordering() {
    // A highly relevant old entry (score 0.9) should still beat
    // a low-relevance recent entry (score 0.2) even with decay
    let old_score = 0.9 * recency_decay(180.0, 90); // 0.9 * 0.25 = 0.225
    let recent_score = 0.2 * recency_decay(1.0, 90); // 0.2 * ~1.0 = ~0.2
    assert!(
        old_score > recent_score,
        "highly relevant old entry ({}) should still beat low-relevance recent entry ({})",
        old_score,
        recent_score
    );
}

// ── Workspace file manifest tests ─────────────────────────

#[test]
fn test_workspace_file_register_and_list() {
    let dir = tempfile::tempdir().unwrap();
    let db = MemoryDB::new(dir.path().join("test.db")).unwrap();

    db.register_workspace_file(
        "workspace/images/screenshot.png",
        "images",
        Some("screenshot.png"),
        12345,
        Some("image_gen"),
        Some("session-abc"),
    )
    .unwrap();

    let files = db.list_workspace_files(Some("images"), None, None).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "workspace/images/screenshot.png");
    assert_eq!(files[0].category, "images");
    assert_eq!(files[0].original_name.as_deref(), Some("screenshot.png"));
    assert_eq!(files[0].size_bytes, 12345);
    assert_eq!(files[0].source_tool.as_deref(), Some("image_gen"));
    assert_eq!(files[0].session_key.as_deref(), Some("session-abc"));
    assert!(!files[0].created_at.is_empty());

    // Listing a different category returns nothing
    let empty = db
        .list_workspace_files(Some("documents"), None, None)
        .unwrap();
    assert!(empty.is_empty());
}

#[test]
fn test_workspace_file_search_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let db = MemoryDB::new(dir.path().join("test.db")).unwrap();

    db.register_workspace_file(
        "workspace/images/photo.png",
        "images",
        Some("vacation_photo.png"),
        1000,
        None,
        None,
    )
    .unwrap();
    db.register_workspace_file(
        "workspace/documents/report.pdf",
        "documents",
        Some("quarterly_report.pdf"),
        5000,
        None,
        None,
    )
    .unwrap();

    // Search by partial original name
    let results = db.search_workspace_files("vacation").unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].path, "workspace/images/photo.png");

    // Search by partial path
    let results = db.search_workspace_files("documents").unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].path, "workspace/documents/report.pdf");

    // Search matching nothing
    let results = db.search_workspace_files("nonexistent").unwrap();
    assert!(results.is_empty());
}

#[test]
fn test_workspace_file_unregister() {
    let dir = tempfile::tempdir().unwrap();
    let db = MemoryDB::new(dir.path().join("test.db")).unwrap();

    db.register_workspace_file("workspace/tmp/file.txt", "tmp", None, 100, None, None)
        .unwrap();

    let files = db.list_workspace_files(None, None, None).unwrap();
    assert_eq!(files.len(), 1);

    db.unregister_workspace_file("workspace/tmp/file.txt")
        .unwrap();

    let files = db.list_workspace_files(None, None, None).unwrap();
    assert!(files.is_empty());
}

#[test]
fn test_workspace_file_update_accessed_at() {
    let dir = tempfile::tempdir().unwrap();
    let db = MemoryDB::new(dir.path().join("test.db")).unwrap();

    db.register_workspace_file("workspace/data/file.csv", "data", None, 500, None, None)
        .unwrap();

    let files = db.list_workspace_files(None, None, None).unwrap();
    assert!(files[0].accessed_at.is_none());

    db.touch_workspace_file("workspace/data/file.csv").unwrap();

    let files = db.list_workspace_files(None, None, None).unwrap();
    assert!(files[0].accessed_at.is_some());
}

#[test]
fn test_workspace_file_update_tags() {
    let dir = tempfile::tempdir().unwrap();
    let db = MemoryDB::new(dir.path().join("test.db")).unwrap();

    db.register_workspace_file(
        "workspace/images/chart.png",
        "images",
        None,
        2000,
        None,
        None,
    )
    .unwrap();

    db.set_workspace_file_tags("workspace/images/chart.png", "important,project-x")
        .unwrap();

    // List with tag filter
    let files = db
        .list_workspace_files(None, None, Some("important"))
        .unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].tags, "important,project-x");

    // Tag filter that doesn't match
    let files = db
        .list_workspace_files(None, None, Some("archived"))
        .unwrap();
    assert!(files.is_empty());

    // Substring false positive: "port" should NOT match "important"
    let files = db.list_workspace_files(None, None, Some("port")).unwrap();
    assert!(files.is_empty());

    // Exact tag match for "project-x" should work
    let files = db
        .list_workspace_files(None, None, Some("project-x"))
        .unwrap();
    assert_eq!(files.len(), 1);
}

#[test]
fn test_workspace_file_list_expired() {
    let dir = tempfile::tempdir().unwrap();
    let db = MemoryDB::new(dir.path().join("test.db")).unwrap();

    // Register with an old date (60 days ago)
    db.register_workspace_file_with_date(
        "workspace/tmp/old_file.txt",
        "tmp",
        None,
        100,
        None,
        None,
        "2020-01-01 00:00:00",
    )
    .unwrap();

    // Register a recent file
    db.register_workspace_file("workspace/tmp/new_file.txt", "tmp", None, 200, None, None)
        .unwrap();

    // TTL of 30 days — the old file should be expired
    let expired = db.list_expired_workspace_files("tmp", 30).unwrap();
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].path, "workspace/tmp/old_file.txt");

    // Different category should return nothing
    let expired = db.list_expired_workspace_files("images", 30).unwrap();
    assert!(expired.is_empty());
}

#[test]
fn test_workspace_file_move() {
    let dir = tempfile::tempdir().unwrap();
    let db = MemoryDB::new(dir.path().join("test.db")).unwrap();

    db.register_workspace_file("workspace/tmp/draft.txt", "tmp", None, 300, None, None)
        .unwrap();

    db.move_workspace_file(
        "workspace/tmp/draft.txt",
        "workspace/documents/final.txt",
        "documents",
    )
    .unwrap();

    // Old path should be gone
    let results = db.search_workspace_files("draft.txt").unwrap();
    assert!(results.is_empty());

    // New path should exist with updated category
    let files = db
        .list_workspace_files(Some("documents"), None, None)
        .unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "workspace/documents/final.txt");
    assert_eq!(files[0].category, "documents");
}

#[test]
fn test_workspace_file_register_upsert() {
    let dir = tempfile::tempdir().unwrap();
    let db = MemoryDB::new(dir.path().join("test.db")).unwrap();

    db.register_workspace_file(
        "workspace/images/photo.png",
        "images",
        Some("photo.png"),
        1000,
        Some("tool_a"),
        None,
    )
    .unwrap();

    // Set tags and touch the file before the second register
    db.set_workspace_file_tags("workspace/images/photo.png", "important,review")
        .unwrap();
    db.touch_workspace_file("workspace/images/photo.png")
        .unwrap();

    let before = db.list_workspace_files(None, None, None).unwrap();
    let original_id = before[0].id;
    assert_eq!(before[0].tags, "important,review");
    assert!(before[0].accessed_at.is_some());

    // Register again with different size — should update, not duplicate
    db.register_workspace_file(
        "workspace/images/photo.png",
        "images",
        Some("photo_v2.png"),
        2000,
        Some("tool_b"),
        None,
    )
    .unwrap();

    let files = db.list_workspace_files(None, None, None).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].size_bytes, 2000);
    assert_eq!(files[0].original_name.as_deref(), Some("photo_v2.png"));
    assert_eq!(files[0].source_tool.as_deref(), Some("tool_b"));

    // Verify tags, accessed_at, and id are preserved across upsert
    assert_eq!(files[0].id, original_id);
    assert_eq!(files[0].tags, "important,review");
    assert!(files[0].accessed_at.is_some());
}
