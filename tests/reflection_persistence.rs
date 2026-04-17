//! Integration test for Track 1: reflection persistence.
//!
//! The reflection LLM call itself is hard to integration-test without
//! a mock provider plumbed through the full agent loop. The unit tests
//! in `src/agent/loop/reflection.rs` cover budget/parse logic. This
//! test focuses on the persistence pathway: given a `ReflectionRecord`,
//! it lands in `tool_reflections` and round-trips correctly.

use oxicrab::agent::memory::memory_db::{
    MemoryDB, ReflectionRecord, ReflectionStatRow, SkillIndexEntry,
};

#[test]
fn reflection_record_persists_and_counts() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("reflection.db");
    let db = MemoryDB::new(&db_path).unwrap();

    let rec = ReflectionRecord {
        request_id: "req-1".to_string(),
        tool_name: "shell".to_string(),
        action: Some("execute".to_string()),
        attempt_number: 1,
        error_excerpt: "command not found: foo".to_string(),
        hypothesis: "binary missing from PATH".to_string(),
        retry_strategy: "use full path or install foo".to_string(),
        next_outcome: None,
        created_at_ms: 1_000_000,
    };
    db.insert_tool_reflection(&rec).unwrap();
    assert_eq!(db.count_reflections_for_request("req-1").unwrap(), 1);

    db.insert_tool_reflection(&ReflectionRecord {
        attempt_number: 2,
        ..rec.clone()
    })
    .unwrap();
    assert_eq!(db.count_reflections_for_request("req-1").unwrap(), 2);
    assert_eq!(db.count_reflections_for_request("req-other").unwrap(), 0);
}

#[test]
fn reflection_outcome_update_handles_null_action() {
    let dir = tempfile::tempdir().unwrap();
    let db = MemoryDB::new(dir.path().join("r.db")).unwrap();
    db.insert_tool_reflection(&ReflectionRecord {
        request_id: "req-A".to_string(),
        tool_name: "shell".to_string(),
        action: None,
        attempt_number: 1,
        error_excerpt: "boom".to_string(),
        hypothesis: "h".to_string(),
        retry_strategy: "r".to_string(),
        next_outcome: None,
        created_at_ms: 1,
    })
    .unwrap();
    db.update_reflection_outcome("req-A", "shell", None, "success")
        .unwrap();
    let conn = db.lock_conn().unwrap();
    let outcome: Option<String> = conn
        .query_row(
            "SELECT next_outcome FROM tool_reflections WHERE request_id = 'req-A'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(outcome.as_deref(), Some("success"));
}

#[test]
fn skill_index_roundtrip_preserves_embedding() {
    let dir = tempfile::tempdir().unwrap();
    let db = MemoryDB::new(dir.path().join("s.db")).unwrap();
    let entry = SkillIndexEntry {
        path: "/tmp/skill.md".to_string(),
        name: "skill".to_string(),
        description: "desc".to_string(),
        embedding: vec![0.1, 0.2, -0.3, 1.5, f32::MAX],
        file_sha256: "abc".to_string(),
        embedding_model_id: "test-model".to_string(),
        use_count: 0,
        last_used_ms: None,
        created_at_ms: 1000,
        last_indexed_ms: 1000,
    };
    db.upsert_skill_index(&entry).unwrap();
    let back = db.list_skill_index_entries().unwrap();
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].embedding, entry.embedding);
}

#[test]
fn skill_index_prune_unused_respects_min_uses() {
    let dir = tempfile::tempdir().unwrap();
    let db = MemoryDB::new(dir.path().join("s.db")).unwrap();
    let now_ms = 100_000_000_i64;
    let day = 86_400_000_i64;
    // Old & unused — should be pruned.
    db.upsert_skill_index(&SkillIndexEntry {
        path: "old_unused.md".into(),
        name: "old_unused".into(),
        description: "x".into(),
        embedding: vec![0.0],
        file_sha256: "a".into(),
        embedding_model_id: "test-model".into(),
        use_count: 0,
        last_used_ms: None,
        created_at_ms: now_ms - 31 * day,
        last_indexed_ms: now_ms - 31 * day,
    })
    .unwrap();
    // Old & used — should be kept.
    db.upsert_skill_index(&SkillIndexEntry {
        path: "old_used.md".into(),
        name: "old_used".into(),
        description: "x".into(),
        embedding: vec![0.0],
        file_sha256: "b".into(),
        embedding_model_id: "test-model".into(),
        use_count: 5,
        last_used_ms: Some(now_ms - 1),
        created_at_ms: now_ms - 31 * day,
        last_indexed_ms: now_ms - 31 * day,
    })
    .unwrap();
    // Young & unused — should be kept.
    db.upsert_skill_index(&SkillIndexEntry {
        path: "young_unused.md".into(),
        name: "young_unused".into(),
        description: "x".into(),
        embedding: vec![0.0],
        file_sha256: "c".into(),
        embedding_model_id: "test-model".into(),
        use_count: 0,
        last_used_ms: None,
        created_at_ms: now_ms - 1,
        last_indexed_ms: now_ms - 1,
    })
    .unwrap();

    let dropped = db.prune_unused_skill_index(now_ms, 30 * day, 1).unwrap();
    assert_eq!(dropped, vec!["old_unused.md".to_string()]);
    let remaining: Vec<String> = db
        .list_skill_index_entries()
        .unwrap()
        .into_iter()
        .map(|e| e.path)
        .collect();
    assert!(remaining.contains(&"old_used.md".to_string()));
    assert!(remaining.contains(&"young_unused.md".to_string()));
    assert!(!remaining.contains(&"old_unused.md".to_string()));
}

#[test]
fn reflection_stats_aggregates_per_tool_action() {
    let dir = tempfile::tempdir().unwrap();
    let db = MemoryDB::new(dir.path().join("r.db")).unwrap();
    let now_ms = chrono::Utc::now().timestamp_millis();

    for (i, outcome) in ["success", "success", "error"].iter().enumerate() {
        db.insert_tool_reflection(&ReflectionRecord {
            request_id: format!("req-{i}"),
            tool_name: "shell".to_string(),
            action: Some("execute".to_string()),
            attempt_number: 1,
            error_excerpt: "x".to_string(),
            hypothesis: "h".to_string(),
            retry_strategy: "r".to_string(),
            next_outcome: None,
            created_at_ms: now_ms,
        })
        .unwrap();
        db.update_reflection_outcome(&format!("req-{i}"), "shell", Some("execute"), outcome)
            .unwrap();
    }
    db.insert_tool_reflection(&ReflectionRecord {
        request_id: "req-gh".to_string(),
        tool_name: "github".to_string(),
        action: Some("list_prs".to_string()),
        attempt_number: 1,
        error_excerpt: "x".to_string(),
        hypothesis: "h".to_string(),
        retry_strategy: "r".to_string(),
        next_outcome: None,
        created_at_ms: now_ms,
    })
    .unwrap();

    let rows: Vec<ReflectionStatRow> = db.reflection_stats(7, 1).unwrap();
    assert_eq!(rows.len(), 2);
    let shell = rows
        .iter()
        .find(|r| r.tool_name == "shell")
        .expect("shell row");
    assert_eq!(shell.total, 3);
    assert_eq!(shell.successes, 2);
    assert_eq!(shell.errors, 1);
    assert_eq!(shell.pending, 0);
    assert!((shell.failure_rate().unwrap() - 1.0 / 3.0).abs() < 1e-6);
    let gh = rows
        .iter()
        .find(|r| r.tool_name == "github")
        .expect("github row");
    assert_eq!(gh.pending, 1);
    assert!(gh.failure_rate().is_none());

    let filtered = db.reflection_stats(7, 2).unwrap();
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].tool_name, "shell");
}

#[test]
fn invalidate_skill_index_for_model_drops_other_models() {
    let dir = tempfile::tempdir().unwrap();
    let db = MemoryDB::new(dir.path().join("s.db")).unwrap();
    for (path, model) in [
        ("a.md", "old-model"),
        ("b.md", "new-model"),
        ("c.md", "old-model"),
    ] {
        db.upsert_skill_index(&SkillIndexEntry {
            path: path.into(),
            name: path.to_string(),
            description: "x".into(),
            embedding: vec![0.0],
            file_sha256: "h".into(),
            embedding_model_id: model.to_string(),
            use_count: 0,
            last_used_ms: None,
            created_at_ms: 1,
            last_indexed_ms: 1,
        })
        .unwrap();
    }
    let n = db.invalidate_skill_index_for_model("new-model").unwrap();
    assert_eq!(n, 2);
    let remaining: Vec<String> = db
        .list_skill_index_entries()
        .unwrap()
        .into_iter()
        .map(|e| e.path)
        .collect();
    assert_eq!(remaining, vec!["b.md".to_string()]);
}

#[test]
fn skill_index_prune_skill_index_drops_dead_paths() {
    use std::collections::HashSet;
    let dir = tempfile::tempdir().unwrap();
    let db = MemoryDB::new(dir.path().join("s.db")).unwrap();
    for path in ["live.md", "dead.md"] {
        db.upsert_skill_index(&SkillIndexEntry {
            path: path.into(),
            name: path.to_string(),
            description: "x".into(),
            embedding: vec![0.0],
            file_sha256: "a".into(),
            embedding_model_id: "test-model".into(),
            use_count: 0,
            last_used_ms: None,
            created_at_ms: 1,
            last_indexed_ms: 1,
        })
        .unwrap();
    }
    let live: HashSet<String> = ["live.md".to_string()].into_iter().collect();
    let dropped = db.prune_skill_index(&live).unwrap();
    assert_eq!(dropped, 1);
    let remaining: Vec<String> = db
        .list_skill_index_entries()
        .unwrap()
        .into_iter()
        .map(|e| e.path)
        .collect();
    assert_eq!(remaining, vec!["live.md".to_string()]);
}

#[test]
fn reflection_outcome_update_targets_latest_record() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("reflection.db");
    let db = MemoryDB::new(&db_path).unwrap();

    let base = ReflectionRecord {
        request_id: "req-X".to_string(),
        tool_name: "github".to_string(),
        action: Some("get_pr".to_string()),
        attempt_number: 1,
        error_excerpt: "not found".to_string(),
        hypothesis: "wrong number".to_string(),
        retry_strategy: "verify PR id".to_string(),
        next_outcome: None,
        created_at_ms: 1,
    };
    db.insert_tool_reflection(&base).unwrap();
    db.insert_tool_reflection(&ReflectionRecord {
        attempt_number: 2,
        created_at_ms: 2,
        ..base.clone()
    })
    .unwrap();

    db.update_reflection_outcome("req-X", "github", Some("get_pr"), "success")
        .unwrap();

    // Only one row should now have a non-null outcome.
    let conn = db.lock_conn().unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM tool_reflections WHERE next_outcome IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    let outcome: String = conn
        .query_row(
            "SELECT next_outcome FROM tool_reflections WHERE id = (SELECT MAX(id) FROM tool_reflections)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(outcome, "success");
}
