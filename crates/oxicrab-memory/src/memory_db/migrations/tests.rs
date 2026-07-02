use super::*;

#[test]
fn applies_latest_user_version() {
    let conn = Connection::open_in_memory().unwrap();
    apply_migrations(&conn).unwrap();
    let v: u32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(v, 15);
}

#[test]
fn adds_request_id_columns_when_missing() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE llm_cost_log (
            id INTEGER PRIMARY KEY,
            timestamp TEXT NOT NULL DEFAULT (datetime('now')),
            model TEXT NOT NULL,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
            cache_read_tokens INTEGER NOT NULL DEFAULT 0,
            cost_cents REAL NOT NULL,
            caller TEXT NOT NULL DEFAULT 'main'
        );
         CREATE TABLE memory_access_log (
            id INTEGER PRIMARY KEY,
            query TEXT NOT NULL,
            search_type TEXT NOT NULL,
            result_count INTEGER NOT NULL,
            top_score REAL,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );",
    )
    .unwrap();
    apply_migrations(&conn).unwrap();

    for table in ["llm_cost_log", "memory_access_log"] {
        let mut stmt = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap();
        let cols: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        assert!(cols.iter().any(|c| c == "request_id"));
    }
}

#[test]
fn migration_12_drops_cost_cents_and_intent_metrics() {
    let conn = Connection::open_in_memory().unwrap();
    apply_migrations(&conn).unwrap();
    let cols: Vec<String> = conn
        .prepare("PRAGMA table_info(llm_cost_log)")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert!(
        !cols.iter().any(|c| c == "cost_cents"),
        "cost_cents should be dropped, got: {cols:?}"
    );
    let intent_exists = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='intent_metrics'",
            [],
            |_| Ok(()),
        )
        .is_ok();
    assert!(!intent_exists, "intent_metrics table should be dropped");
}

#[test]
fn test_migration_v3_adds_source_key_index() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(MIGRATION_0001_BASE).unwrap();
    conn.execute("PRAGMA user_version = 2", []).unwrap();
    apply_migrations(&conn).unwrap();
    assert!(user_version(&conn).unwrap() >= 3);
    // Verify index exists
    let mut stmt = conn.prepare("PRAGMA index_list('memory_entries')").unwrap();
    let indexes: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    assert!(
        indexes.iter().any(|n| n.contains("source_key")),
        "source_key index should exist, found: {indexes:?}",
    );
}

#[test]
fn test_migration_v5_creates_rss_tables() {
    let conn = Connection::open_in_memory().unwrap();
    apply_migrations(&conn).unwrap();
    let v = user_version(&conn).unwrap();
    assert!(v >= 5, "expected version >= 5, got {v}");

    for table in [
        "rss_feeds",
        "rss_articles",
        "rss_article_tags",
        "rss_profile",
        "rss_model",
    ] {
        let count: i64 = conn
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='{table}'"
                ),
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "table {table} should exist");
    }

    for idx in [
        "idx_rss_articles_feed",
        "idx_rss_articles_status",
        "idx_rss_articles_published",
        "idx_rss_article_tags_tag",
    ] {
        let count: i64 = conn
            .query_row(
                &format!("SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='{idx}'"),
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "index {idx} should exist");
    }
}

#[test]
fn test_migration_v4_adds_sessions_updated_index() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(MIGRATION_0001_BASE).unwrap();
    conn.execute("PRAGMA user_version = 3", []).unwrap();
    apply_migrations(&conn).unwrap();
    assert!(user_version(&conn).unwrap() >= 4);
    // Verify index exists
    let mut stmt = conn.prepare("PRAGMA index_list('sessions')").unwrap();
    let indexes: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    assert!(
        indexes.iter().any(|n| n.contains("sessions_updated")),
        "sessions updated_at index should exist, found: {indexes:?}",
    );
}

#[test]
fn test_migration_v11_creates_claims_and_evidence_tables() {
    let conn = Connection::open_in_memory().unwrap();
    apply_migrations(&conn).unwrap();
    let v = user_version(&conn).unwrap();
    assert!(v >= 11, "expected version >= 11, got {v}");

    for table in ["claims", "claim_evidence"] {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                [table],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "table {table} should exist");
    }

    for idx in [
        "idx_claims_status",
        "idx_claims_confidence",
        "idx_claim_evidence_claim",
    ] {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name=?1",
                [idx],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "index {idx} should exist");
    }
}

#[test]
fn test_migration_v10_creates_trajectory_and_skill_refinement_tables() {
    let conn = Connection::open_in_memory().unwrap();
    apply_migrations(&conn).unwrap();
    let v = user_version(&conn).unwrap();
    assert!(v >= 10, "expected version >= 10, got {v}");

    for table in [
        "trajectory_events",
        "trajectory_summaries",
        "skill_refinements",
    ] {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                [table],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "table {table} should exist");
    }

    for idx in [
        "idx_traj_events_session_turn",
        "idx_traj_events_created",
        "idx_skill_refinements_name",
    ] {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name=?1",
                [idx],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "index {idx} should exist");
    }
}
