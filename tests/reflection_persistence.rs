//! Integration test for Track 1: reflection persistence.
//!
//! The reflection LLM call itself is hard to integration-test without
//! a mock provider plumbed through the full agent loop. The unit tests
//! in `src/agent/loop/reflection.rs` cover budget/parse logic. This
//! test focuses on the persistence pathway: given a `ReflectionRecord`,
//! it lands in `tool_reflections` and round-trips correctly.

use oxicrab::agent::memory::memory_db::{MemoryDB, ReflectionRecord};

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
