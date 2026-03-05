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
    let terms: Vec<String> = (0..30).map(|i| format!("word{i}")).collect();
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
fn test_insert_memory_and_search() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_memory.db");
    let db = MemoryDB::new(&db_path).unwrap();

    db.insert_memory("notes.md", "This is a test document about Rust programming")
        .unwrap();
    db.insert_memory(
        "notes.md",
        "Another paragraph about async runtime and tokio",
    )
    .unwrap();

    let results = db.search("Rust programming", 10, None).unwrap();
    assert!(!results.is_empty());
    assert!(results[0].content.contains("Rust"));
}

#[test]
fn test_search_no_results() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_memory.db");
    let db = MemoryDB::new(&db_path).unwrap();

    db.insert_memory("notes.md", "This document is about cooking recipes")
        .unwrap();
    db.insert_memory("notes.md", "Another paragraph about food")
        .unwrap();

    let results = db.search("quantum physics", 10, None).unwrap();
    assert!(results.is_empty());
}

#[test]
fn test_insert_memory_exclude_sources() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_memory.db");
    let db = MemoryDB::new(&db_path).unwrap();

    db.insert_memory("notes1.md", "This is about Rust programming language")
        .unwrap();
    db.insert_memory("notes2.md", "This is also about Rust async patterns")
        .unwrap();

    let mut exclude = std::collections::HashSet::new();
    exclude.insert("notes1.md".to_string());

    let results = db.search("Rust", 10, Some(&exclude)).unwrap();
    // notes1.md should be excluded
    for hit in &results {
        assert_ne!(hit.source_key, "notes1.md");
    }
}

#[test]
fn test_insert_memory_clone_works() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_memory.db");
    let db = MemoryDB::new(&db_path).unwrap();

    db.insert_memory(
        "notes.md",
        "Content about cloning and testing database connections",
    )
    .unwrap();

    let db2 = db.clone();
    let results = db2.search("cloning", 10, None).unwrap();
    assert!(!results.is_empty());
}

#[test]
fn test_search_logging_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_memory.db");
    let db = MemoryDB::new(&db_path).unwrap();

    db.insert_memory(
        "notes.md",
        "This is a document about Rust programming and memory safety",
    )
    .unwrap();

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
    let db = MemoryDB::new(&db_path).unwrap();

    db.insert_memory(
        "notes.md",
        "This is a document about Rust programming and concurrency",
    )
    .unwrap();

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
fn test_entries_missing_embeddings() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_memory.db");
    let db = MemoryDB::new(&db_path).unwrap();

    db.insert_memory("notes.md", "This is about embeddings and vector search")
        .unwrap();
    db.insert_memory("notes.md", "Another paragraph about neural networks")
        .unwrap();

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
fn test_insert_memory_empty_content_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_memory.db");
    let db = MemoryDB::new(&db_path).unwrap();

    // Empty and whitespace-only content should be silently ignored
    db.insert_memory("test", "").unwrap();
    db.insert_memory("test", "   ").unwrap();
    db.insert_memory("test", "\n\t\n").unwrap();

    let entries = db.get_recent_entries("test", 10).unwrap();
    assert!(entries.is_empty());
}

#[test]
fn test_insert_memory_dedup_by_hash() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_memory.db");
    let db = MemoryDB::new(&db_path).unwrap();

    // Insert the same content twice under the same source key
    db.insert_memory("test", "duplicate content here").unwrap();
    db.insert_memory("test", "duplicate content here").unwrap();

    let entries = db.get_recent_entries("test", 10).unwrap();
    assert_eq!(entries.len(), 1, "duplicate content should be ignored");
}

#[test]
fn test_get_recent_entries_order() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_memory.db");
    let db = MemoryDB::new(&db_path).unwrap();

    db.insert_memory("test", "first entry").unwrap();
    // Ensure different timestamps by inserting different content
    db.insert_memory("test", "second entry").unwrap();
    db.insert_memory("test", "third entry").unwrap();

    let entries = db.get_recent_entries("test", 10).unwrap();
    assert_eq!(entries.len(), 3);
    // Newest first (ORDER BY created_at DESC)
    assert_eq!(entries[0], "third entry");
    assert_eq!(entries[2], "first entry");
}

#[test]
fn test_get_recent_entries_limit() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_memory.db");
    let db = MemoryDB::new(&db_path).unwrap();

    for i in 0..10 {
        db.insert_memory("test", &format!("entry number {i}"))
            .unwrap();
    }

    let entries = db.get_recent_entries("test", 3).unwrap();
    assert_eq!(entries.len(), 3, "limit should cap results");
}

#[test]
fn test_token_record_and_summary() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_memory.db");
    let db = MemoryDB::new(&db_path).unwrap();

    db.record_tokens("claude-sonnet-4", 1000, 500, 0, 0, "main", None)
        .unwrap();
    db.record_tokens("claude-sonnet-4", 2000, 1000, 0, 0, "main", None)
        .unwrap();
    db.record_tokens("gpt-4o", 500, 200, 0, 0, "main", None)
        .unwrap();

    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let summary = db.get_token_summary(&today).unwrap();
    assert!(!summary.is_empty());
    // Should have two groups: claude-sonnet-4 and gpt-4o
    assert_eq!(summary.len(), 2);
    // claude-sonnet-4 should be first (more input tokens)
    assert_eq!(summary[0].model, "claude-sonnet-4");
    assert_eq!(summary[0].total_input_tokens, 3000);
    assert_eq!(summary[0].total_output_tokens, 1500);
    assert_eq!(summary[0].call_count, 2);
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
        "highly relevant old entry ({old_score}) should still beat low-relevance recent entry ({recent_score})"
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

#[test]
fn test_record_complexity_event() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("memory.sqlite3");
    let db = MemoryDB::new(&db_path).unwrap();

    db.record_complexity_event(
        "req-test-001",
        0.72,
        "heavy",
        Some("claude-opus-4-6"),
        Some("reasoning_keywords"),
        Some("telegram"),
        "analyze the architectural tradeoffs of event sourcing",
    )
    .unwrap();

    db.record_complexity_event(
        "req-test-002",
        0.15,
        "lightweight",
        Some("claude-haiku-4-5"),
        None,
        Some("whatsapp"),
        "hey what's up",
    )
    .unwrap();

    let conn = db.conn.lock().unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM complexity_routing_log", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 2);

    let tier: String = conn
        .query_row(
            "SELECT resolved_tier FROM complexity_routing_log WHERE request_id = ?",
            ["req-test-001"],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tier, "heavy");
}

#[test]
fn test_get_complexity_stats() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("memory.sqlite3");
    let db = MemoryDB::new(&db_path).unwrap();

    // Insert complexity events
    db.record_complexity_event(
        "req-1",
        0.12,
        "lightweight",
        Some("haiku"),
        None,
        Some("telegram"),
        "hey",
    )
    .unwrap();
    db.record_complexity_event(
        "req-2",
        0.15,
        "lightweight",
        Some("haiku"),
        Some("greeting"),
        Some("whatsapp"),
        "hi there",
    )
    .unwrap();
    db.record_complexity_event(
        "req-3",
        0.45,
        "standard",
        Some("sonnet"),
        None,
        Some("telegram"),
        "explain how async works",
    )
    .unwrap();
    db.record_complexity_event(
        "req-4",
        0.82,
        "heavy",
        Some("opus"),
        Some("reasoning_keywords"),
        Some("discord"),
        "analyze the tradeoffs",
    )
    .unwrap();
    db.record_complexity_event(
        "req-5",
        0.71,
        "heavy",
        Some("opus"),
        None,
        Some("telegram"),
        "compare event sourcing vs cqrs",
    )
    .unwrap();

    // Insert correlated token data
    db.record_tokens("haiku", 100, 50, 0, 0, "main", Some("req-1"))
        .unwrap();
    db.record_tokens("haiku", 120, 60, 0, 0, "main", Some("req-2"))
        .unwrap();
    db.record_tokens("sonnet", 500, 200, 0, 0, "main", Some("req-3"))
        .unwrap();
    db.record_tokens("opus", 800, 400, 0, 0, "main", Some("req-4"))
        .unwrap();
    db.record_tokens("opus", 700, 350, 0, 0, "main", Some("req-5"))
        .unwrap();

    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let stats = db.get_complexity_stats(&today).unwrap();

    assert_eq!(stats.total_scored, 5);
    assert_eq!(stats.tier_counts.len(), 3);

    let light = stats
        .tier_counts
        .iter()
        .find(|t| t.tier == "lightweight")
        .unwrap();
    assert_eq!(light.count, 2);

    let heavy = stats
        .tier_counts
        .iter()
        .find(|t| t.tier == "heavy")
        .unwrap();
    assert_eq!(heavy.count, 2);
    assert!(heavy.avg_score > 0.7);
    assert!(heavy.total_tokens > 2000);

    // Force overrides: greeting + reasoning_keywords
    assert_eq!(stats.force_counts.len(), 2);

    // Recent events
    let recent = db.get_recent_complexity_events("heavy", 5).unwrap();
    assert_eq!(recent.len(), 2);
}
