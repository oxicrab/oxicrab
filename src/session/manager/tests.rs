use super::*;
use proptest::prelude::*;

proptest! {
    #[test]
    fn add_message_never_exceeds_max(count in 0..500usize) {
        let mut session = Session::new("prop:test".to_string());
        let extra = HashMap::new();
        for i in 0..count {
            session.add_message("user", format!("msg {}", i), extra.clone());
        }
        prop_assert!(session.messages.len() <= MAX_SESSION_MESSAGES);
    }

    #[test]
    fn get_history_respects_limit(
        msg_count in 0..300usize,
        limit in 0..400usize,
    ) {
        let mut session = Session::new("prop:history".to_string());
        let extra = HashMap::new();
        for i in 0..msg_count {
            session.add_message("user", format!("msg {}", i), extra.clone());
        }
        let history = session.get_history(limit);
        prop_assert!(history.len() <= limit);
        prop_assert!(history.len() <= session.messages.len());
    }
}

#[test]
fn test_session_get_history_with_limit() {
    let mut session = Session::new("test_key".to_string());
    let extra = HashMap::new();

    // Add 5 messages
    for i in 0..5 {
        session.add_message("user".to_string(), format!("Message {}", i), extra.clone());
    }

    let history = session.get_history(3);
    assert_eq!(history.len(), 3);

    // Should return last 3 messages (indices 2, 3, 4)
    assert_eq!(
        history[0]["content"],
        Value::String("Message 2".to_string())
    );
    assert_eq!(
        history[1]["content"],
        Value::String("Message 3".to_string())
    );
    assert_eq!(
        history[2]["content"],
        Value::String("Message 4".to_string())
    );
}

#[test]
fn test_session_get_full_history() {
    let mut session = Session::new("test_key".to_string());
    let extra = HashMap::new();

    // Add 3 messages
    for i in 0..3 {
        session.add_message("user".to_string(), format!("Message {}", i), extra.clone());
    }

    let history = session.get_full_history();
    assert_eq!(history.len(), 3);

    for (i, entry) in history.iter().enumerate() {
        assert_eq!(entry["content"], Value::String(format!("Message {}", i)));
        assert_eq!(entry["role"], Value::String("user".to_string()));
        // Timestamp and extra fields are now included
        assert!(entry.contains_key("timestamp"));
    }
}

#[test]
fn test_session_get_history_includes_extra_fields() {
    let mut session = Session::new("test_key".to_string());
    let mut extra = HashMap::new();
    extra.insert(
        "tool_call_id".to_string(),
        Value::String("tc_123".to_string()),
    );
    session.add_message("tool", "result", extra);

    let history = session.get_history(10);
    assert_eq!(history.len(), 1);
    assert_eq!(history[0]["role"], Value::String("tool".to_string()));
    assert_eq!(
        history[0]["tool_call_id"],
        Value::String("tc_123".to_string())
    );
    assert!(history[0].contains_key("timestamp"));
}

#[test]
fn test_session_add_message_prunes_at_capacity() {
    let mut session = Session::new("test_key".to_string());
    let extra = HashMap::new();

    // Add MAX_SESSION_MESSAGES + 5 messages
    for i in 0..(MAX_SESSION_MESSAGES + 5) {
        session.add_message("user".to_string(), format!("Message {}", i), extra.clone());
    }

    // Should be capped at MAX_SESSION_MESSAGES
    assert_eq!(session.messages.len(), MAX_SESSION_MESSAGES);

    // First message should be the one at index 5 (0-4 should be pruned)
    assert_eq!(session.messages[0].content, "Message 5");

    // Last message should be the last one we added
    assert_eq!(
        session.messages[MAX_SESSION_MESSAGES - 1].content,
        format!("Message {}", MAX_SESSION_MESSAGES + 4)
    );
}

// --- SessionManager persistence tests ---

#[tokio::test]
async fn test_save_and_load_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let mgr = SessionManager::new(dir.path()).unwrap();

    let mut session = Session::new("roundtrip:key");
    session.add_message("user", "hello world", HashMap::new());
    session.add_message("assistant", "hi there", HashMap::new());
    mgr.save(&session).await.unwrap();

    // Create a fresh manager at the same path — forces disk load
    let mgr2 = SessionManager::new(dir.path()).unwrap();
    let loaded = mgr2.get_or_create("roundtrip:key").await.unwrap();

    assert_eq!(loaded.key, "roundtrip:key");
    assert_eq!(loaded.messages.len(), 2);
    assert_eq!(loaded.messages[0].role, "user");
    assert_eq!(loaded.messages[0].content, "hello world");
    assert_eq!(loaded.messages[1].role, "assistant");
    assert_eq!(loaded.messages[1].content, "hi there");
}

#[tokio::test]
async fn test_cache_hit_returns_same_session() {
    let dir = tempfile::tempdir().unwrap();
    let mgr = SessionManager::new(dir.path()).unwrap();

    // First call creates and caches
    let s1 = mgr.get_or_create("cache:test").await.unwrap();
    assert_eq!(s1.key, "cache:test");

    // Second call should return from cache (same content)
    let s2 = mgr.get_or_create("cache:test").await.unwrap();
    assert_eq!(s2.key, s1.key);
    assert_eq!(s2.messages.len(), s1.messages.len());
}

#[tokio::test]
async fn test_lru_eviction() {
    let dir = tempfile::tempdir().unwrap();
    let mgr = SessionManager::new(dir.path()).unwrap();

    // Save a session with known content
    let mut first = Session::new("evict:first");
    first.add_message("user", "important data", HashMap::new());
    mgr.save(&first).await.unwrap();

    // Fill cache to capacity with other sessions to evict the first one
    for i in 0..MAX_CACHED_SESSIONS {
        let _ = mgr
            .get_or_create(&format!("evict:filler_{}", i))
            .await
            .unwrap();
    }

    // "evict:first" should be evicted from LRU cache.
    // get_or_create should still load it from disk.
    let reloaded = mgr.get_or_create("evict:first").await.unwrap();
    assert_eq!(reloaded.key, "evict:first");
    assert_eq!(reloaded.messages.len(), 1);
    assert_eq!(reloaded.messages[0].content, "important data");
}

#[tokio::test]
async fn test_cleanup_old_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let mgr = SessionManager::new(dir.path()).unwrap();

    // Save a session so a file exists
    let session = Session::new("old:session");
    mgr.save(&session).await.unwrap();

    let path = mgr.get_session_path("old:session");
    assert!(path.exists());

    // Backdate the file to 100 days ago
    let old_time =
        filetime::FileTime::from_unix_time(chrono::Utc::now().timestamp() - 100 * 86400, 0);
    filetime::set_file_mtime(&path, old_time).unwrap();

    // Cleanup with 30-day TTL should delete it
    let deleted = mgr.cleanup_old_sessions(30).unwrap();
    assert_eq!(deleted, 1);
    assert!(!path.exists());
}

#[tokio::test]
async fn test_cleanup_preserves_recent_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let mgr = SessionManager::new(dir.path()).unwrap();

    let session = Session::new("recent:session");
    mgr.save(&session).await.unwrap();

    let path = mgr.get_session_path("recent:session");
    assert!(path.exists());

    // Cleanup with 30-day TTL should preserve a just-created file
    let deleted = mgr.cleanup_old_sessions(30).unwrap();
    assert_eq!(deleted, 0);
    assert!(path.exists());
}

#[tokio::test]
async fn test_load_handles_metadata_line() {
    let dir = tempfile::tempdir().unwrap();
    let mgr = SessionManager::new(dir.path()).unwrap();

    // Save a session (which writes a metadata line with key + created_at)
    let mut session = Session::new("meta:test");
    session.metadata.insert(
        "custom".to_string(),
        serde_json::Value::String("value".to_string()),
    );
    mgr.save(&session).await.unwrap();

    // Load from a fresh manager
    let mgr2 = SessionManager::new(dir.path()).unwrap();
    let loaded = mgr2.get_or_create("meta:test").await.unwrap();

    assert_eq!(loaded.key, "meta:test");
    assert_eq!(
        loaded.metadata.get("custom"),
        Some(&serde_json::Value::String("value".to_string()))
    );
}

#[tokio::test]
async fn test_load_handles_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    let mgr = SessionManager::new(dir.path()).unwrap();

    // No file on disk — should create a new empty session
    let session = mgr.get_or_create("does:not:exist").await.unwrap();
    assert_eq!(session.key, "does:not:exist");
    assert!(session.messages.is_empty());
}

#[tokio::test]
async fn test_session_key_roundtrip_with_colons() {
    let dir = tempfile::tempdir().unwrap();
    let mgr = SessionManager::new(dir.path()).unwrap();

    // Keys with colons are common (e.g. "telegram:12345")
    let mut session = Session::new("telegram:12345");
    session.add_message("user", "test", HashMap::new());
    mgr.save(&session).await.unwrap();

    let mgr2 = SessionManager::new(dir.path()).unwrap();
    let loaded = mgr2.get_or_create("telegram:12345").await.unwrap();

    // The metadata line preserves the original key despite filename mangling
    assert_eq!(loaded.key, "telegram:12345");
    assert_eq!(loaded.messages.len(), 1);
}
